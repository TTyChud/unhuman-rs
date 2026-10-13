//! Device context.
//!
//! This is the C++ `VulkanContext` global bag (`VulkanContext.h:56`:
//! `extern VulkanContext* g_VulkanContext`), turned into an owned value with a
//! real lifetime. The C++ version is dereferenced from
//! `VulkanUtils.cpp:15,56,65,82,123,141,207,253`, `VulkanTexture.cpp:20,138,271`,
//! `VulkanFramebuffer.cpp:10` and `VulkanDescriptorManager.cpp:90,94,113` —
//! every helper reaching up to a process-wide handle for its own device. That is
//! also why `VulkanCommandBuffer.cpp:79-85` has to `static_cast` a global back
//! into a `VulkanDevice` it could already have reached through this struct.
//!
//! Here it is owned by the `Gpu` that owns it, borrowed as `&Context`, and
//! dropped in a defined order.

use ash::{vk, Device, Entry, Instance};

use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_extension_check::{Capabilities, SyncTier};

/// Everything one device owns, in one value with one drop order.
pub struct Context {
    pub entry: Entry,
    pub instance: Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: Device,
    pub graphics_queue: vk::Queue,
    pub capabilities: Capabilities,
    /// True when the validation layer was requested and found.
    pub validation_enabled: bool,
}

/// Order matters: the device must die before the instance it was created from,
/// and the instance before the loader. Both are destroyed explicitly rather than
/// left to field order, because `ash::Device` and `ash::Instance` are handles
/// with no `Drop`.
impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            // `device_wait_idle` before teardown: destroying a device with work
            // in flight is undefined, and the deferred-deletion queue is
            // flushed by `frame_begin` before this can run.
            // The device is being torn down; a failure here means it was
            // already lost, and there is nothing left to report it to.
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

impl Context {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        entry: Entry,
        instance: Instance,
        physical_device: vk::PhysicalDevice,
        device: Device,
        graphics_queue: vk::Queue,
        capabilities: Capabilities,
        validation_enabled: bool,
    ) -> Self {
        Self {
            entry,
            instance,
            physical_device,
            device,
            graphics_queue,
            capabilities,
            validation_enabled,
        }
    }

    pub fn tier(&self) -> SyncTier {
        self.capabilities.sync_tier
    }

    pub fn device_name(&self) -> &str {
        &self.capabilities.device_name
    }

    pub fn graphics_queue_family(&self) -> Result<u32> {
        self.capabilities
            .graphics_queue_family
            .ok_or(BackendError::NoGraphicsQueue)
    }

    pub fn memory_properties(&self) -> vk::PhysicalDeviceMemoryProperties {
        unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        }
    }

    pub fn queue_properties(&self) -> Vec<vk::QueueFamilyProperties> {
        unsafe {
            self.instance
                .get_physical_device_queue_family_properties(self.physical_device)
        }
    }
}
