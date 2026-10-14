//! The wgpu render graph executor.
//!
//! ## Why this lives in `RenderGraph/`
//!
//! Because the Vulkan backend puts its executor there, and a reader who knows
//! where to look in one backend should not have to go looking in the other. The
//! C++ engine has `Vulkan/RenderGraph/VulkanRenderGraphExecutor.{h,cpp}`; this is
//! the same file for the same reason.
//!
//! ## What is the same and what is not
//!
//! The *graph* is shared: `chassis::graph` compiles a frame, and both backends
//! consume the same `CompiledGraph`. Ordering, culling and versioning are
//! identical, and they have to be -- two backends that ordered a frame
//! differently would not be interchangeable, they would be two engines.
//!
//! The *execution* is not shared, and the difference is not a workaround:
//!
//!  * Vulkan records barriers. `chassis::graph` emits them from declared reads
//!    and writes; `vulkan_render_graph_barriers` lowers them.
//!  * wgpu emits nothing. It inserts transitions itself, from the access a
//!    render pass declares. A barrier recorded here would be a lie about a
//!    pipeline that does not take them.
//!
//! So the two executors share a shape and not a body. Each has its own
//! `ExecutorResources`, its own command buffer type, and its own idea of what a
//! frame costs.

//! Executes a compiled frame on wgpu.
//!
//! ## The one thing this does not do
//!
//! **It records no barriers.** The graph emits backend-neutral barriers and the
//! Vulkan executor lowers them through `VulkanBarrierEncoder`, choosing
//! `vkCmdPipelineBarrier2` or `vkCmdPipelineBarrier` per the device's `SyncTier`.
//! wgpu has no per-barrier control at all -- it inserts its own transitions -- so
//! this executor ignores the graph's barrier output entirely.
//!
//! That is not a gap, and it is worth being precise about why it is safe:
//! barriers are *one backend's expression* of the graph's ordering. Ordering is
//! what makes a frame correct and what lets passes record in parallel;
//! expressions are per-backend. The graph would have been unusable from a second
//! backend if its compiler emitted `VkImageMemoryBarrier`, which is exactly why
//! it emits neutral ones.
//!
//! ## What is shared with the Vulkan executor, verbatim
//!
//! `SlotTable`, `PassScope`, `PassRecorder` and `NoRecording` all live in
//! `chassis::graph::pass`. A renderer implements `PassRecorder` once and both
//! backends call it with the same scope. That is what "the front end is the same
//! and only the renderer has options" actually means.
//!
//! ## What wgpu makes simpler
//!
//! No command pools. The Vulkan executor takes one list per pass from a `Recorder`
//! that owns a pool per slot, and that pool is the external-synchronisation
//! object for recording -- sharing one across worker threads is invalid usage. wgpu
//! has no equivalent: a `Recording` owns its own encoder and there is nothing to
//! share. No timeline semaphores either; `queue.submit` is the whole synchronisation
//! story and wgpu orders submissions to one queue itself.

use chassis::graph::pass::{PassRecorder, PassScope, SlotTable};
use chassis::graph::CompiledGraph;

use crate::wgpu_command_buffer::{Recording, WgpuCommandList};
use crate::wgpu_device::Queue;
use crate::wgpu_resources::WgpuResources;

/// What the executor did, for a frame dump and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutorStats {
    /// Passes submitted.
    pub passes: u32,
    /// Command buffers finished.
    pub buffers: u32,
    /// Frames run.
    pub frames: u64,
}

/// Runs compiled frames.
///
/// Owns nothing that has to be torn down -- wgpu owns its objects -- so this is a
/// thin type by design. It exists to make the ordering guarantee legible in one
/// place rather than spread across a call site.
pub struct WgpuRenderGraphExecutor {
    queue: Queue,
    resources: WgpuResources,
    slots: SlotTable,
    extent: chassis::Extent2d,
    stats: ExecutorStats,
}

impl WgpuRenderGraphExecutor {
    pub fn new(queue: Queue, resources: WgpuResources, extent: chassis::Extent2d) -> Self {
        Self {
            queue,
            resources,
            slots: SlotTable::default(),
            extent,
            stats: ExecutorStats::default(),
        }
    }

    pub fn stats(&self) -> ExecutorStats {
        self.stats
    }

    pub fn slots(&self) -> &SlotTable {
        &self.slots
    }

    pub fn slots_mut(&mut self) -> &mut SlotTable {
        &mut self.slots
    }

    pub fn resources(&self) -> &WgpuResources {
        &self.resources
    }

    /// Mutable, because the executor has to own the only registry.
    ///
    /// It has to, and that was a bug: `WgpuGpu` once built a `WgpuResources` for
    /// itself *and* handed a second one to the executor, so a texture created
    /// through `dyn Resources` had a handle the executor could not resolve. Every
    /// such use failed with `InvalidHandle` at record time -- far from the cause.
    ///
    /// One registry, owned here, reachable from both directions.
    pub fn resources_mut(&mut self) -> &mut WgpuResources {
        &mut self.resources
    }

    /// The frame's size, so a pass gets a viewport without asking the swapchain.
    pub fn set_extent(&mut self, extent: chassis::Extent2d) {
        self.extent = extent;
    }

    /// Record every pass and submit, in the graph's compiled order.
    ///
    /// `recordings` must be in compiled order and the submit keeps it, because
    /// wgpu executes submissions to one queue in the order they were made. That is
    /// what replaces the Vulkan executor's whole chain of timeline semaphore
    /// waits: same guarantee, one fewer piece of machinery to get wrong.
    pub fn execute(
        &mut self,
        graph: &CompiledGraph,
        record: &dyn PassRecorder,
    ) -> Result<(), chassis::Error> {
        let mut buffers = Vec::with_capacity(graph.passes.len());

        for pass in &graph.passes {
            let mut recording = Recording::new(self.resources.device().inner(), &pass.name);
            {
                let mut list = WgpuCommandList::new(&mut recording, &self.resources);
                let scope = PassScope {
                    index: pass.index,
                    name: &pass.name,
                    extent: self.extent,
                    slots: &self.slots,
                };
                record.record(&scope, &mut list)?;
            }
            if recording.is_inside_render_pass() {
                // A pass that began a render scope and never ended it. Submitted
                // as-is, wgpu ends the scope at `finish` and the tail of the pass
                // is quietly lost -- so it is refused here, where the message can
                // name the pass.
                return Err(chassis::Error::Unsupported(format!(
                    "pass '{}' ended while still inside a render pass",
                    pass.name
                )));
            }
            buffers.push(recording.finish());
        }

        let submitted = buffers.len() as u32;
        // Moved in, not copied: a `CommandBuffer` is not `Copy`, and the
        // executor has no use for them afterwards.
        self.queue.inner().submit(buffers);

        self.stats.passes = graph.passes.len() as u32;
        self.stats.buffers = submitted;
        self.stats.frames += 1;
        Ok(())
    }

    /// Block until the queue has drained.
    ///
    /// Test and shutdown only -- the frame loop never calls this, for the same
    /// reason the Vulkan backend's `Gpu::wait_idle` is documented as test-only.
    pub fn wait_idle(&self) -> Result<(), chassis::Error> {
        self.resources
            .device()
            .inner()
            .poll(wgpu::PollType::wait_indefinitely())
            .map(|_| ())
            .map_err(|error| chassis::Error::Unsupported(format!("poll failed: {error:?}")))
    }
}
