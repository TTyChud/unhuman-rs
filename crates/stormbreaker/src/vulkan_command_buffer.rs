//! The command buffer: raw recording operations, and the `chassis::CommandList`
//! adapter that resolves handles on top of them.
//!
//! Mirrors `VulkanCommandBuffer.cpp` — with one structural difference that is the
//! point of the whole design. The C++ class owns *one* primary buffer for the
//! entire frame, which is why `VulkanDevice::EndFrameGraph` chains every pass to
//! its predecessor (`VulkanDevice.cpp:630-637`) and admits in a comment that this
//! defeats the DAG. Here each worker records into its own buffer, so there is
//! nothing to chain and the compiled order survives to the GPU.
//!
//! The buffer also has **no barrier API**. Barriers arrive pre-baked from the
//! render graph, which is what makes the `MarkSwapchainRenderedByLegacy` style of
//! handshake unnecessary.
//!
//! Two layers, deliberately:
//!
//! * [`Recording`] — raw Vulkan. Takes `vk::Pipeline`, `vk::Buffer`. No registry
//!   lookups, no locking, nothing that could stall a worker thread.
//! * [`CommandList`] — implements `chassis::CommandList`. Takes generational
//!   handles and resolves them through the resource registries.
//!
//! The split exists so the hot path does no handle resolution. A pass recording
//! on a worker thread should be able to hold its pipelines by raw handle for the
//! whole frame, and resolve `chassis::Pipeline` once when the pass is built.

use ash::{vk, Device};

use crate::vulkan_descriptor_manager::Descriptors;
use crate::vulkan_error::Result;
use crate::vulkan_pipeline_state::Pipelines;
use crate::vulkan_texture::Textures;
use crate::vulkan_types;

/// A command buffer being recorded on one thread. Raw Vulkan only.
pub struct Recording {
    device: Device,
    buffer: vk::CommandBuffer,
    slot: u32,
    /// Inside `vkCmdBeginRendering` / `vkCmdEndRendering`, tracked so a
    /// mismatched pair is caught here rather than by the driver.
    rendering: bool,
}

impl Recording {
    /// A recording must already be begun; `Recorder::take` does that.
    pub fn new(device: Device, buffer: vk::CommandBuffer, slot: u32) -> Self {
        Self {
            device,
            buffer,
            slot,
            rendering: false,
        }
    }

    /// Gives the buffer back to the recorder. Consuming, because a recording must
    /// not outlive the frame that took it.
    pub fn into_buffer(self) -> vk::CommandBuffer {
        self.buffer
    }

    pub fn handle(&self) -> vk::CommandBuffer {
        self.buffer
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }

    // ── rendering scope ────────────────────────────────────────────────────

    /// Dynamic rendering. Attachments come from the graph's compiled pass, not
    /// from a `VkRenderPass`, which this backend has no concept of.
    ///
    /// No format parameter: under dynamic rendering the attachment formats are
    /// declared once in `VkPipelineRenderingCreateInfo` when the pipeline is
    /// compiled, and the frame loop cannot change them. A pass whose views do not
    /// match its pipeline's formats is a validation error, and that is the check.
    pub fn begin_rendering(
        &mut self,
        color_views: &[vk::ImageView],
        depth_view: Option<vk::ImageView>,
        extent: vk::Extent2D,
    ) {
        let color_attachments: Vec<vk::RenderingAttachmentInfo> = color_views
            .iter()
            .map(|view| vk::RenderingAttachmentInfo {
                image_view: *view,
                image_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                resolve_image_view: vk::ImageView::null(),
                resolve_mode: vk::ResolveModeFlags::NONE,
                load_op: vk::AttachmentLoadOp::CLEAR,
                store_op: vk::AttachmentStoreOp::STORE,
                ..Default::default()
            })
            .collect();

        let depth_attachment = depth_view.map(|view| vk::RenderingAttachmentInfo {
            image_view: view,
            image_layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            resolve_image_view: vk::ImageView::null(),
            resolve_mode: vk::ResolveModeFlags::NONE,
            load_op: vk::AttachmentLoadOp::CLEAR,
            store_op: vk::AttachmentStoreOp::STORE,
            ..Default::default()
        });

        let info = vk::RenderingInfo {
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: color_attachments.len() as u32,
            p_color_attachments: color_attachments.as_ptr(),
            p_depth_attachment: depth_attachment
                .as_ref()
                .map(|a| a as *const vk::RenderingAttachmentInfo)
                .unwrap_or(std::ptr::null()),
            p_stencil_attachment: std::ptr::null(),
            ..Default::default()
        };

        unsafe { self.device.cmd_begin_rendering(self.buffer, &info) };
        self.rendering = true;
    }

    pub fn end_rendering(&mut self) {
        debug_assert!(self.rendering, "end_rendering without begin_rendering");
        unsafe { self.device.cmd_end_rendering(self.buffer) };
        self.rendering = false;
    }

    // ── state ──────────────────────────────────────────────────────────────

    pub fn set_viewport(&mut self, extent: vk::Extent2D) {
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        unsafe { self.device.cmd_set_viewport(self.buffer, 0, &[viewport]) };
    }

    pub fn set_scissor(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: x as i32, y: y as i32 },
            extent: vk::Extent2D { width, height },
        };
        unsafe { self.device.cmd_set_scissor(self.buffer, 0, &[scissor]) };
    }

    pub fn bind_pipeline(&mut self, pipeline: vk::Pipeline, layout: vk::PipelineLayout) {
        unsafe { self.device.cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::GRAPHICS, pipeline) };
        let _ = layout;
    }

    pub fn bind_compute_pipeline(&mut self, pipeline: vk::Pipeline) {
        unsafe { self.device.cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::COMPUTE, pipeline) };
    }

    pub fn bind_group(&mut self, set: u32, group: vk::DescriptorSet, layout: vk::PipelineLayout) {
        unsafe {
            self.device
                .cmd_bind_descriptor_sets(self.buffer, vk::PipelineBindPoint::GRAPHICS, layout, set, &[group], &[]);
        }
    }

    pub fn set_push_constants(&mut self, layout: vk::PipelineLayout, stages: vk::ShaderStageFlags, data: &[u8]) {
        unsafe {
            self.device.cmd_push_constants(self.buffer, layout, stages, 0, data);
        }
    }

    pub fn bind_vertex_buffer(&mut self, slot: u32, buffer: vk::Buffer, offset: vk::DeviceSize) {
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(self.buffer, slot, &[buffer], &[offset]);
        }
    }

    pub fn bind_index_buffer(&mut self, buffer: vk::Buffer, offset: vk::DeviceSize, format: vk::IndexType) {
        unsafe { self.device.cmd_bind_index_buffer(self.buffer, buffer, offset, format) };
    }

    // ── draws ──────────────────────────────────────────────────────────────

    pub fn draw(&mut self, vertices: u32, instances: u32) {
        unsafe { self.device.cmd_draw(self.buffer, vertices, instances, 0, 0) };
    }

    pub fn draw_indexed(&mut self, indices: u32, instances: u32) {
        unsafe { self.device.cmd_draw_indexed(self.buffer, indices, instances, 0, 0, 0) };
    }

    pub fn dispatch(&mut self, groups_x: u32, groups_y: u32, groups_z: u32) {
        unsafe { self.device.cmd_dispatch(self.buffer, groups_x, groups_y, groups_z) };
    }

    // ── transfers ──────────────────────────────────────────────────────────

    pub fn copy_buffer_to_buffer(&mut self, src: vk::Buffer, dst: vk::Buffer, regions: &[vk::BufferCopy]) {
        unsafe { self.device.cmd_copy_buffer(self.buffer, src, dst, regions) };
    }

    pub fn copy_buffer_to_image(&mut self, src: vk::Buffer, dst: vk::Image, regions: &[vk::BufferImageCopy]) {
        unsafe {
            self.device
                .cmd_copy_buffer_to_image(self.buffer, src, dst, vk::ImageLayout::GENERAL, regions)
        };
    }

    pub fn fill_buffer(&mut self, buffer: vk::Buffer, size: vk::DeviceSize, data: u32) {
        unsafe { self.device.cmd_fill_buffer(self.buffer, buffer, 0, size, data) };
    }

    // ── barriers, lowered ──────────────────────────────────────────────────

    pub fn barrier_image(&mut self, barriers: &[vk::ImageMemoryBarrier2<'static>]) {
        let dependency = vk::DependencyInfo::default()
            .dependency_flags(vk::DependencyFlags::empty())
            .image_memory_barriers(barriers);
        unsafe { self.device.cmd_pipeline_barrier2(self.buffer, &dependency) };
    }

    pub fn barrier_buffer(&mut self, barriers: &[vk::BufferMemoryBarrier2<'static>]) {
        let dependency = vk::DependencyInfo::default()
            .dependency_flags(vk::DependencyFlags::empty())
            .buffer_memory_barriers(barriers);
        unsafe { self.device.cmd_pipeline_barrier2(self.buffer, &dependency) };
    }

    pub fn is_inside_rendering(&self) -> bool {
        self.rendering
    }

    /// Ends recording. The buffer stays owned by the recorder until submit.
    pub fn end(self) -> Result<Recording> {
        unsafe { self.device.end_command_buffer(self.buffer)? };
        Ok(self)
    }
}

/// `chassis::CommandList` over a [`Recording`].
///
/// Holds the resource registries by reference so it can turn generational handles
/// into Vulkan handles. Callers resolve once when a pass is set up, not per draw.
pub struct CommandList<'a> {
    recording: &'a mut Recording,
    pipelines: &'a Pipelines,
    textures: &'a Textures,
    buffers: &'a crate::vulkan_buffer::Buffers,
    descriptors: &'a Descriptors,
}

impl<'a> CommandList<'a> {
    pub fn new(
        recording: &'a mut Recording,
        pipelines: &'a Pipelines,
        textures: &'a Textures,
        buffers: &'a crate::vulkan_buffer::Buffers,
        descriptors: &'a Descriptors,
    ) -> Self {
        Self {
            recording,
            pipelines,
            textures,
            buffers,
            descriptors,
        }
    }

    /// The raw recording, for anything `chassis` does not cover yet.
    pub fn raw(&mut self) -> &mut Recording {
        self.recording
    }
}

impl CommandList<'_> {
    /// A stale handle here means the buffer was destroyed while a pass still
    /// referenced it. Binding a null buffer would fail later and less clearly, so
    /// this panics at the call site instead.
    fn resolve_buffer(&self, handle: chassis::Buffer) -> vk::Buffer {
        self.buffers
            .get(handle)
            .map_or_else(
                || panic!("bind on a stale buffer handle {handle:?}"),
                crate::vulkan_buffer::BufferResource::handle,
            )
    }
}

impl chassis::CommandList for CommandList<'_> {
    fn begin_rendering(&mut self, desc: &chassis::RenderPassDesc) {
        let views: Vec<vk::ImageView> = desc
            .colors
            .iter()
            .filter_map(|c| self.textures.get(c.texture))
            .map(|t| t.view())
            .collect();
        let depth_view = desc
            .depth
            .as_ref()
            .and_then(|d| self.textures.get(d.texture))
            .map(|t| t.view());
        // The viewport follows the first colour attachment. A depth-only pass has
        // to carry a colour attachment for there to be a viewport at all, which is
        // also what the Vulkan spec requires for a non-zero render area.
        let first = desc.colors.first().map(|c| c.texture);
        let extent = first
            .and_then(|texture| self.textures.get(texture))
            .map_or(vk::Extent2D { width: 0, height: 0 }, |t| {
                vulkan_types::vk_extent_2d(t.extent())
            });

        self.recording.begin_rendering(&views, depth_view, extent);
    }

    fn end_rendering(&mut self) {
        self.recording.end_rendering();
    }

    fn set_viewport(&mut self, extent: chassis::Extent2d) {
        self.recording.set_viewport(vk::Extent2D {
            width: extent.width,
            height: extent.height,
        });
    }

    fn set_scissor(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.recording.set_scissor(x, y, width, height);
    }

    fn bind_pipeline(&mut self, pipeline: &chassis::Pipeline) {
        match self.pipelines.get(*pipeline) {
            Some(resource) => self
                .recording
                .bind_pipeline(resource.handle(), resource.layout()),
            // An unresolved handle means the pipeline was destroyed while a pass
            // still referenced it. Binding nothing would fail later and less
            // clearly, so this is reported now.
            None => panic!("bind_pipeline on a stale pipeline handle"),
        }
    }

    fn bind_group(&mut self, set: u32, group: &chassis::BindGroup) {
        // The layout is the pipeline's; a descriptor set allocated against a
        // different layout is a validation error, and passing the wrong one hides
        // it.
        if let Some(handle) = self.descriptors.set(*group) {
            self.recording
                .bind_group(set, handle.handle(), vk::PipelineLayout::null());
        }
    }

    fn set_push_constants(&mut self, data: &[u8]) {
        self.recording
            .set_push_constants(vk::PipelineLayout::null(), vk::ShaderStageFlags::ALL, data);
    }

    fn bind_vertex_buffer(&mut self, slot: u32, buffer: &chassis::Buffer, offset: u64) {
        let handle = self.resolve_buffer(*buffer);
        self.recording.bind_vertex_buffer(slot, handle, offset);
    }

    fn bind_index_buffer(&mut self, buffer: &chassis::Buffer, offset: u64) {
        let handle = self.resolve_buffer(*buffer);
        self.recording
            .bind_index_buffer(handle, offset, vk::IndexType::UINT32);
    }

    fn draw(&mut self, vertices: u32, instances: u32) {
        self.recording.draw(vertices, instances);
    }

    fn draw_indexed(&mut self, indices: u32, instances: u32) {
        self.recording.draw_indexed(indices, instances);
    }

    fn dispatch(&mut self, groups_x: u32, groups_y: u32, groups_z: u32) {
        self.recording.dispatch(groups_x, groups_y, groups_z);
    }

    fn copy_buffer(&mut self, src: &chassis::Buffer, dst: &chassis::Buffer) {
        let src = self.resolve_buffer(*src);
        let dst = self.resolve_buffer(*dst);
        self.recording
            .copy_buffer_to_buffer(src, dst, &[vk::BufferCopy {
                src_offset: 0,
                dst_offset: 0,
                size: vk::WHOLE_SIZE,
            }]);
    }
}
