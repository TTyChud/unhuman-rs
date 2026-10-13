//! A command buffer being recorded.
//!
//! Mirrors `VulkanCommandBuffer.cpp` — with one structural difference that is the
//! point of the whole design. The C++ class owns *one* primary buffer for the
//! entire frame, which is why `VulkanDevice::EndFrameGraph` chains every pass to
//! its predecessor (`VulkanDevice.cpp:630-637`) and admits in a comment that this
//! defeats the DAG. Here each worker records into its own buffer, so there is
//! nothing to chain and the compiled order survives to the GPU.
//!
//! The buffer also has **no barrier API**. Barriers arrive pre-baked from the
//! render graph, which is what makes `MarkSwapchainRenderedByLegacy`-style
//! handshakes unnecessary.

use ash::{vk, Device};

use crate::vulkan_error::Result;

/// A command buffer being recorded on one thread.
pub struct Recording {
    device: Device,
    buffer: vk::CommandBuffer,
    /// Position in the submission chain, assigned at `take` time because that is
    /// the order the graph compiled.
    slot: u32,
}

impl Recording {
    /// A recording must already be begun; `Recorder::take` does that.
    pub fn new(device: Device, buffer: vk::CommandBuffer, slot: u32) -> Self {
        Self {
            device,
            buffer,
            slot,
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

    pub fn copy_buffer_to_buffer(&mut self, src: vk::Buffer, dst: vk::Buffer, regions: &[vk::BufferCopy]) {
        unsafe { self.device.cmd_copy_buffer(self.buffer, src, dst, regions) }
    }

    pub fn copy_buffer_to_image(&mut self, src: vk::Buffer, dst: vk::Image, regions: &[vk::BufferImageCopy]) {
        unsafe {
            self.device
                .cmd_copy_buffer_to_image(self.buffer, src, dst, vk::ImageLayout::GENERAL, regions)
        }
    }

    pub fn fill_buffer(&mut self, buffer: vk::Buffer, size: vk::DeviceSize, data: u32) {
        unsafe { self.device.cmd_fill_buffer(self.buffer, buffer, 0, size, data) }
    }

    pub fn end(self) -> Result<Recording> {
        unsafe { self.device.end_command_buffer(self.buffer)? };
        Ok(self)
    }

    /// Records an image or buffer barrier, lowered through the same
    /// `SyncTier` decision as every other barrier in the backend.
    pub fn barrier_image(&mut self, barriers: &[vk::ImageMemoryBarrier2<'static>]) {
        let dependency = vk::DependencyInfo::default()
            .dependency_flags(vk::DependencyFlags::empty())
            .image_memory_barriers(barriers);
        unsafe { self.device.cmd_pipeline_barrier2(self.buffer, &dependency) }
    }
}
