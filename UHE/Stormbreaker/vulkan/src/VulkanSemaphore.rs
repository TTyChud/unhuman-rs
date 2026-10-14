//! Semaphores.
//!
//! Mirrors `VulkanSemaphore.cpp` (the tier-switching submit) and
//! `VulkanBinarySemaphore.cpp`. The C++ tree's `VulkanBinarySemaphore.h` and
//! `VulkanBinaryFence.h` both define the same type, which is the ODR violation
//! called out in `docs/vulkan-sync-and-rendergraph.md:87-90`; the fence side lives
//! in `fence.rs` and this file owns semaphores.
//!
//! Binary semaphores exist on every tier. Timeline semaphores need Vulkan 1.2, and
//! the whole point of the `SyncTier` ladder is that callers never have to ask: they
//! ask for a chain and the tier decides whether it rides one semaphore with values
//! or one binary semaphore per link. That switch is in `submit_encoder.rs`.

use std::time::Duration;

use ash::vk;

use crate::vulkan_error::{BackendError, Result};

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

    /// Timeline semaphores need Vulkan 1.2. Whether to ask for one is the tier's
    /// decision, not the caller's — see `Recorder::uses_timeline`.
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

    /// Current timeline value. Binary semaphores have none, which is the whole
    /// difference the tier ladder papers over.
    pub fn value(&self) -> Result<u64> {
        if !self.timeline {
            return Err(BackendError::NotTimeline);
        }
        unsafe { self.device.get_semaphore_counter_value(self.semaphore) }
            .map_err(|e| BackendError::vk("vkGetSemaphoreCounterValue", e))
    }

    /// Blocks on the host until the timeline reaches `value`.
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

    /// Waits for the host, consuming the value. This is the form the frame loop
    /// uses for frames in flight: a timeline semaphore that is only waited on
    /// would grow its value forever and eventually overflow.
    pub fn consume(&self, value: u64) -> Result<bool> {
        if !self.timeline {
            return Err(BackendError::NotTimeline);
        }
        self.wait_for_value(value, Duration::from_nanos(u64::MAX))
            .map(|reached| {
                let _ = reached;
                true
            })
    }

    pub fn destroy(&self, device: &ash::Device) {
        unsafe { device.destroy_semaphore(self.semaphore, None) };
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        unsafe { self.device.destroy_semaphore(self.semaphore, None) };
    }
}
