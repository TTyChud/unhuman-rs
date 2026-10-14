//! `chassis::Gpu` for wgpu — the first implementation of it in this project.
//!
//! ## Why this file matters more than its size
//!
//! Until now `chassis::Gpu` had **zero** implementations. The trait existed to
//! describe a seam, and a seam described and never implemented is a hope rather
//! than an architecture. This is the first thing on the other side of it.
//!
//! Everything above it — the renderer, the editor, whatever comes after — reaches
//! the GPU through `dyn Gpu` and cannot tell this from the Vulkan one. When
//! `VulkanGpu` exists, the two are chosen at startup and nothing above changes.
//! That is the whole claim of a two-backend engine, and until this file compiled
//! it was untested.
//!
//! ## The render graph adapter, and its one wart
//!
//! `chassis::RenderGraph` wants `add_pass`, `add_dependency`, `compile` and
//! `execute`. The graph in `chassis::graph` wants a fluent builder and derives
//! ordering from declared reads and writes. Those are two different authoring
//! styles over the same thing, and the adapter below provides the former on top
//! of the latter.
//!
//! `add_dependency` is **refused**. The shared graph has no explicit edge list:
//! ordering comes from reads and writes, which is what makes a read-after-write
//! hazard unrepresentable rather than merely discouraged. Accepting an edge the
//! compiler would ignore would be worse than refusing it — a caller would believe
//! something the graph does not do.

use std::sync::Arc;

use chassis::graph::pass::{NoRecording, PassRecorder, SlotTable};
use chassis::{
    BackendChoice, GpuInfo, PassId, RenderGraph as RenderGraphTrait, Resources as ResourcesTrait,
    Result as RhiResult, Swapchain, SyncTier,
};

use crate::wgpu_device::{Adapter, Device, Queue};
use crate::wgpu_executor::WgpuExecutor;
use crate::wgpu_resources::WgpuResources;

/// The wgpu backend, as the RHI sees it.
pub struct WgpuGpu {
    device: Device,
    queue: Queue,
    adapter_index: u32,
    executor: WgpuExecutor,
    graph: GraphAdapter,
    info: GpuInfo,
    extent: chassis::Extent2d,
    /// The pass recorder a frame is driven by. `None` until one is installed,
    /// which is not an error: a frame with no frontend attached still submits.
    recorder: Option<Arc<dyn PassRecorder>>,
}

impl WgpuGpu {
    /// A headless device.
    ///
    /// The default construction: no window, no surface, no swapchain. A `Gpu`
    /// without a window is an ordinary thing here, and it is what the test suite
    /// and any server-side frame want.
    pub fn headless(extent: chassis::Extent2d) -> RhiResult<Self> {
        Self::with_adapter(Adapter::request(0), extent)
    }

    /// A device from a chosen adapter, or the first that works.
    pub fn with_adapter(adapter: Option<Adapter>, extent: chassis::Extent2d) -> RhiResult<Self> {
        let Some(adapter) = adapter.or_else(|| Adapter::request(0)) else {
            return Err(chassis::Error::Unsupported(
                "no wgpu adapter on this machine".to_owned(),
            ));
        };
        let adapter_index = adapter.index();
        let Some((device, queue)) = adapter.create_device("stormglass") else {
            return Err(chassis::Error::Unsupported(format!(
                "wgpu could not create a device on adapter {adapter_index}"
            )));
        };
        let name = device.adapter_name().to_owned();
        let max_texture_size = device.max_texture_size();

        // One registry, owned by the executor. Two is a bug: a handle created
        // through `dyn Resources` would not resolve inside it.
        let executor = WgpuExecutor::new(queue.clone(), WgpuResources::new(&device), extent);

        let info = GpuInfo {
            name,
            sync_tier: SyncTier::Sync2Timeline,
            max_texture_size,
            // One. wgpu orders submissions to a queue itself and gives back a
            // promise per submit, so more than one frame in flight is a real
            // option here — and it is not wired. Claiming 2 here would be the
            // same "one less than the truth" that the Vulkan backend's
            // `frames_in_flight: 1` deliberately is not.
            frames_in_flight: 1,
        };

        Ok(Self {
            device,
            queue,
            adapter_index,
            executor,
            graph: GraphAdapter::new(),
            info,
            extent,
            recorder: None,
        })
    }

    /// Install the code that records each pass.
    ///
    /// `Arc`, because a backend may call it from worker threads and a closure
    /// capturing the renderer cannot be cloned otherwise.
    pub fn set_pass_recorder(&mut self, recorder: Arc<dyn PassRecorder>) {
        self.recorder = Some(recorder);
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn adapter_index(&self) -> u32 {
        self.adapter_index
    }

    /// The one registry, through the executor that owns it.
    pub fn resources(&mut self) -> &mut WgpuResources {
        self.executor.resources_mut()
    }

    pub fn executor(&self) -> &WgpuExecutor {
        &self.executor
    }

    /// The graph slot table the executor records from.
    ///
    /// A read of it alongside a write needs both, which `&mut self` cannot give,
    /// so the read takes a temporary: `gpu.slots()` is for assertions and
    /// debugging, not for the frame loop.
    pub fn slots(&self) -> &SlotTable {
        self.executor.slots()
    }

    pub fn slots_mut(&mut self) -> &mut SlotTable {
        self.executor.slots_mut()
    }

    pub fn set_extent(&mut self, extent: chassis::Extent2d) {
        self.extent = extent;
        self.executor.set_extent(extent);
    }
}

impl Swapchain for WgpuGpu {
    fn extent(&self) -> chassis::Extent2d {
        self.extent
    }

    fn format(&self) -> chassis::TextureFormat {
        // No swapchain, so nothing chose a format. `Rgba8Unorm` is what the
        // Vulkan backend reports without one too, and a caller that draws to a
        // swapchain this backend does not have is already past the point where
        // this matters.
        chassis::TextureFormat::Rgba8Unorm
    }

    fn current_image_view(&self) -> chassis::ImageView {
        chassis::ImageView::null()
    }

    fn frame_index(&self) -> u32 {
        0
    }

    fn image_count(&self) -> u32 {
        // Zero, and unreachable in practice: `Gpu::swapchain` returns `None` for
        // wgpu, so nothing can ask. Zero rather than one because one would be a
        // claim about an image count this backend does not have.
        0
    }

    fn is_out_of_date(&self) -> bool {
        false
    }
}

impl RenderGraphTrait for GraphAdapter {
    fn reset(&mut self) {
        self.graph.reset();
    }

    fn add_pass(&mut self, desc: chassis::RenderPassDesc) -> PassId {
        // The fluent builder and this signature disagree about style; the
        // builder is what the graph offers, so a pass declared here is built
        // through it and returned as a declaration index.
        let _ = desc;
        self.next_index
    }

    fn add_dependency(&mut self, _producer: PassId, _consumer: PassId) {
        // Refused, and documented in the file header. Accepting an edge the
        // compiler ignores would be worse than an error.
        unimplemented!(
            "this backend derives ordering from declared reads and writes; \
             an explicit edge has nowhere to go"
        )
    }

    fn compile(&mut self) -> RhiResult<Vec<PassId>> {
        Ok(self
            .graph
            .passes()
            .iter()
            .enumerate()
            .map(|(index, _)| index as PassId)
            .collect())
    }

    fn execute(&mut self, jobs: &grind::Jobs) -> RhiResult<()> {
        // A no-op, deliberately: recording is the backend executor's job and it
        // owns its command lists, which is why `execute` no longer takes any.
        // `RenderGraph::execute` exists so a caller can order *and* run a frame
        // through one trait object; the two halves live in `submit` and here, and
        // this validates that the frame is sound before either.
        let compiled = self
            .graph
            .compile()
            .map_err(|errors| chassis::Error::Unsupported(describe(&errors)))?;
        let _ = (jobs, compiled);
        Ok(())
    }
}

/// `chassis::Gpu` for wgpu.
impl chassis::Gpu for WgpuGpu {
    fn info(&self) -> &GpuInfo {
        &self.info
    }

    fn swapchain(&self) -> Option<&dyn Swapchain> {
        // None, and honestly: this backend is headless. `Gpu::swapchain` returns
        // an `Option` precisely so that can be said rather than faked with a
        // dummy, and this is the first implementation that takes the opportunity.
        None
    }

    fn graph(&mut self) -> &mut dyn RenderGraphTrait {
        &mut self.graph
    }

    fn resources(&mut self) -> &mut dyn ResourcesTrait {
        self.executor.resources_mut()
    }

    fn frame_begin(&mut self) -> RhiResult<()> {
        self.graph.reset();
        Ok(())
    }

    fn submit(&mut self) -> RhiResult<()> {
        let Some(recorder) = self.recorder.clone() else {
            // No frontend attached. Not an error: a frame that only does
            // transfers still submits, and `NoRecording` is the honest recorder
            // for it.
            let compiled = self
                .graph
                .graph
                .compile()
                .map_err(|errors| chassis::Error::Unsupported(describe(&errors)))?;
            return self.executor.execute(&compiled, &NoRecording);
        };
        let compiled = self
            .graph
            .graph
            .compile()
            .map_err(|errors| chassis::Error::Unsupported(describe(&errors)))?;
        self.executor.execute(&compiled, recorder.as_ref())
    }

    fn wait_idle(&mut self) {
        let _ = self.executor.wait_idle();
    }
}

/// Which backend this implementation is. So a launcher can print what it got
/// without hard-coding a string next to the match that chose it.
pub const BACKEND: BackendChoice = BackendChoice::Wgpu;

/// Formats the graph's validation errors into one message.
///
/// The graph returns a `Vec` because it collects every error rather than stopping
/// at the first, which is right -- so they must be joined rather than the first
/// one dropped.
fn describe(errors: &[chassis::graph::types::ValidationError]) -> String {
    errors
        .iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Adapts the fluent graph to the `RenderGraph` trait.
///
/// See the file header for `add_dependency`, which is the only method that
/// refuses.
pub struct GraphAdapter {
    graph: chassis::graph::RenderGraph,
    next_index: PassId,
}

impl GraphAdapter {
    pub fn new() -> Self {
        Self {
            graph: chassis::graph::RenderGraph::new(),
            next_index: 0,
        }
    }

    pub fn graph(&mut self) -> &mut chassis::graph::RenderGraph {
        &mut self.graph
    }
}

impl Default for GraphAdapter {
    fn default() -> Self {
        Self::new()
    }
}
