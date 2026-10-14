//! `chassis::Gpu` for Vulkan.
//!
//! ## The other half of the seam
//!
//! [`crate::vulkan_resources`] has been a working `chassis::Resources` for a
//! while. This is the `Gpu` that sits above it, and it is deliberately shaped
//! like [`crate::wgpu_gpu::WgpuGpu`] method for method: same constructors, same
//! accessors, same trait impls. Two backends that differ in their accessor names
//! are not interchangeable however well the trait fits.
//!
//! Where they genuinely differ, the difference is in the return type and nothing
//! else:
//!
//! | | wgpu | Vulkan |
//! |---|---|---|
//! | headless | ordinary | ordinary |
//! | swapchain | none yet | `Option<VulkanSwapchain>` |
//! | queue | one, unnumbered | a family index out of several |
//!
//! ## What this backend is for
//!
//! Bare-metal control: per-barrier transitions chosen from the device's
//! `SyncTier`, explicit memory, and the only path that reaches a 1.0 device.
//! It is opt-in (`--backend vulkan`) because wgpu is the default and this is the
//! one more likely to be the thing that is misbehaving.
//!
//! ## The executor is the interesting part
//!
//! Unlike the wgpu one, this executor records barriers: the graph's neutral
//! barriers lowered by [`crate::vulkan_barrier_encoder`], per the device's tier.
//! And unlike wgpu, it needs its resources to be the *same* registry the `Gpu`
//! exposes — a handle made through `dyn Resources` has to resolve in the command
//! list, or every pass that uses one fails with `InvalidHandle`.

use std::sync::Arc;

use chassis::graph::pass::{NoRecording, PassRecorder, SlotTable};
use chassis::{
    BackendChoice, GpuInfo, PassId, RenderGraph as RenderGraphTrait, Resources as ResourcesTrait,
    Result as RhiResult, Swapchain,
};

use crate::vulkan_context::Context;
use crate::vulkan_device::DeviceOptions;
use crate::vulkan_render_graph_executor::{ExecutorResources, VulkanRenderGraphExecutor};
use crate::vulkan_resources::VulkanResources;
use crate::vulkan_swapchain::Swapchain as VulkanSwapchain;

/// Which backend this implementation is.
pub const BACKEND: BackendChoice = BackendChoice::Vulkan;

/// The Vulkan backend, as the RHI sees it.
pub struct VulkanGpu {
    /// **Declaration order is drop order. Do not reorder these.**
    ///
    /// `context` owns the `vk::Device`, and everything below owns objects made
    /// from it. Rust drops fields in declaration order, so `context` must come
    /// *last* -- it is first here only to be corrected, and it was not: with
    /// `context` first the device was destroyed before the recorder freed its
    /// command pools, and every one of the 64 lists leaked.
    ///
    /// It is visible under validation and completely invisible otherwise, which
    /// is the worst combination: a leak that only shows up on a machine that is
    /// set up to catch it. So it is stated here rather than left to be re-broken
    /// by a tidy-up.
    executor: VulkanRenderGraphExecutor,
    resources: VulkanResources,
    swapchain: Option<VulkanSwapchain>,
    graph: GraphAdapter,
    recorder: Option<Arc<dyn PassRecorder>>,
    info: GpuInfo,
    extent: chassis::Extent2d,
    /// Last, because it destroys the device everything above was made from.
    context: Context,
}

impl VulkanGpu {
    /// Headless: a device, no surface, no swapchain.
    ///
    /// Takes an existing [`Context`] rather than creating one, so a caller that
    /// already has a device -- a test, a second view of the same GPU -- does not
    /// create a second one.
    pub fn headless(context: Context, extent: chassis::Extent2d) -> RhiResult<Self> {
        let capabilities = &context.capabilities;
        let family = capabilities.graphics_queue_family.ok_or_else(|| {
            chassis::Error::Unsupported("no graphics queue family on this device".to_owned())
        })?;

        let max_lists = 64;
        let recorder = crate::vulkan_frame_context::Recorder::new(
            &context.device,
            context.graphics_queue,
            family,
            capabilities.sync_tier,
            max_lists,
        )
        .map_err(|error| chassis::Error::Unsupported(error.to_string()))?;

        let memory = context.memory_properties();
        let resources = VulkanResources::new(&context.device, memory);
        let executor = VulkanRenderGraphExecutor::new(recorder, extent);

        let info = GpuInfo {
            name: capabilities.device_name.clone(),
            sync_tier: capabilities.sync_tier,
            max_texture_size: capabilities.max_texture_dimension_2d,
            // One. More needs the recorder to hold two sets of pools and submit to
            // a swapchain image that is not the current one; claiming more here
            // would be a lie that shows up as a validation error about a fence.
            frames_in_flight: 1,
        };

        Ok(Self {
            executor,
            resources,
            swapchain: None,
            graph: GraphAdapter::new(),
            recorder: None,
            info,
            extent,
            context,
        })
    }

    /// Headless, creating the device. The constructor a launcher uses.
    pub fn create_headless(
        engine: &'static std::ffi::CStr,
        extent: chassis::Extent2d,
    ) -> RhiResult<Self> {
        let context = crate::vulkan_device::create_device(DeviceOptions::from_env(engine))
            .map_err(|error| chassis::Error::Unsupported(error.to_string()))?;
        Self::headless(context, extent)
    }

    /// With a swapchain, which is what a window gives.
    pub fn with_swapchain(mut self, swapchain: VulkanSwapchain) -> Self {
        let (width, height) = swapchain.extent();
        self.extent = chassis::Extent2d { width, height };
        self.executor.set_extent(self.extent);
        self.swapchain = Some(swapchain);
        self
    }

    /// Install the code that records each pass.
    pub fn set_pass_recorder(&mut self, recorder: Arc<dyn PassRecorder>) {
        self.recorder = Some(recorder);
    }

    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn resources(&mut self) -> &mut VulkanResources {
        &mut self.resources
    }

    pub fn executor(&self) -> &VulkanRenderGraphExecutor {
        &self.executor
    }

    pub fn executor_mut(&mut self) -> &mut VulkanRenderGraphExecutor {
        &mut self.executor
    }

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

/// The registries a command list borrows, for the duration of one frame.
///
/// Takes `&VulkanResources` rather than `&self`. As a method it would borrow all
/// of `self`, which includes `self.executor`, and the executor needs that
/// mutably one line later -- so the two disjoint fields would be fused into one
/// borrow and refused. Taking the registry alone is what lets the borrow checker
/// see that they really are disjoint.
fn executor_resources(resources: &VulkanResources) -> ExecutorResources<'_> {
    ExecutorResources {
        pipelines: resources.pipelines(),
        textures: resources.textures(),
        buffers: resources.buffers(),
        descriptors: resources.descriptors(),
    }
}

impl Swapchain for VulkanGpu {
    fn extent(&self) -> chassis::Extent2d {
        self.swapchain.as_ref().map_or(self.extent, |swapchain| {
            let (width, height) = swapchain.extent();
            chassis::Extent2d { width, height }
        })
    }

    fn format(&self) -> chassis::TextureFormat {
        self.swapchain
            .as_ref()
            .map_or(chassis::TextureFormat::Rgba8Unorm, |swapchain| {
                crate::vulkan_types::from_vk_format(swapchain.format())
                    .unwrap_or(chassis::TextureFormat::Rgba8Unorm)
            })
    }

    fn current_image_view(&self) -> chassis::ImageView {
        // No view accessor on the backend's swapchain yet — it hands out raw
        // `vk::ImageView` and the RHI hands out generational handles, and the
        // registry that would bridge them does not exist.
        //
        // A null handle, and a null handle here is honest: there is no acquired
        // image because `frame_begin` does not acquire one. A caller that needs
        // one gets a clear failure rather than a view of image zero.
        chassis::ImageView::null()
    }

    fn frame_index(&self) -> u32 {
        0
    }

    fn is_out_of_date(&self) -> bool {
        self.swapchain
            .as_ref()
            .is_some_and(VulkanSwapchain::is_out_of_date)
    }
}

impl RenderGraphTrait for GraphAdapter {
    fn reset(&mut self) {
        self.graph.reset();
    }

    fn add_pass(&mut self, _desc: chassis::RenderPassDesc) -> PassId {
        // The shared graph is fluent: passes are declared through `pass(name)`
        // and a builder, not through a `RenderPassDesc` plus an index. Refusing
        // beats returning an index that names nothing -- the caller would have a
        // handle to a pass it never described.
        unimplemented!(
            "declare passes through chassis::graph::RenderGraph::pass, not through \
             RenderGraph::add_pass: the shared graph takes a builder"
        )
    }

    fn add_dependency(&mut self, _producer: PassId, _consumer: PassId) {
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

    fn execute(&mut self, _jobs: &grind::Jobs) -> RhiResult<()> {
        // Recording is the executor's job and it owns its command lists, which is
        // why `RenderGraph::execute` takes none. This validates the frame.
        self.graph
            .compile()
            .map_err(|errors| chassis::Error::Unsupported(describe(&errors)))?;
        Ok(())
    }
}

/// `chassis::Gpu` for Vulkan.
impl chassis::Gpu for VulkanGpu {
    fn info(&self) -> &GpuInfo {
        &self.info
    }

    fn swapchain(&self) -> Option<&dyn Swapchain> {
        // A `VulkanSwapchain` is not itself a `chassis::Swapchain`, and making it
        // one would mean the two concepts share a name and a meaning. Headless
        // returns `None`, which is the honest answer and the reason the trait's
        // signature is an `Option`.
        None
    }

    fn graph(&mut self) -> &mut dyn RenderGraphTrait {
        &mut self.graph
    }

    fn resources(&mut self) -> &mut dyn ResourcesTrait {
        &mut self.resources
    }

    fn frame_begin(&mut self) -> RhiResult<()> {
        self.graph.reset();
        Ok(())
    }

    fn submit(&mut self) -> RhiResult<()> {
        let compiled = self
            .graph
            .graph
            .compile()
            .map_err(|errors| chassis::Error::Unsupported(describe(&errors)))?;

        // The recorder is cloned out first so the mutable borrow of the executor
        // is not live across the immutable one, which the borrow checker is right
        // to refuse.
        let recorder = self.recorder.clone();
        let resources = executor_resources(&self.resources);
        let result = match recorder {
            Some(recorder) => self
                .executor
                .execute(&compiled, resources, recorder.as_ref()),
            None => self.executor.execute(&compiled, resources, &NoRecording),
        };
        result.map_err(|error| chassis::Error::Unsupported(error.to_string()))
    }

    fn wait_idle(&mut self) {
        unsafe {
            let _ = self.context.device.device_wait_idle();
        }
    }
}

/// Adapts the fluent graph to the `RenderGraph` trait. See the file header for
/// the two methods that refuse.
pub struct GraphAdapter {
    graph: chassis::graph::RenderGraph,
}

impl GraphAdapter {
    pub fn new() -> Self {
        Self {
            graph: chassis::graph::RenderGraph::new(),
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

/// Formats the graph's validation errors into one message.
///
/// The graph returns a `Vec` because it collects every error rather than stopping
/// at the first -- which is right, so they must be joined rather than the first
/// one dropped.
fn describe(errors: &[chassis::graph::types::ValidationError]) -> String {
    errors
        .iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}
