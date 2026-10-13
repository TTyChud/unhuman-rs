//! Fences and semaphores.
//!
//! Mirrors `VulkanBinaryFence.h` and `VulkanSemaphore.cpp`. One deliberate
//! difference: the C++ tree has `VulkanBinaryFence.h` and
//! `VulkanBinarySemaphore.h` both defining `VulkanBinarySemaphore` — an ODR
//! violation flagged in the engine's own
//! `docs/vulkan-sync-and-rendergraph.md:87-90`. There is one type here.
//!
//! Both types own an `ash::Device` clone so they can destroy their handle in
//! `Drop`. A handle plus a destructor that cannot run is a leak waiting for the
//! end of the process.

use std::time::Duration;

use ash::vk;

use crate::error::{BackendError, Result};

/// A host-side fence. One per frame in flight.
pub struct Fence {
    device: ash::Device,
    fence: vk::Fence,
    name: String,
}

impl Fence {
    /// Created signalled, so the first wait cannot block on a fence nobody has
    /// signalled yet.
    pub fn new(device: &ash::Device, name: impl Into<String>) -> Result<Self> {
        let info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let fence = unsafe { device.create_fence(&info, None) }
            .map_err(|e| BackendError::vk("vkCreateFence", e))?;
        Ok(Self {
            device: device.clone(),
            fence,
            name: name.into(),
        })
    }

    pub fn handle(&self) -> vk::Fence {
        self.fence
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_signalled(&self) -> bool {
        unsafe { self.device.get_fence_status(self.fence) }.is_ok()
    }

    /// Blocks until signalled. Returns false on timeout.
    pub fn wait(&self, timeout: Duration) -> Result<bool> {
        let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        match unsafe { self.device.wait_for_fences(&[self.fence], true, nanos) } {
            Ok(()) => Ok(true),
            Err(vk::Result::TIMEOUT) => Ok(false),
            Err(e) => Err(BackendError::vk("vkWaitForFences", e)),
        }
    }

    /// Blocks indefinitely. Only correct when the caller owns proof that the
    /// work completed, or at shutdown.
    pub fn wait_forever(&self) -> Result<()> {
        unsafe { self.device.wait_for_fences(&[self.fence], true, u64::MAX) }
            .map_err(|e| BackendError::vk("vkWaitForFences", e))
    }

    pub fn reset(&self) -> Result<()> {
        unsafe { self.device.reset_fences(&[self.fence]) }
            .map_err(|e| BackendError::vk("vkResetFences", e))
    }
}

impl Drop for Fence {
    fn drop(&mut self) {
        unsafe { self.device.destroy_fence(self.fence, None) };
    }
}

/// Cross-queue ordering and frame pacing. Binary, or timeline when the device's
/// tier has one.
pub struct Semaphore {
    device: ash::Device,
    semaphore: vk::Semaphore,
    timeline: bool,
}

impl Semaphore {
    pub fn binary(device: &ash::Device) -> Result<Self> {
        let info = vk::SemaphoreCreateInfo::default();
        let semaphore = unsafe { device.create_semaphore(&info, None) }
            .map_err(|e| BackendError::vk("vkCreateSemaphore", e))?;
        Ok(Self {
            device: device.clone(),
            semaphore,
            timeline: false,
        })
    }

    /// Timeline semaphores need Vulkan 1.2. Callers decide that from the tier
    /// rather than discovering it here.
    pub fn timeline(device: &ash::Device, initial: u64) -> Result<Self> {
        let type_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(initial);
        let info = vk::SemaphoreCreateInfo {
            p_next: &type_info as *const _ as *const _,
            ..Default::default()
        };
        let semaphore = unsafe { device.create_semaphore(&info, None) }
            .map_err(|e| BackendError::vk("vkCreateSemaphore", e))?;
        Ok(Self {
            device: device.clone(),
            semaphore,
            timeline: true,
        })
    }

    pub fn handle(&self) -> vk::Semaphore {
        self.semaphore
    }

    pub fn is_timeline(&self) -> bool {
        self.timeline
    }

    /// Current timeline value. Binary semaphores have none, which is the
    /// difference the `SyncTier` ladder exists to paper over.
    pub fn value(&self) -> Result<u64> {
        if !self.timeline {
            return Err(BackendError::NotTimeline);
        }
        unsafe { self.device.get_semaphore_counter_value(self.semaphore) }
            .map_err(|e| BackendError::vk("vkGetSemaphoreCounterValue", e))
    }

    /// Waits on the host until the timeline reaches `value`.
    pub fn wait_for_value(&self, value: u64, timeout: Duration) -> Result<bool> {
        if !self.timeline {
            return Err(BackendError::NotTimeline);
        }
        let semaphores = [self.semaphore];
        let values = [value];
        let wait = vk::SemaphoreWaitInfo {
            semaphore_count: semaphores.len() as u32,
            p_semaphores: semaphores.as_ptr(),
            p_values: values.as_ptr(),
            ..Default::default()
        };
        let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        match unsafe { self.device.wait_semaphores(&wait, nanos) } {
            Ok(()) => Ok(true),
            Err(vk::Result::TIMEOUT) => Ok(false),
            Err(e) => Err(BackendError::vk("vkWaitSemaphores", e)),
        }
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        unsafe { self.device.destroy_semaphore(self.semaphore, None) };
    }
}