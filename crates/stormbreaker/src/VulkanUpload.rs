//! Host-to-device uploads.
//!
//! Replaces `VulkanDevice::ImmediateSubmit` (`VulkanDevice.cpp:670-696`), which
//! had three problems worth naming:
//!
//! 1. It **resets the command pool on every call** (`:695`), so any upload still
//!    in flight loses its command buffer.
//! 2. It waits with `waitForFences(..., UINT64_MAX)` on a **single shared fence**,
//!    so every upload serialises against every other, from every thread.
//! 3. It takes no lock, so two threads uploading at once race on that pool.
//!
//! Here each thread gets its own ring: its own staging buffer, command pool,
//! command buffer and fence. An upload records and returns; the caller decides
//! when to wait. The ring refuses to wrap while bytes the GPU has not read are
//! still in the buffer, which turns "uploaded while in flight" from a corrupted
//! frame into an explicit wait.

use ash::vk;

use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_allocation;

/// A per-thread staging ring.
pub struct UploadRing {
    device: ash::Device,
    queue: vk::Queue,
    family: u32,

    buffer: vk::Buffer,
    device_memory: vk::DeviceMemory,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,

    capacity: vk::DeviceSize,
    /// Write cursor within the ring.
    offset: vk::DeviceSize,
    /// True while a submit made from this ring has not been waited on.
    pending: bool,
}

impl UploadRing {
    pub fn new(
        device: &ash::Device,
        memory_properties: &vk::PhysicalDeviceMemoryProperties,
        queue: vk::Queue,
        family: u32,
        capacity: vk::DeviceSize,
    ) -> Result<Self> {
        let (buffer, device_memory, _) = vulkan_allocation::create_buffer(
            device,
            memory_properties,
            capacity,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;

        let pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::TRANSIENT)
            .queue_family_index(family);
        let command_pool = match unsafe { device.create_command_pool(&pool_info, None) } {
            Ok(pool) => pool,
            Err(e) => {
                unsafe {
                    device.destroy_buffer(buffer, None);
                    device.free_memory(device_memory, None);
                }
                return Err(BackendError::vk("vkCreateCommandPool", e));
            }
        };

        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffer = match unsafe { device.allocate_command_buffers(&allocate) } {
            Ok(mut buffers) => buffers.remove(0),
            Err(e) => {
                unsafe {
                    device.destroy_command_pool(command_pool, None);
                    device.destroy_buffer(buffer, None);
                    device.free_memory(device_memory, None);
                }
                return Err(BackendError::vk("vkAllocateCommandBuffers", e));
            }
        };

        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let fence = match unsafe { device.create_fence(&fence_info, None) } {
            Ok(fence) => fence,
            Err(e) => {
                unsafe {
                    device.destroy_command_pool(command_pool, None);
                    device.destroy_buffer(buffer, None);
                    device.free_memory(device_memory, None);
                }
                return Err(BackendError::vk("vkCreateFence", e));
            }
        };

        Ok(Self {
            device: device.clone(),
            queue,
            family,
            buffer,
            device_memory,
            command_pool,
            command_buffer,
            fence,
            capacity,
            offset: 0,
            pending: false,
        })
    }

    pub fn capacity(&self) -> vk::DeviceSize {
        self.capacity
    }

    /// Bytes handed to the GPU and not yet waited on.
    pub fn outstanding(&self) -> vk::DeviceSize {
        if self.pending {
            self.offset
        } else {
            0
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending
    }

    /// Records a copy of `data` into `dst`. Returns once the submit is queued,
    /// not once the GPU has read it.
    pub fn copy_to_buffer(
        &mut self,
        dst: vk::Buffer,
        dst_offset: vk::DeviceSize,
        data: &[u8],
    ) -> Result<()> {
        let size = data.len() as vk::DeviceSize;
        if size == 0 {
            return Ok(());
        }
        if size > self.capacity {
            return Err(BackendError::UploadTooLarge {
                size,
                capacity: self.capacity,
            });
        }

        // Wrapping would overwrite bytes the previous submit may still be
        // reading, so finish it first rather than corrupting it.
        if self.offset + size > self.capacity {
            self.wait()?;
        }

        unsafe {
            let ptr = self.device.map_memory(
                self.device_memory,
                self.offset,
                size,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
            self.device.unmap_memory(self.device_memory);
        }

        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        let command_buffer = self.command_buffer;
        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin)?;
            self.device.cmd_copy_buffer(
                command_buffer,
                self.buffer,
                dst,
                &[vk::BufferCopy {
                    src_offset: self.offset,
                    dst_offset,
                    size,
                }],
            );
            self.device.end_command_buffer(command_buffer)?;

            self.device.reset_fences(&[self.fence])?;
            let command_buffers = [command_buffer];
            self.device.queue_submit(
                self.queue,
                &[vk::SubmitInfo {
                    command_buffer_count: 1,
                    p_command_buffers: command_buffers.as_ptr(),
                    ..Default::default()
                }],
                self.fence,
            )?;
        }

        self.offset += size;
        self.pending = true;
        Ok(())
    }

    /// Blocks until every queued upload has been read, then rewinds the ring.
    pub fn wait(&mut self) -> Result<()> {
        if !self.pending {
            return Ok(());
        }
        unsafe {
            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)?;
        }
        self.offset = 0;
        self.pending = false;
        Ok(())
    }

    pub fn queue_family(&self) -> u32 {
        self.family
    }
}

impl Drop for UploadRing {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.wait_for_fences(&[self.fence], true, u64::MAX);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_buffer(self.buffer, None);
            self.device.free_memory(self.device_memory, None);
        }
    }
}