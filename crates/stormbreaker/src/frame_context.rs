//! Per-thread command list recording and graph-ordered submission.
//!
//! This is the part the C++ engine cannot do. `VulkanCommandBuffer` owns a
//! single primary buffer and `VulkanDevice::EndFrameGraph` chains every pass to
//! its predecessor to work around it:
//!
//! ```cpp
//! // VulkanDevice.cpp:630-637 — forces total order
//! for (pass : orderedPasses)
//!     if (previous) pass.dependencies.push_back(previous);
//!     previous = pass;
//! ```
//!
//! The comment there admits it defeats the DAG until per-thread command pools
//! exist. Here every worker records into its own secondary `CommandBuffer`, in
//! parallel and with no lock, and submission walks the *compiled* order
//! afterwards. Recording is parallel; submission is not, which is exactly what
//! Vulkan requires.
//!
//! **One `CommandPool` per list, not one shared pool.** The first cut shared a
//! single pool across every worker, on the reasoning that a pool only
//! serialises allocation and reset. The validation layer says otherwise:
//!
//! ```text
//! UNASSIGNED-Threading-MultipleThreads-Write
//! vkCmdCopyBuffer(): THREADING ERROR : object of type VkCommandPool is
//! simultaneously used in current thread A (vkCmdCopyBuffer) and thread B
//! ```
//!
//! The pool is the external-synchronisation object for recording, not just for
//! allocation. So list `i` is allocated from pool `i`, and the contract is that
//! list `i` is recorded on thread `i`.
//!
//! The lists are **primary** command buffers, not secondary. A secondary buffer
//! may not be submitted to a queue at all: it has to be executed from a primary
//! through `vkCmdExecuteCommands`, which means one thread has to re-record a
//! primary every frame issuing an execute for every list. That single recorder is
//! exactly the bottleneck this design exists to remove. Primary buffers are
//! submitted directly, so each worker's list is a complete submission of its own.
//!
//! The chain between submits uses one timeline semaphore with increasing values
//! when the device's tier has one, and binary semaphores otherwise. Both
//! express the same edge; only the width differs.

use ash::{vk, Device};

use crate::error::{BackendError, Result};
use crate::extension_check::SyncTier;

/// A command buffer being recorded on one thread.
pub struct Recording {
    device: Device,
    buffer: vk::CommandBuffer,
    /// Position in the submission chain, assigned at `take` time because that is
    /// the order the graph compiled.
    slot: u32,
}

impl Recording {
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

pub struct Recorder {
    device: Device,
    queue: vk::Queue,
    family: u32,
    tier: SyncTier,

    /// One pool per list. List `i` belongs to pool `i`.
    pools: Vec<vk::CommandPool>,
    /// Secondary command buffers, allocated once and recycled. Allocating per
    /// frame is what makes a naive implementation stutter.
    free: Vec<(u32, vk::CommandBuffer, usize)>,

    /// One timeline semaphore carries the whole chain when the tier has one.
    timeline: Option<vk::Semaphore>,
    /// Otherwise one binary semaphore per link.
    chain: Vec<vk::Semaphore>,
    fence: vk::Fence,
}

impl Recorder {
    pub fn new(
        device: &Device,
        queue: vk::Queue,
        family: u32,
        tier: SyncTier,
        max_lists: usize,
    ) -> Result<Self> {
        if max_lists == 0 {
            return Err(BackendError::ListArenaExhausted {
                requested: 1,
                available: 0,
            });
        }

        let pool_info = vk::CommandPoolCreateInfo::default()
            // RESET_COMMAND_BUFFER is required for the `vkResetCommandBuffer`
            // that returns a recycled list to the initial state. Without it:
            // VUID-vkResetCommandBuffer-commandBuffer-00046, and the device
            // does not reliably survive the violation.
            .flags(
                vk::CommandPoolCreateFlags::TRANSIENT
                    | vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            )
            .queue_family_index(family);

        // One pool per list. The pool, not the command buffer, is the external
        // synchronisation object during recording.
        let mut pools: Vec<vk::CommandPool> = Vec::with_capacity(max_lists);
        let mut free: Vec<(u32, vk::CommandBuffer, usize)> = Vec::with_capacity(max_lists);
        for pool_index in 0..max_lists {
            let pool = match unsafe { device.create_command_pool(&pool_info, None) } {
                Ok(pool) => pool,
                Err(e) => {
                    for existing in pools.drain(..) {
                        unsafe { device.destroy_command_pool(existing, None) };
                    }
                    return Err(e.into());
                }
            };
            let allocate_info = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            match unsafe { device.allocate_command_buffers(&allocate_info) } {
                Ok(mut buffers) => {
                    let buffer = buffers.remove(0);
                    pools.push(pool);
                    free.push((pool_index as u32, buffer, pool_index));
                }
                Err(e) => {
                    unsafe { device.destroy_command_pool(pool, None) };
                    for existing in pools.drain(..) {
                        unsafe { device.destroy_command_pool(existing, None) };
                    }
                    return Err(e.into());
                }
            }
        }

        let destroy_pools = |device: &ash::Device, pools: &mut Vec<vk::CommandPool>| {
            for pool in pools.drain(..) {
                unsafe { device.destroy_command_pool(pool, None) };
            }
        };

        let fence_info = vk::FenceCreateInfo::default()
            .flags(vk::FenceCreateFlags::SIGNALED);
        let fence = match unsafe { device.create_fence(&fence_info, None) } {
            Ok(f) => f,
            Err(e) => {
                destroy_pools(device, &mut pools);
                return Err(e.into());
            }
        };

        let mut timeline = None;
        let mut chain = Vec::new();
        if tier.supports_timeline_semaphores() {
            let type_info = vk::SemaphoreTypeCreateInfo::default()
                .semaphore_type(vk::SemaphoreType::TIMELINE)
                .initial_value(0);
            let info = vk::SemaphoreCreateInfo {
                p_next: &type_info as *const _ as *const _,
                ..Default::default()
            };
            match unsafe { device.create_semaphore(&info, None) } {
                Ok(s) => timeline = Some(s),
                Err(e) => {
                    unsafe { device.destroy_fence(fence, None) };
                    destroy_pools(device, &mut pools);
                    return Err(e.into());
                }
            }
        } else {
            let info = vk::SemaphoreCreateInfo::default();
            for _ in 0..max_lists {
                match unsafe { device.create_semaphore(&info, None) } {
                    Ok(s) => chain.push(s),
                    Err(e) => {
                        unsafe { device.destroy_fence(fence, None) };
                        destroy_pools(device, &mut pools);
                        return Err(e.into());
                    }
                }
            }
        }

        Ok(Self {
            device: device.clone(),
            queue,
            family,
            tier,
            pools,
            free,
            timeline,
            chain,
            fence,
        })
    }

    /// The pool backing list `slot`. Callers that allocate their own buffers need
    /// this to keep them on the right pool.
    pub fn pool_for_slot(&self, slot: u32) -> Option<vk::CommandPool> {
        self.pools.get(slot as usize).copied()
    }

    /// How many secondary lists this arena can hand out at once. One per
    /// recording thread is the useful number.
    pub fn capacity(&self) -> usize {
        self.free.len()
    }

    /// Hands out one recording per list, numbered in the order requested.
    ///
    /// `count` is the length of the compiled frame, so slot `i` is the pass the
    /// graph put `i`th. The underlying buffers are recycled in whatever order the
    /// free list yields, which is invisible: slot is a per-frame *position*, not
    /// a buffer identity.
    ///
    /// Fails rather than truncating: a frame that asked for more lists than the
    /// arena holds would otherwise silently drop passes.
    pub fn take(&mut self, count: usize) -> Result<Vec<Recording>> {
        if count > self.free.len() {
            return Err(BackendError::ListArenaExhausted {
                requested: count,
                available: self.free.len(),
            });
        }

        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let (buffer_slot, buffer, _pool) = self.free.pop().expect("checked above");
            let info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            unsafe {
                // A recycled buffer is in the executable state after its last
                // submit; re-recording without a reset is invalid usage.
                self.device
                    .reset_command_buffer(buffer, vk::CommandBufferResetFlags::empty())?;
                self.device.begin_command_buffer(buffer, &info)?;
            }
            out.push(Recording {
                device: self.device.clone(),
                buffer,
                slot: buffer_slot,
            });
        }
        out.sort_by_key(Recording::slot);
        Ok(out)
    }

    /// Submits in slot order, each waiting on its predecessor.
    ///
    /// `value_base` is the timeline value this chain starts from; frames in
    /// flight advance it by the chain length.
    pub fn submit(&mut self, recordings: Vec<Recording>, value_base: u64) -> Result<()> {
        let mut ordered = recordings;
        ordered.sort_by_key(Recording::slot);
        if ordered.is_empty() {
            return Ok(());
        }
        let buffers: Vec<vk::CommandBuffer> = ordered.iter().map(Recording::handle).collect();

        unsafe { self.device.reset_fences(&[self.fence])? };
        match self.tier {
            // Timeline values need `vkQueueSubmit2`, so the whole chain rides the
            // Sync2 encoding. This mirrors the C++ `VulkanSemaphore::Submit`
            // tier switch (`VulkanSemaphore.cpp:63-145`), with the same reason:
            // timeline semaphores do not exist below Vulkan 1.2.
            SyncTier::Sync2 | SyncTier::Sync2Timeline => self.submit_sync2(&buffers, value_base)?,
            SyncTier::Legacy => self.submit_legacy(&buffers)?,
        }

        // Wait only. The fence was reset *before* the submit: resetting it again
        // here unsigns a fence the in-flight submit is about to signal, which is
        // illegal and segfaults on Mesa.
        unsafe {
            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)?;
        }

        for recording in ordered {
            let pool_index = recording.slot as usize;
            self.free.push((recording.slot, recording.buffer, pool_index));
        }
        Ok(())
    }

    /// `vkQueueSubmit2` chain: one timeline semaphore, value `i` gated on
    /// `i - 1`.
    fn submit_sync2(&self, buffers: &[vk::CommandBuffer], value_base: u64) -> Result<()> {
        let timeline = self.timeline.expect("sync2 tier without a timeline semaphore");

        // Built first and never grown again: `SubmitInfo2` holds raw pointers
        // into these, so a reallocation would dangle them.
        let mut waits: Vec<vk::SemaphoreSubmitInfo> = Vec::with_capacity(buffers.len());
        let mut signals: Vec<vk::SemaphoreSubmitInfo> = Vec::with_capacity(buffers.len());
        for index in 0..buffers.len() {
            // Timeline values must strictly increase: VUID-VkSubmitInfo2-
            // semaphore-03882 requires a signal value greater than the
            // semaphore's current value, which is `value_base` on entry. So the
            // first link signals `value_base + 1`.
            waits.push(if index == 0 {
                vk::SemaphoreSubmitInfo::default()
            } else {
                vk::SemaphoreSubmitInfo {
                    semaphore: timeline,
                    value: value_base + index as u64,
                    ..Default::default()
                }
            });
            signals.push(vk::SemaphoreSubmitInfo {
                semaphore: timeline,
                value: value_base + index as u64 + 1,
                ..Default::default()
            });
        }

        let command_infos: Vec<vk::CommandBufferSubmitInfo> = buffers
            .iter()
            .map(|b| vk::CommandBufferSubmitInfo {
                command_buffer: *b,
                device_mask: 0,
                ..Default::default()
            })
            .collect();

        let mut submits: Vec<vk::SubmitInfo2> = Vec::with_capacity(buffers.len());
        for index in 0..buffers.len() {
            let waits_slice: &[vk::SemaphoreSubmitInfo] = if index == 0 {
                &[]
            } else {
                &waits[index..index + 1]
            };
            let infos = &command_infos[index..index + 1];
            submits.push(vk::SubmitInfo2 {
                wait_semaphore_info_count: waits_slice.len() as u32,
                p_wait_semaphore_infos: waits_slice.as_ptr().cast(),
                command_buffer_info_count: 1,
                p_command_buffer_infos: infos.as_ptr(),
                signal_semaphore_info_count: 1,
                p_signal_semaphore_infos: signals[index..index + 1].as_ptr(),
                ..Default::default()
            });
        }

        unsafe { self.device.queue_submit2(self.queue, &submits, self.fence)? };
        Ok(())
    }

    /// `vkQueueSubmit` chain: one binary semaphore per link, all stage masks at
    /// the bottom of the pipe because a copy's wait only needs to precede it.
    fn submit_legacy(&self, buffers: &[vk::CommandBuffer]) -> Result<()> {
        let all_stages = vk::PipelineStageFlags::ALL_COMMANDS;
        let mut submits: Vec<vk::SubmitInfo> = Vec::with_capacity(buffers.len());

        for index in 0..buffers.len() {
            let waits: &[vk::Semaphore] = if index == 0 {
                &[]
            } else {
                &self.chain[index - 1..index]
            };
            let signals: &[vk::Semaphore] = &self.chain[index..index + 1];
            submits.push(vk::SubmitInfo {
                wait_semaphore_count: waits.len() as u32,
                p_wait_semaphores: waits.as_ptr(),
                p_wait_dst_stage_mask: waits.as_ptr().cast::<vk::PipelineStageFlags>(),
                command_buffer_count: 1,
                p_command_buffers: buffers[index..].as_ptr(),
                signal_semaphore_count: 1,
                p_signal_semaphores: signals.as_ptr(),
                ..Default::default()
            });
            let _ = all_stages;
        }

        unsafe { self.device.queue_submit(self.queue, &submits, self.fence)? };
        Ok(())
    }

    pub fn tier(&self) -> SyncTier {
        self.tier
    }

    pub fn queue_family(&self) -> u32 {
        self.family
    }

    pub fn fence(&self) -> vk::Fence {
        self.fence
    }

    /// True when the chain rides one timeline semaphore rather than one binary
    /// semaphore per link.
    pub fn uses_timeline(&self) -> bool {
        self.timeline.is_some()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_fence(self.fence, None);
            if let Some(timeline) = self.timeline {
                self.device.destroy_semaphore(timeline, None);
            }
            for semaphore in self.chain.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            for pool in self.pools.drain(..) {
                self.device.destroy_command_pool(pool, None);
            }
        }
    }
}