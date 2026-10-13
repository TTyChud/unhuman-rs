//! Command pools.
//!
//! Mirrors `VulkanCommandPool.cpp`.
//!
//! **One pool per command list, not one shared pool.** The first cut in this
//! engine shared a single pool across every worker, reasoning that a pool only
//! serialises allocation and reset. The validation layer says otherwise:
//!
//! ```text
//! UNASSIGNED-Threading-MultipleThreads-Write
//! vkCmdCopyBuffer(): THREADING ERROR : object of type VkCommandPool is
//! simultaneously used in current thread A (vkCmdCopyBuffer) and thread B
//! ```
//!
//! The pool — not the command buffer — is the external-synchronisation object
//! during recording. So list `i` is allocated from pool `i`, and the contract is
//! that list `i` is recorded on thread `i`.
//!
//! `TRANSIENT` because every list in this engine is single-use. A graph that
//! replays a pass many times (shadow cascades) will want `SIMULTANEOUS_USE` on
//! the pool instead, and that is a one-flag change here.

use ash::vk;

use crate::vulkan_error::{BackendError, Result};

/// Flags every pool in this engine is created with.
///
/// `RESET_COMMAND_BUFFER` is not optional: it is what makes
/// `vkResetCommandBuffer` legal, and recycling a list without it is
/// `VUID-vkResetCommandBuffer-commandBuffer-00046`, after which the device is
/// allowed to be lost.
///
/// A function rather than a const because ash's flag types do not expose a const
/// bit-or.
pub fn pool_flags() -> vk::CommandPoolCreateFlags {
    vk::CommandPoolCreateFlags::TRANSIENT
        | vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER
}

/// The level every list is allocated at.
///
/// PRIMARY, not SECONDARY. A secondary buffer cannot be submitted to a queue at
/// all — it must be executed from a primary through `vkCmdExecuteCommands`,
/// which puts one thread back in the recording path every frame. That is the
/// exact bottleneck the per-list design exists to remove.
pub const LEVEL: vk::CommandBufferLevel = vk::CommandBufferLevel::PRIMARY;

pub fn create_info(queue_family: u32) -> vk::CommandPoolCreateInfo<'static> {
    vk::CommandPoolCreateInfo::default()
        .flags(pool_flags())
        .queue_family_index(queue_family)
}

/// Creates one pool. Kept separate from `Recorder` so a caller can own a pool for
/// something the recorder does not know about — a primary upload list, say.
pub fn create(device: &ash::Device, queue_family: u32) -> Result<vk::CommandPool> {
    let info = create_info(queue_family);
    unsafe { device.create_command_pool(&info, None) }
        .map_err(|e| BackendError::vk("vkCreateCommandPool", e))
}

/// Allocates one buffer from `pool`.
pub fn allocate(device: &ash::Device, pool: vk::CommandPool) -> Result<vk::CommandBuffer> {
    let info = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(LEVEL)
        .command_buffer_count(1);
    let mut buffers = unsafe { device.allocate_command_buffers(&info) }
        .map_err(|e| BackendError::vk("vkAllocateCommandBuffers", e))?;
    Ok(buffers.remove(0))
}

/// Resets a pool and frees every buffer in it, then destroys the pool.
///
/// Cheaper than per-buffer resets when a whole frame's worth is being recycled.
pub fn reset_and_destroy(device: &ash::Device, pool: vk::CommandPool) {
    unsafe {
        // Reset then destroy: the pool owns buffers, and freeing the buffers by
        // resetting the pool is cheaper and less error prone than per-buffer
        // resets at teardown.
        let _ = device.reset_command_pool(pool, vk::CommandPoolResetFlags::empty());
        device.destroy_command_pool(pool, None);
    }
}
