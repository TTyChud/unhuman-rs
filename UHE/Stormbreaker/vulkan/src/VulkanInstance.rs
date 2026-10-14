//! Vulkan instance.
//!
//! Mirrors `VulkanInstance.cpp`.
//!
//! Two things here are easy to get wrong and both have bitten this engine:
//!
//! * **Which API version to request.** It is the *loader's* version, capped at
//!   what the engine uses — not a hardcoded 1.1. The loader only exposes entry
//!   points for the version requested here, so asking for less than the engine
//!   intends to use means `vkCmdPipelineBarrier2` and `vkQueueSubmit2` are never
//!   loaded, and ash's stub for a missing function panics. The hardcoded 1.1
//!   that was here first made every Sync2 path die on a 1.4 device.
//! * **No surface extensions yet.** `eyes` owns window creation. Requesting
//!   `VK_KHR_surface` with no surface validates on one loader and not another.
//!
//! The 1.1 *floor* matters too: `vkGetPhysicalDeviceFeatures2` is the only sane
//! way to probe feature chains, and it is core from 1.1.

use ash::{vk, Entry, Instance};

use crate::vulkan_error::{BackendError, Result};

pub const VALIDATION_LAYER: &str = "VK_LAYER_KHRONOS_validation";

/// Highest core version the engine is prepared to load functions for.
const MAX_API: u32 = (1 << 22) | (3 << 12);

/// The version to request, given what the loader reports.
///
/// Spelled with `make_api_version` rather than `(1 << 22) | (1 << 12)`: that was
/// once written as `(0 << 22) | (1 << 12)`, which is version **0.1**, and every
/// instance creation failed with `ERROR_INCOMPATIBLE_DRIVER`.
pub fn requested_api_version(loader_version: u32) -> u32 {
    let floor = vk::make_api_version(0, 1, 1, 0);
    loader_version.clamp(floor, MAX_API)
}

/// What instance creation returns.
///
/// **No `Drop`.** This is a construction step, not an owner: `create_device`
/// always moves the instance and entry into `vulkan_context::Context`, which is
/// what destroys them. A `Drop` here would destroy the instance a second time,
/// and a double `vkDestroyInstance` is a segfault rather than an error.
pub struct InstanceBundle {
    pub entry: Entry,
    pub instance: Instance,
    /// True when the validation layer was requested *and* found.
    pub validation_enabled: bool,
}

pub fn create_instance(engine_name: &std::ffi::CStr, validation: bool) -> Result<InstanceBundle> {
    let entry = unsafe { Entry::load() }.map_err(|e| BackendError::Loader(e.to_string()))?;

    // `enumerate_instance_version` is deprecated *and* panics on a Vulkan 1.0
    // loader, which is exactly the case where the answer matters.
    let loader_version = match unsafe { entry.try_enumerate_instance_version() } {
        Ok(Some(version)) => version,
        Ok(None) | Err(_) => vk::make_api_version(0, 1, 0, 0),
    };

    let validation_enabled = validation && layer_available(&entry, VALIDATION_LAYER);
    let layer_ptrs: Vec<*const std::os::raw::c_char> = if validation_enabled {
        vec![c"VK_LAYER_KHRONOS_validation".as_ptr()]
    } else {
        Vec::new()
    };

    let app_info = vk::ApplicationInfo {
        p_application_name: c"unhuman-rs".as_ptr(),
        application_version: vk::make_api_version(0, 1, 0, 0),
        p_engine_name: engine_name.as_ptr(),
        engine_version: vk::make_api_version(0, 1, 0, 0),
        api_version: requested_api_version(loader_version),
        ..Default::default()
    };

    let instance_info = vk::InstanceCreateInfo {
        p_application_info: &app_info,
        ..Default::default()
    }
    .enabled_layer_names(&layer_ptrs)
    .enabled_extension_names(&[]);

    let instance = unsafe { entry.create_instance(&instance_info, None) }
        .map_err(|e| BackendError::vk("vkCreateInstance", e))?;

    Ok(InstanceBundle {
        entry,
        instance,
        validation_enabled,
    })
}

pub fn layer_available(entry: &Entry, name: &str) -> bool {
    unsafe { entry.enumerate_instance_layer_properties() }
        .map(|layers| {
            layers
                .iter()
                .any(|l| unsafe { name_of(l.layer_name.as_ptr()) } == name)
        })
        .unwrap_or(false)
}

/// Instance extensions the loader offers, for callers that need one.
pub fn available_instance_extensions(entry: &Entry) -> Vec<String> {
    unsafe { entry.enumerate_instance_extension_properties(None) }
        .map(|extensions| {
            extensions
                .iter()
                .map(|e| unsafe { name_of(e.extension_name.as_ptr()) })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads a `*const c_char` the driver owns.
///
/// # Safety
/// `ptr` must be null or point at a NUL-terminated string that outlives the
/// call. Vulkan's extension and layer name arrays satisfy this for the lifetime
/// of the enumeration result they came from.
pub unsafe fn name_of(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_api_version_is_valid() {
        let api = requested_api_version(vk::make_api_version(0, 1, 4, 0));
        assert_eq!(vk::api_version_major(api), 1, "major must be 1, not 0");
        assert!(vk::api_version_minor(api) >= 1);
        assert!(vk::api_version_minor(api) <= 3);
    }

    #[test]
    fn a_high_loader_is_capped_at_what_the_engine_uses() {
        let api = requested_api_version(vk::make_api_version(0, 1, 4, 200));
        assert_eq!(vk::api_version_minor(api), 3);
    }

    #[test]
    fn a_low_loader_still_gets_the_1_1_floor() {
        let api = requested_api_version(vk::make_api_version(0, 1, 0, 0));
        assert_eq!(vk::api_version_minor(api), 1);
    }
}
