//! Device creation.
//!
//! Mirrors `VulkanDevice.cpp`, reduced to the orchestration its C++ counterpart
//! has. The work lives in the three files that correspond to its C++ siblings:
//!
//! | here | C++ |
//! |---|---|
//! | `vulkan_instance.rs` | `VulkanInstance.cpp` |
//! | `vulkan_physical_device.rs` | `VulkanPhysicalDevice.cpp` |
//! | `vulkan_logical_device.rs` | `VulkanLogicalDevice.cpp` |
//! | `vulkan_context.rs` | `VulkanContext.h` — the global bag, now a value |
//!
//! What is left here is the sequencing, plus the extension-name constants the
//! three of them share. The C++ file is 500 lines of VulkanDevice plus a
//! `Begin`/`End` frame that this engine keeps in `vulkan_frame_context.rs`,
//! because a frame context is a different thing from a device.

use std::ffi::CStr;

use ash::vk;

use crate::vulkan_error::Result;
use crate::vulkan_instance;
use crate::vulkan_logical_device;
use crate::vulkan_physical_device;

/// Instance and device creation options.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceOptions {
    /// Enables the validation layer when the loader offers it. Never fatal: a
    /// missing layer must not stop a release build from starting.
    pub validation: bool,
    /// Engine name reported to the loader, for driver-side logs.
    pub engine_name: &'static CStr,
}

impl DeviceOptions {
    pub fn new(engine_name: &'static CStr) -> Self {
        Self {
            validation: false,
            engine_name,
        }
    }

    pub fn with_validation(mut self, enabled: bool) -> Self {
        self.validation = enabled;
        self
    }

    /// Honours `UHE_VALIDATION=1`, mirroring how the C++ engine gates its
    /// sanitizer framework (`CMakeLists.txt:32-174`).
    ///
    /// Worth having on by default for tests: it found four real bugs in this
    /// backend that `cargo test` alone could not see, including a command pool
    /// shared across threads and a timeline value that did not increase.
    pub fn from_env(engine_name: &'static CStr) -> Self {
        let validation = std::env::var("UHE_VALIDATION")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self::new(engine_name).with_validation(validation)
    }
}

pub fn create_device(options: DeviceOptions) -> Result<crate::vulkan_context::Context> {
    let bundle = vulkan_instance::create_instance(options.engine_name, options.validation)?;
    let instance = &bundle.instance;

    let physical_devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|e| crate::vulkan_error::BackendError::vk("vkEnumeratePhysicalDevices", e))?;
    let physical_device = vulkan_physical_device::choose(instance, &physical_devices)?;

    let properties = unsafe { instance.get_physical_device_properties(physical_device) };

    // **Once**, here, and handed down.
    //
    // This used to be `vkEnumerateDeviceExtensionProperties` called once per
    // capability -- eight times during one device creation. The C++ engine
    // enumerates once into a `VulkanExtensionCheck` and answers named questions
    // off the cache, and the queries are core-promotion-aware there in a way the
    // string check here was not.
    let check = crate::vulkan_extension_check::ExtensionCheck::probe(
        instance,
        physical_device,
        properties.api_version,
    );

    let probe = vulkan_physical_device::probe_capabilities(
        instance,
        physical_device,
        properties.api_version,
        &check,
    );
    let families = vulkan_physical_device::queue_families(instance, physical_device)?;

    let logical = vulkan_logical_device::create(
        instance,
        physical_device,
        properties,
        probe,
        &families,
        &check,
    )?;

    Ok(crate::vulkan_context::Context::new(
        bundle.entry.clone(),
        bundle.instance.clone(),
        physical_device,
        logical.device,
        logical.graphics_queue,
        logical.capabilities,
        bundle.validation_enabled,
    ))
}

/// True when a Vulkan loader is present.
///
/// Lets the test suite skip rather than fail on a machine with no driver, which
/// is the same reason the workspace has `default-members`.
pub fn device_available() -> bool {
    unsafe { ash::Entry::load() }.is_ok()
}

/// The graphics queue family of the current device, or an error.
pub fn graphics_queue_family(context: &crate::vulkan_context::Context) -> Result<u32> {
    context.graphics_queue_family()
}

/// Convenience for callers that only want the format a swapchain would use.
///
/// Provisional: the real answer belongs to `vulkan_swapchain.rs`, which asks the
/// surface. This is here so a headless caller has something sensible.
pub fn preferred_swapchain_format(formats: &[vk::Format]) -> vk::Format {
    for candidate in [
        vk::Format::B8G8R8A8_SRGB,
        vk::Format::R8G8B8A8_SRGB,
        vk::Format::B8G8R8A8_UNORM,
        vk::Format::R8G8B8A8_UNORM,
    ] {
        if formats.contains(&candidate) {
            return candidate;
        }
    }
    formats.first().copied().unwrap_or(vk::Format::UNDEFINED)
}
