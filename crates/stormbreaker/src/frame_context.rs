//! Per-thread command list recording and graph-ordered submission.
//!
//! Mirrors `VulkanFrameContext.cpp`. This is the orchestrator: it owns the
//! per-list pools from `command_pool.rs`, hands out buffers wrapped as
//! `command_buffer::Recording`, and turns a batch of finished recordings into a
//! submission through `submit_encoder.rs`.
//!
//! The recording model is the part the C++ engine cannot express.
//! `VulkanCommandBuffer` owns one primary buffer for the whole frame, and
//! `VulkanDevice::EndFrameGraph` chains every pass to its predecessor to work
//! around it:
//!
//! ```cpp
//! // VulkanDevice.cpp:630-637 — forces total order
//! for (pass : orderedPasses)
//!     if (previous) pass.dependencies.push_back(previous);
//!     previous = pass;
//! ```
//!
//! The comment there admits it defeats the DAG until per-thread command pools
//! exist. Here every worker owns a pool and a buffer, records in parallel with no
//! lock, and submission walks the *compiled* order afterwards. Recording is
//! parallel; submission is not, which is exactly what Vulkan requires.

use ash::{vk, Device};

use crate::command_buffer::Recording;
use crate::command_pool;
use crate::error::{BackendError, Result};
use crate::extension_check::SyncTier;
use crate::submit_encoder::Chain;

/// Secondary buffers, allocated once and recycled.
pub struct Recorder {
    device: Device,
    queue: vk::Queue,
    family: u32,
    tier: SyncTier,

    /// One pool per list. Slot `i` belongs to pool `i`, and the contract is that
    /// list `i` is recorded on thread `i`.
    pools: Vec<vk::CommandPool>,
    /// `(slot, buffer)`. The slot is the pool index, so it is a stable identity
    /// rather than a per-frame position.
    free: Vec<(u32, vk::CommandBuffer)>,

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

        let mut pools = Vec::with_capacity(max_lists);
        let mut free = Vec::with_capacity(max_lists);
        for slot in 0..max_lists {
            match command_pool::create(device, family)
                .and_then(|pool| command_pool::allocate(device, pool).map(|buf| (pool, buf)))
            {
                Ok((pool, buffer)) => {
                    pools.push(pool);
                    free.push((slot as u32, buffer));
                }
                Err(e) => {
                    for existing in pools.drain(..) {
                        command_pool::reset_and_destroy(device, existing);
                    }
                    return Err(e);
                }
            }
        }

        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let fence = match unsafe { device.create_fence(&fence_info, None) } {
            Ok(fence) => fence,
            Err(e) => {
                for existing in pools.drain(..) {
                    command_pool::reset_and_destroy(device, existing);
                }
                return Err(BackendError::vk("vkCreateFence", e));
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
                    for existing in pools.drain(..) {
                        command_pool::reset_and_destroy(device, existing);
                    }
                    return Err(BackendError::vk("vkCreateSemaphore", e));
                }
            }
        } else {
            let info = vk::SemaphoreCreateInfo::default();
            for _ in 0..max_lists {
                match unsafe { device.create_semaphore(&info, None) } {
                    Ok(s) => chain.push(s),
                    Err(e) => {
                        unsafe { device.destroy_fence(fence, None) };
                        for existing in pools.drain(..) {
                            command_pool::reset_and_destroy(device, existing);
                        }
                        return Err(BackendError::vk("vkCreateSemaphore", e));
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

    /// The pool backing list `slot`, for callers allocating their own buffers.
    pub fn pool_for_slot(&self, slot: u32) -> Option<vk::CommandPool> {
        self.pools.get(slot as usize).copied()
    }

    /// How many lists can be in flight at once. One per recording thread.
    pub fn capacity(&self) -> usize {
        self.free.len()
    }

    /// Hands out one recording per list, in ascending slot order.
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
            let (slot, buffer) = self.free.pop().expect("checked above");
            let info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            unsafe {
                // A recycled buffer is in the executable state after its last
                // submit; re-recording without a reset is invalid usage.
                self.device
                    .reset_command_buffer(buffer, vk::CommandBufferResetFlags::empty())?;
                self.device.begin_command_buffer(buffer, &info)?;
            }
            out.push(Recording::new(self.device.clone(), buffer, slot));
        }
        out.sort_by_key(Recording::slot);
        Ok(out)
    }

    /// Submits in slot order, each waiting on its predecessor, then blocks until
    /// the frame's work is done.
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

        // Reset once, *before* the submit. Resetting again afterwards unsigns a
        // fence the in-flight submit is about to signal, which is illegal and
        // segfaults on Mesa.
        unsafe { self.device.reset_fences(&[self.fence])? };

        let chain = Chain {
            device: &self.device,
            queue: self.queue,
            fence: self.fence,
            buffers: &buffers,
            timeline: self.timeline,
            binary_chain: &self.chain,
            tier: self.tier,
            value_base,
        };
        chain.submit()?;

        unsafe {
            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)?;
        }

        for recording in ordered {
            self.free.push((recording.slot(), recording.into_buffer()));
        }
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
    /// semaphore per link. Diagnostics only — callers do not branch on it.
    pub fn uses_timeline(&self) -> bool {
        self.timeline.is_some()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.wait_for_fences(&[self.fence], true, u64::MAX);
            self.device.destroy_fence(self.fence, None);
            if let Some(timeline) = self.timeline {
                self.device.destroy_semaphore(timeline, None);
            }
            for semaphore in self.chain.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
        }
        for pool in self.pools.drain(..) {
            command_pool::reset_and_destroy(&self.device, pool);
        }
    }
}
