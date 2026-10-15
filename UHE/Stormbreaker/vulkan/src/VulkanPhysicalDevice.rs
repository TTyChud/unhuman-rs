//! Physical device selection and capability probing.
//!
//! Mirrors `VulkanPhysicalDevice.cpp`.
//!
//! The probing rules here are the ones that are easy to get subtly wrong:
//!
//! * `synchronization2` and `dynamic_rendering` are only reachable through
//!   `PhysicalDeviceVulkan13Features` in this ash version; `timeline_semaphore`
//!   and `descriptor_indexing` live in the 1.2 struct. The chain is
//!   `Features2 -> V13 -> V12` and is only ever assembled on a device reporting
//!   1.3 or newer, because a `pNext` node from an unreported version is a
//!   validation error at best.
//! * Below 1.3 the same capabilities come from enumerating device extensions.
//! * The feature **bits** are the authority, not the extension list. An extension
//!   can be advertised while the feature is unsupported, and only the bit says
//!   whether the entry point actually works.

use ash::{vk, Instance};

use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_extension_check::{version_major, version_minor, QueueFamily};

pub const TIMELINE_SEMAPHORE: &str = "VK_KHR_timeline_semaphore";
pub const SYNC_2: &str = "VK_KHR_synchronization2";
pub const DYNAMIC_RENDERING: &str = "VK_KHR_dynamic_rendering";
pub const MEMORY_BUDGET: &str = "VK_EXT_memory_budget";
pub const DESCRIPTOR_INDEXING: &str = "VK_EXT_descriptor_indexing";

/// What the device reports it can do, before anything is enabled.
#[derive(Clone, Copy, Debug, Default)]
pub struct Probe {
    pub synchronization2: bool,
    pub timeline_semaphore: bool,
    pub dynamic_rendering: bool,
    pub descriptor_indexing: bool,
}

/// True when `api_version` is at least `major.minor`.
pub fn at_least(api_version: u32, major: u32, minor: u32) -> bool {
    let (vmaj, vmin) = (version_major(api_version), version_minor(api_version));
    vmaj > major || (vmaj == major && vmin >= minor)
}

/// Picks the device to use: the first with a graphics-capable queue, preferring a
/// discrete GPU when the driver reports a type.
pub fn choose(instance: &Instance, devices: &[vk::PhysicalDevice]) -> Result<vk::PhysicalDevice> {
    let mut best: Option<(bool, vk::PhysicalDevice)> = None;

    for device in devices {
        let families = unsafe { instance.get_physical_device_queue_family_properties(*device) };
        if !families
            .iter()
            .any(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        {
            continue;
        }
        let properties = unsafe { instance.get_physical_device_properties(*device) };
        let discrete = properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU;
        match best {
            Some((true, _)) => {}
            _ => best = Some((discrete, *device)),
        }
    }

    best.map(|(_, device)| device)
        .ok_or(BackendError::NoGraphicsQueue)
}

/// Reads capability bits, falling back to extension enumeration below 1.3.
pub fn probe_capabilities(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    api_version: u32,
    check: &crate::vulkan_extension_check::ExtensionCheck,
) -> Probe {
    if !at_least(api_version, 1, 3) {
        // Below 1.3 the API version cannot supply these, so it is the extension
        // string or nothing. Above it, the feature structs below are the better
        // answer and this branch is not taken.
        return Probe {
            synchronization2: check.supports_sync2(),
            timeline_semaphore: check.supports_timeline_semaphores(),
            dynamic_rendering: check.supports_dynamic_rendering(),
            descriptor_indexing: check.supports_descriptor_indexing(),
        };
    }

    // The chain nodes must outlive the query they are handed to.
    let mut v12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut v13 = vk::PhysicalDeviceVulkan13Features::default();
    let mut features2 = vk::PhysicalDeviceFeatures2::default();
    unsafe {
        v13.p_next = (&raw mut v12).cast::<std::os::raw::c_void>();
        features2.p_next = (&raw mut v13).cast::<std::os::raw::c_void>();
        instance.get_physical_device_features2(physical_device, &mut features2);
    }

    Probe {
        synchronization2: v13.synchronization2 == vk::TRUE,
        timeline_semaphore: v12.timeline_semaphore == vk::TRUE,
        dynamic_rendering: v13.dynamic_rendering == vk::TRUE,
        descriptor_indexing: v12.descriptor_indexing == vk::TRUE,
    }
}

/// Reads the queue families out of the driver.
///
/// `present` is false for every family until `eyes` supplies a surface: the
/// surface loader needs `VK_KHR_surface` enabled on the instance, which happens
/// with window creation, not device creation.
pub fn queue_families(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<Vec<QueueFamily>> {
    let properties =
        unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
    if properties.is_empty() {
        return Err(BackendError::NoQueueFamilies);
    }

    Ok(properties
        .iter()
        .enumerate()
        .map(|(index, family)| QueueFamily {
            index: index as u32,
            graphics: family.queue_flags.contains(vk::QueueFlags::GRAPHICS),
            compute: family.queue_flags.contains(vk::QueueFlags::COMPUTE),
            transfer: family.queue_flags.contains(vk::QueueFlags::TRANSFER),
            present: false,
            timestamp_valid_bits: family.timestamp_valid_bits,
        })
        .collect())
}

pub fn has_device_extension(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    name: &str,
) -> bool {
    unsafe { instance.enumerate_device_extension_properties(physical_device) }
        .map(|extensions| {
            extensions.iter().any(
                |e| unsafe { crate::vulkan_instance::name_of(e.extension_name.as_ptr()) } == name,
            )
        })
        .unwrap_or(false)
}

/// Vulkan stores the device name as a fixed NUL-padded `[c_char; 256]`.
pub fn read_device_name(chars: &[i8; 256]) -> String {
    chars
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8 as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_least_compares_major_then_minor() {
        let v = |major, minor| vk::make_api_version(0, major, minor, 0);
        assert!(!at_least(v(1, 0), 1, 1));
        assert!(at_least(v(1, 1), 1, 1));
        assert!(at_least(v(1, 3), 1, 1));
        assert!(at_least(v(2, 0), 1, 3));
        assert!(!at_least(v(1, 2), 1, 3), "1.2 is below 1.3");
        assert!(at_least(v(1, 4), 1, 3), "1.4 is above 1.3");
    }
}
