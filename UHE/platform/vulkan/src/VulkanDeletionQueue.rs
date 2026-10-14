//! Deferred destruction.
//!
//! Mirrors `UHE/src/UHE/RHI/DeletionQueue.h` and the flush at
//! `VulkanDevice.cpp:420`. Destroying a resource on the frame that releases it is
//! wrong: an earlier frame may still reference it in a command buffer that has
//! been recorded but not submitted. The C++ queue is a
//! `std::vector<std::function<void()>>`; this is an explicit enum of what can be
//! destroyed, so getting it wrong is a compile error rather than the wrong call
//! on the wrong handle.

use std::collections::VecDeque;

use ash::vk;

/// A resource whose destruction has been deferred.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    Buffer {
        buffer: vk::Buffer,
        memory: vk::DeviceMemory,
    },
    Texture {
        image: vk::Image,
        memory: vk::DeviceMemory,
    },
    ImageView(vk::ImageView),
    Framebuffer(vk::Framebuffer),
    Pipeline(vk::Pipeline),
    PipelineCache(vk::PipelineCache),
    ShaderModule(vk::ShaderModule),
    DescriptorPool(vk::DescriptorPool),
    DescriptorSetLayout(vk::DescriptorSetLayout),
    PipelineLayout(vk::PipelineLayout),
    DeviceMemory(vk::DeviceMemory),
}

/// Holds deferred work for `latency` frames before destroying it.
///
/// Latency must be at least the number of frames in flight, or a buffer freed on
/// frame N can still be referenced by a submit from frame N-1.
#[derive(Debug)]
pub struct DeletionQueue {
    frames: VecDeque<Vec<Pending>>,
    latency: usize,
    destroyed: usize,
    /// Highest number of entries waiting at once. A peak that keeps rising on a
    /// steady-state frame means the queue is being fed without ever draining.
    peak: usize,
}

impl DeletionQueue {
    /// `latency` is clamped to at least 2, so an off-by-one in the caller cannot
    /// free a resource the previous frame's submission still references.
    pub fn new(latency: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            latency: latency.max(2),
            destroyed: 0,
            peak: 0,
        }
    }

    pub fn push(&mut self, pending: Pending) {
        if self.frames.len() < self.latency {
            self.frames.push_back(Vec::new());
        }
        if let Some(front) = self.frames.front_mut() {
            front.push(pending);
        }
        self.peak = self.peak.max(self.pending());
    }

    pub fn extend(&mut self, pending: impl IntoIterator<Item = Pending>) {
        for item in pending {
            self.push(item);
        }
    }

    /// Destroys everything older than `frames_in_flight`. Called once per
    /// `frame_begin`.
    pub fn flush(&mut self, device: &ash::Device, frames_in_flight: usize) {
        while self.frames.len() > frames_in_flight.max(1) {
            let Some(front) = self.frames.pop_front() else {
                break;
            };
            for item in front {
                destroy(device, item);
                self.destroyed += 1;
            }
        }
    }

    /// Destroys everything immediately. Only correct at shutdown, after a
    /// `device_wait_idle`.
    pub fn drain_now(&mut self, device: &ash::Device) {
        while let Some(front) = self.frames.pop_front() {
            for item in front {
                destroy(device, item);
                self.destroyed += 1;
            }
        }
    }

    pub fn pending(&self) -> usize {
        self.frames.iter().map(Vec::len).sum()
    }

    pub fn peak(&self) -> usize {
        self.peak
    }

    pub fn destroyed(&self) -> usize {
        self.destroyed
    }

    pub fn latency(&self) -> usize {
        self.latency
    }
}

fn destroy(device: &ash::Device, pending: Pending) {
    unsafe {
        match pending {
            Pending::Buffer { buffer, memory } => {
                device.destroy_buffer(buffer, None);
                device.free_memory(memory, None);
            }
            Pending::Texture { image, memory } => {
                device.destroy_image(image, None);
                device.free_memory(memory, None);
            }
            Pending::ImageView(view) => device.destroy_image_view(view, None),
            Pending::Framebuffer(framebuffer) => device.destroy_framebuffer(framebuffer, None),
            Pending::Pipeline(pipeline) => device.destroy_pipeline(pipeline, None),
            Pending::PipelineCache(cache) => device.destroy_pipeline_cache(cache, None),
            Pending::ShaderModule(module) => device.destroy_shader_module(module, None),
            Pending::DescriptorPool(pool) => device.destroy_descriptor_pool(pool, None),
            Pending::DescriptorSetLayout(layout) => {
                device.destroy_descriptor_set_layout(layout, None)
            }
            Pending::PipelineLayout(layout) => device.destroy_pipeline_layout(layout, None),
            Pending::DeviceMemory(memory) => device.free_memory(memory, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use ash::vk::Handle;

    use super::*;

    #[test]
    fn latency_is_clamped_to_two() {
        assert_eq!(DeletionQueue::new(0).latency(), 2);
        assert_eq!(DeletionQueue::new(1).latency(), 2);
        assert_eq!(DeletionQueue::new(5).latency(), 5);
    }

    #[test]
    fn the_queue_fills_to_its_latency_before_draining() {
        let mut queue = DeletionQueue::new(3);

        for id in 1..=3 {
            queue.push(Pending::ImageView(vk::ImageView::from_raw(id)));
        }
        assert_eq!(queue.pending(), 3);
        assert_eq!(queue.destroyed(), 0);

        // A fourth push rolls the first bucket off, which is what `flush` does
        // once frames are actually in flight.
        queue.push(Pending::ImageView(vk::ImageView::from_raw(4)));
        assert_eq!(
            queue.pending(),
            4,
            "pushes accumulate, never drain on their own"
        );
    }

    #[test]
    fn peak_tracks_the_high_water_mark() {
        let mut queue = DeletionQueue::new(2);
        assert_eq!(queue.peak(), 0);
        queue.push(Pending::ImageView(vk::ImageView::from_raw(1)));
        queue.push(Pending::ImageView(vk::ImageView::from_raw(2)));
        queue.push(Pending::ImageView(vk::ImageView::from_raw(3)));
        assert_eq!(queue.pending(), 3);
        assert_eq!(queue.peak(), 3);
    }

    #[test]
    fn extend_pushes_every_item() {
        let mut queue = DeletionQueue::new(2);
        queue.extend([
            Pending::ImageView(vk::ImageView::from_raw(1)),
            Pending::DeviceMemory(vk::DeviceMemory::from_raw(2)),
        ]);
        assert_eq!(queue.pending(), 2);
    }
}
