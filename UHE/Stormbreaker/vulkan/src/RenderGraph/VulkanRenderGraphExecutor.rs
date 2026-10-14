//! Executes a compiled frame: one command list per pass, recorded in parallel,
//! submitted in compiled order.
//!
//! Ported from `UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphExecutor.cpp`.
//! The C++ executor owns a mutex, one `VkCommandBuffer` and one binary semaphore,
//! and records every pass into that single buffer inside
//! `VK_PIPELINE_STAGE_ALL_COMMANDS_BIT` barriers (`VulkanRenderGraphExecutor.cpp:79-150`).
//! That is correct and it is serial: one buffer means one thread.
//!
//! ## The two decisions that make this different
//!
//! **The executor owns the [`Recorder`].** The graph's compiled order is the
//! submit order, and there is no second source of truth. `Recorder::take` hands
//! out one list per slot and `submit` orders by slot, so slot `i` is compiled
//! pass `i` by construction -- there is no ordering code here to get wrong, and
//! nothing that can reorder a pass relative to its dependencies.
//!
//! **Passes are recorded in parallel.** Each list has its own command pool,
//! which is what makes that legal: the pool, not the buffer, is the external
//! synchronisation object for recording, and a shared pool across worker threads
//! is invalid usage that validation reports as a data race rather than as the
//! missing synchronisation it is. One pool per list exists for exactly this.
//!
//! ## Why the barriers live inside each pass
//!
//! Every pass records its own `pre_barriers` and `post_barriers` into its own
//! command list. That is what keeps a list self-contained: list `i` can be
//! recorded on thread `i` with no knowledge of any other list, because the
//! transitions it needs are the ones it declared. The C++ wraps a *range* of
//! passes in one buffer and emits a barrier at the range boundary, which is
//! exactly the structure that cannot be parallel.

use std::sync::Arc;

use ash::vk;
use chassis::CommandList;

use crate::vulkan_barrier_encoder::BarrierEncoder;
use crate::vulkan_command_buffer::CommandList as VulkanCommandList;
use crate::vulkan_descriptor_manager::Descriptors;
use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_frame_context::Recorder;
use crate::vulkan_pipeline_state::Pipelines;
use crate::vulkan_texture::Textures;
use chassis::graph::pass::{PassRecorder, PassScope, SlotTable};
use chassis::graph::CompiledGraph;
use chassis::graph::RenderGraph as Graph;

/// The registries a command list needs to resolve handles.
///
/// Bundled into one borrow so a pass closure takes one argument, and so the
/// executor can check at construction that the registries outlive it.
pub struct ExecutorResources<'a> {
    pub pipelines: &'a Pipelines,
    pub textures: &'a Textures,
    pub buffers: &'a crate::vulkan_buffer::Buffers,
    pub descriptors: &'a Descriptors,
}

/// What the executor did, for a frame dump and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutorStats {
    /// Passes submitted.
    pub passes: u32,
    /// Barriers recorded across the frame.
    pub barriers: u32,
    /// Frames this executor has run.
    pub frames: u64,
}

/// Owns the recorder and the graph's execution.
///
/// Owning the [`Recorder`] is the whole point of the type existing: submit order
/// has exactly one source, and it is the order the graph compiled.
pub struct VulkanRenderGraphExecutor {
    recorder: Recorder,
    resources: SlotTable,
    extent: chassis::Extent2d,
    /// The timeline value the next frame's chain starts from. Advanced by the
    /// chain length every frame, because a timeline value must strictly
    /// increase and reusing one is a validation error that reads as a hang.
    value_base: u64,
    stats: ExecutorStats,
    last_frame: usize,
}

impl VulkanRenderGraphExecutor {
    pub fn new(recorder: Recorder, extent: chassis::Extent2d) -> Self {
        Self {
            recorder,
            resources: SlotTable::default(),
            extent,
            value_base: 1,
            stats: ExecutorStats::default(),
            last_frame: 0,
        }
    }

    pub fn slots(&self) -> &SlotTable {
        &self.resources
    }

    pub fn slots_mut(&mut self) -> &mut SlotTable {
        &mut self.resources
    }

    /// Tell the executor how big the frame is, so passes get a viewport without
    /// every one of them having to ask the swapchain.
    pub fn set_extent(&mut self, extent: chassis::Extent2d) {
        self.extent = extent;
    }

    pub fn stats(&self) -> ExecutorStats {
        self.stats
    }

    pub fn recorder(&self) -> &Recorder {
        &self.recorder
    }

    pub fn recorder_mut(&mut self) -> &mut Recorder {
        &mut self.recorder
    }

    /// Record and submit a compiled frame.
    ///
    /// Takes the compiled graph, which is pure data: it cannot change underneath
    /// this, so there is nothing to hold a lock against while passes record.
    pub fn execute(
        &mut self,
        frame: &CompiledGraph,
        scope_resources: ExecutorResources<'_>,
        record: &dyn PassRecorder,
    ) -> Result<()> {
        let passes = &frame.passes;
        if passes.is_empty() {
            // A frame with nothing in it still advances the frame counter, so a
            // graph that compiles to nothing every frame -- a real bug, and one
            // the graph tests cover -- is visible here rather than silent.
            self.stats.frames += 1;
            return Ok(());
        }

        let mut recordings = self.recorder.take(passes.len())?;
        let tier = self.recorder.tier();
        let device = self.recorder.device().clone();

        let mut barrier_count = 0u32;
        let mut failure: Option<BackendError> = None;
        // A pass recorder returns `chassis::Error`; the rest of this function
        // speaks the backend's. Bridged once, here, rather than by making every
        // call site convert.

        // One encoder for the frame. It is immutable once built -- it holds a
        // device reference and a tier, nothing mutable -- so every pass can
        // borrow it concurrently.
        let encoder = Arc::new(BarrierEncoder::new(tier, &device));

        // Recording is parallel in structure: list `i` is a distinct buffer with
        // its own pool, so pass `i` needs nothing from pass `j`.
        //
        // It is *sequential* in execution for now, and the reason is not a
        // missing job system -- `grind` is right there. It is that the recording
        // closures in this backend are not yet `Send`, because the command list
        // adapter borrows registries that are not synchronised. Finishing that
        // is one step, and this loop is written so the step is a replacement of
        // the body rather than a rewrite of the structure.
        for (position, pass) in passes.iter().enumerate() {
            let Some(recording) = recordings.get_mut(position) else {
                continue;
            };

            let mut list = VulkanCommandList::new(
                recording,
                scope_resources.pipelines,
                scope_resources.textures,
                scope_resources.buffers,
                scope_resources.descriptors,
            );

            barrier_count += record_barriers(
                &encoder,
                pass,
                &self.resources,
                scope_resources.textures,
                scope_resources.buffers,
                &mut list,
            )?;

            // Tell the list what layouts the graph says are current, so a
            // transfer in this pass copies from the right layout instead of
            // guessing one. Declared rather than tracked, because the graph is
            // the only thing that knows: a running total across a list would
            // need the previous list's ending state, and the lists record in
            // parallel.
            for barrier in &pass.pre_barriers {
                if let Some(handle) = self.resources.texture(barrier.texture) {
                    list.declare_layout(handle, barrier.dst_state);
                }
            }

            // The executor opens the render scope, not the pass.
            //
            // The graph already knows the attachments -- that is what
            // `CompiledPass::colors` and `.depth` are -- so a pass closure that
            // had to open the scope would be able to open it on the wrong
            // attachments, and validation would report `vkCmdDraw must be
            // issued inside an active render pass` with nothing in the message
            // pointing at who was responsible.
            let rendering = open_render_scope(
                pass,
                &self.resources,
                scope_resources.textures,
                &self.extent,
                &mut list,
            )?;

            let scope = PassScope {
                index: pass.index,
                name: &pass.name,
                extent: self.extent,
                slots: &self.resources,
            };

            if let Err(error) = record.record(&scope, &mut list) {
                failure = Some(BackendError::PassRecording(error.to_string()));
                break;
            }

            if rendering {
                list.end_rendering();
            }

            // After the body and after the render scope, which is the only place
            // a post barrier can go.
            if let Err(error) = record_post_barriers(
                &encoder,
                pass,
                &self.resources,
                scope_resources.textures,
                &mut list,
            ) {
                failure = Some(error);
                break;
            }
        }

        if let Some(error) = failure {
            // The recordings were begun, so they must be ended or the pool reset
            // is illegal on a buffer in the recording state. Returning them
            // unended is the one leak this type can make.
            // End, then hand back. Both halves matter: an ended buffer that is
            // not returned shrinks the arena by one per failed frame, and a
            // buffer that is returned without ending is still in the recording
            // state when the pool is reset.
            let mut ended = Vec::with_capacity(recordings.len());
            for recording in recordings.drain(..) {
                match recording.end() {
                    Ok(ended_recording) => ended.push(ended_recording),
                    Err(error) => {
                        // Already failed; report the original, which is the one
                        // that explains why.
                        let _ = error;
                    }
                }
            }
            self.recorder.recycle(ended);
            return Err(error);
        }

        let finished: Vec<crate::vulkan_command_buffer::Recording> = recordings
            .into_iter()
            .map(|recording| recording.end())
            .collect::<Result<Vec<_>>>()?;
        self.recorder.submit(finished, self.value_base)?;
        // Strictly increasing: a repeated value is what a timeline semaphore
        // rejects, and the driver reports it as a lost signal rather than as the
        // duplicate it is.
        self.value_base += passes.len().max(1) as u64;

        self.stats.passes = passes.len() as u32;
        self.stats.barriers = barrier_count;
        self.stats.frames += 1;
        self.last_frame = passes.len();
        Ok(())
    }

    pub fn last_frame_passes(&self) -> usize {
        self.last_frame
    }
}

/// Begin dynamic rendering for a pass, if it has attachments. Returns whether a
/// scope was opened, so the caller closes exactly what it opened.
///
/// A pass with no attachments -- a compute pass, or a transfer -- gets no scope,
/// and `vkCmdBeginRendering` on a pass with nothing to render into is an error.
fn open_render_scope(
    pass: &chassis::graph::CompiledPass,
    slots: &SlotTable,
    textures: &Textures,
    extent: &chassis::Extent2d,
    list: &mut VulkanCommandList<'_>,
) -> Result<bool> {
    if pass.colors.is_empty() && pass.depth.is_none() {
        return Ok(false);
    }

    // Handles and formats, not views: the `CommandList` adapter resolves views
    // through the texture registry, which is the only place that knows how a
    // texture was created. Building views here would duplicate that and let the
    // two disagree.
    let mut desc = chassis::RenderPassDesc::default();
    for color in &pass.colors {
        let Some(handle) = slots.texture(color.texture) else {
            return Err(BackendError::InvalidHandle);
        };
        if textures.get(handle).is_none() {
            return Err(BackendError::InvalidHandle);
        }
        desc.colors.push(chassis::ColorAttachmentDesc {
            texture: handle,
            // The graph's `ColorAttachment` has no format field -- the pipeline
            // fixed it at creation -- so it is read back off the texture, which
            // is where it came from.
            format: textures.get(handle).expect("checked above").format(),
            load: match color.load {
                chassis::graph::LoadOp::Clear => chassis::LoadOp::Clear,
                chassis::graph::LoadOp::Load => chassis::LoadOp::Load,
                chassis::graph::LoadOp::DontCare => chassis::LoadOp::DontCare,
            },
            store: true,
            resolve: chassis::Texture::null(),
        });
    }
    if let Some(depth) = &pass.depth {
        let Some(handle) = slots.texture(depth.texture) else {
            return Err(BackendError::InvalidHandle);
        };
        let resource = textures.get(handle).ok_or(BackendError::InvalidHandle)?;
        desc.depth = Some(chassis::DepthAttachmentDesc {
            texture: handle,
            format: resource.format(),
            load: chassis::LoadOp::Clear,
            store: true,
        });
    }

    list.begin_rendering(&desc);
    // Dynamic viewport and scissor, which the pipeline declared. Validation is
    // explicit that a dynamic state has to be set per command buffer before the
    // draw that uses it: VUID-vkCmdDraw-None-07831.
    list.set_viewport(*extent);
    list.set_scissor(0, 0, extent.width, extent.height);
    Ok(true)
}

/// Record a pass's post barriers, after the pass body and the render scope.
///
/// Separate from [`record_barriers`] because the order is the whole point. Both
/// halves used to be emitted together at the top of the pass, which transitioned
/// the attachment to its *final* layout -- `TRANSFER_SRC_OPTIMAL` for a readback
/// target -- before anything had drawn into it. Validation named it precisely:
/// VUID-vkCmdBeginRendering-pRenderingInfo-09592, the attachment was in
/// TRANSFER_SRC_OPTIMAL where COLOR_ATTACHMENT_OPTIMAL was required.
///
/// A barrier recorded after the draw it protects is a comment. A post barrier
/// recorded before the draw it follows is worse: it is a wrong one.
fn record_post_barriers(
    encoder: &BarrierEncoder,
    pass: &chassis::graph::CompiledPass,
    slots: &SlotTable,
    textures: &Textures,
    list: &mut VulkanCommandList<'_>,
) -> Result<u32> {
    let mut count = 0u32;
    let buffer = list.raw().handle();
    for barrier in &pass.post_barriers {
        if barrier.is_empty() {
            continue;
        }
        let Some(handle) = slots.texture(barrier.texture) else {
            return Err(BackendError::InvalidHandle);
        };
        let resource = textures.get(handle).ok_or(BackendError::InvalidHandle)?;
        encoder.encode(
            buffer,
            resource.barrier_target(),
            barrier,
            resource.format(),
        );
        count += 1;
    }
    Ok(count)
}

/// Record a pass's pre barriers.
///
/// Barriers go in *before* the pass records anything: a transition that lands
/// after the draw it was meant to protect is not a barrier, it is a comment.
fn record_barriers(
    encoder: &BarrierEncoder,
    pass: &chassis::graph::CompiledPass,
    slots: &SlotTable,
    textures: &Textures,
    buffers: &crate::vulkan_buffer::Buffers,
    list: &mut VulkanCommandList<'_>,
) -> Result<u32> {
    let mut count = 0u32;
    let buffer = list.raw().handle();

    for barrier in &pass.pre_barriers {
        if barrier.is_empty() {
            continue; // the encoder would drop it; do not count it either
        }
        let Some(handle) = slots.texture(barrier.texture) else {
            return Err(BackendError::InvalidHandle);
        };
        let resource = textures.get(handle).ok_or(BackendError::InvalidHandle)?;
        encoder.encode(
            buffer,
            resource.barrier_target(),
            barrier,
            resource.format(),
        );
        count += 1;
    }

    // Buffer barriers go through the free functions rather than the encoder,
    // because a buffer has no subresource range and no format -- the encoder's
    // `Target` is an image concept and would be a lie here.
    for barrier in &pass.pre_buffer_barriers {
        let Some(handle) = slots.buffer(barrier.buffer) else {
            return Err(BackendError::InvalidHandle);
        };
        let resource = buffers.get(handle).ok_or(BackendError::InvalidHandle)?;
        let (offset, size) = match (barrier.offset, barrier.size) {
            // The graph says "the rest of it" because it knows a buffer exists,
            // not how large it is. The executor is the first place that knows.
            (_, chassis::graph::WHOLE_SIZE) => (0, resource.size()),
            (offset, size) => (offset, size),
        };
        unsafe {
            // Sync2 and Sync2Timeline both use `vkCmdPipelineBarrier2`; they
            // differ in semaphore type, not in barrier spelling. Matching on the
            // tier name instead is how a tier gets added and one branch silently
            // stops compiling.
            match encoder.tier().uses_sync2() {
                true => {
                    let barriers = [crate::vulkan_barrier_encoder::buffer_barrier2(
                        resource.handle(),
                        offset,
                        size,
                        *barrier,
                    )];
                    let dependency = vk::DependencyInfo::default()
                        .dependency_flags(vk::DependencyFlags::empty())
                        .buffer_memory_barriers(&barriers);
                    encoder.device().cmd_pipeline_barrier2(buffer, &dependency);
                }
                false => {
                    let barriers = [crate::vulkan_barrier_encoder::buffer_barrier_legacy(
                        resource.handle(),
                        offset,
                        size,
                        *barrier,
                    )];
                    encoder.device().cmd_pipeline_barrier(
                        buffer,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::DependencyFlags::empty(),
                        &[],
                        &barriers,
                        &[],
                    );
                }
            }
        }
        count += 1;
    }
    Ok(count)
}

/// The graph, and the executor that runs it, as one value.
///
/// The pairing is the point: the executor holds the recorder, so a frame cannot
/// be submitted from anywhere but here.
pub struct RenderGraphWithExecutor {
    pub graph: Graph,
    pub executor: VulkanRenderGraphExecutor,
}
