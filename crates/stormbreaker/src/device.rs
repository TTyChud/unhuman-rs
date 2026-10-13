//! Instance and logical device creation.
//!
//! Three things here are easy to get subtly wrong and are worth stating:
//!
//! * The instance asks for Vulkan **1.1**, not 1.3. Asking for a version the
//!   loader lacks rejects an otherwise usable device, and the sync tier is
//!   decided from what the device reports, not from what we requested.
//! * Feature bits live in version-specific structs, and a `pNext` node from a
//!   version the device does not report is a validation error at best.
//!   `timeline_semaphore` and `descriptor_indexing` are 1.2 features;
//!   `synchronization2` and `dynamic_rendering` are only reachable through the
//!   1.3 struct in this ash version. So the chain is `Features2 -> V13 -> V12`
//!   and only ever on a device reporting 1.3 or newer.
//! * Below 1.3 the same capabilities are detected by enumerating device
//!   extensions instead, which is what the extension-based world looked like
//!   before those features were promoted.

use std::ffi::CStr;
use std::os::raw::c_void;

use ash::{vk, Entry, Instance};

use crate::context::{choose_physical_device, probe_queue_families, Context};
use crate::error::{BackendError, Result};
use crate::extension_check::{select_sync_tier, version_major, version_minor, Capabilities};

pub const VALIDATION_LAYER: &str = "VK_LAYER_KHRONOS_validation";
pub const TIMELINE_SEMAPHORE: &str = "VK_KHR_timeline_semaphore";
pub const SYNC_2: &str = "VK_KHR_synchronization2";
pub const DYNAMIC_RENDERING: &str = "VK_KHR_dynamic_rendering";
pub const MEMORY_BUDGET: &str = "VK_EXT_memory_budget";
pub const DESCRIPTOR_INDEXING: &str = "VK_EXT_descriptor_indexing";

/// Lowest instance API version we ask for. 1.1 unlocks
/// `vkGetPhysicalDeviceFeatures2`, the only sane way to probe feature chains,
/// and is available essentially everywhere.
///
/// Spelled with `make_api_version` rather than as `(1 << 22) | (1 << 12)`: the
/// hand-encoded form was written as `(0 << 22) | (1 << 12)`, which is version
/// **0.1**, and every instance creation failed with `ERROR_INCOMPATIBLE_DRIVER`.
/// `requested_api_version_is_valid` below keeps that from coming back.
pub fn requested_api_version() -> u32 {
    vk::make_api_version(0, 1, 1, 0)
}

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
    pub fn from_env(engine_name: &'static CStr) -> Self {
        let validation = std::env::var("UHE_VALIDATION")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self::new(engine_name).with_validation(validation)
    }
}

/// What the device reports it can do, before anything is enabled.
#[derive(Clone, Copy, Debug, Default)]
struct Probe {
    synchronization2: bool,
    timeline_semaphore: bool,
    dynamic_rendering: bool,
    descriptor_indexing: bool,
}

fn at_least(api_version: u32, major: u32, minor: u32) -> bool {
    let (vmaj, vmin) = (version_major(api_version), version_minor(api_version));
    vmaj > major || (vmaj == major && vmin >= minor)
}

pub fn create_device(options: DeviceOptions) -> Result<Context> {
    let entry = unsafe { Entry::load() }.map_err(|e| BackendError::Loader(e.to_string()))?;

    let validation_enabled = options.validation && layer_available(&entry, VALIDATION_LAYER);
    let layer_ptrs: Vec<*const std::os::raw::c_char> = if validation_enabled {
        vec![c"VK_LAYER_KHRONOS_validation".as_ptr()]
    } else {
        Vec::new()
    };

    // No surface extensions yet: `eyes` owns window creation. Requesting
    // `VK_KHR_surface` with no surface is the sort of thing that validates on
    // one loader and not another.
    let app_info = vk::ApplicationInfo {
        p_application_name: c"unhuman-rs".as_ptr(),
        application_version: vk::make_api_version(0, 1, 0, 0),
        p_engine_name: options.engine_name.as_ptr(),
        engine_version: vk::make_api_version(0, 1, 0, 0),
        api_version: requested_api_version(),
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

    let physical_devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|e| BackendError::vk("vkEnumeratePhysicalDevices", e))?;
    let physical_device = choose_physical_device(&instance, &physical_devices)?;

    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let api_version = properties.api_version;

    let probe = probe_capabilities(&instance, physical_device, api_version)?;
    let sync_tier = select_sync_tier(api_version, probe.synchronization2, probe.timeline_semaphore);

    let families = probe_queue_families(&instance, physical_device)?;
    let graphics_family = families
        .iter()
        .find(|f| f.graphics)
        .map(|f| f.index)
        .ok_or(BackendError::NoGraphicsQueue)?;

    // The extension list mirrors what the tier will actually use, so a legacy
    // path never carries an unused extension.
    let mut extension_ptrs: Vec<*const std::os::raw::c_char> = Vec::new();
    if sync_tier.supports_timeline_semaphores() {
        if !has_device_extension(&instance, physical_device, TIMELINE_SEMAPHORE) {
            return Err(BackendError::ExtensionUnavailable {
                name: TIMELINE_SEMAPHORE.into(),
                required_for: "Sync2Timeline",
            });
        }
        extension_ptrs.push(c"VK_KHR_timeline_semaphore".as_ptr());
    }

    // Both arrays must outlive the create calls, so they are bindings rather
    // than temporaries.
    let queue_priorities = [1.0f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(graphics_family)
        .queue_priorities(&queue_priorities);

    // Enable chain, same ordering rule as the probe. These locals must outlive
    // `vkCreateDevice`; the driver copies the values during the call.
    let mut enable_v12 = vk::PhysicalDeviceVulkan12Features::default()
        .timeline_semaphore(sync_tier.supports_timeline_semaphores())
        .descriptor_indexing(probe.descriptor_indexing);
    let mut enable_v13 = vk::PhysicalDeviceVulkan13Features::default()
        .synchronization2(sync_tier.supports_sync2())
        .dynamic_rendering(probe.dynamic_rendering);

    let queue_infos = [queue_info];
    let base_features = vk::PhysicalDeviceFeatures::default();
    let mut device_info = vk::DeviceCreateInfo {
        p_enabled_features: &base_features,
        ..Default::default()
    }
    .queue_create_infos(&queue_infos)
    .enabled_extension_names(&extension_ptrs);

    if at_least(api_version, 1, 3) {
        enable_v13.p_next = (&raw mut enable_v12).cast::<c_void>();
        device_info.p_next = (&raw mut enable_v13).cast::<c_void>();
    }

    let device = unsafe { instance.create_device(physical_device, &device_info, None) }
        .map_err(|e| BackendError::vk("vkCreateDevice", e))?;
    let graphics_queue = unsafe { device.get_device_queue(graphics_family, 0) };

    let capabilities = Capabilities {
        device_name: read_name(&properties.device_name),
        api_version,
        driver_version: properties.driver_version,
        vendor_id: properties.vendor_id,
        device_id: properties.device_id,
        sync_tier,
        queue_families: families,
        graphics_queue_family: Some(graphics_family),
        dynamic_rendering: probe.dynamic_rendering,
        memory_budget: has_device_extension(&instance, physical_device, MEMORY_BUDGET),
        descriptor_indexing: probe.descriptor_indexing,
        max_texture_dimension_2d: properties.limits.max_image_dimension2_d,
    };

    Ok(Context::new(
        entry,
        instance,
        physical_device,
        device,
        graphics_queue,
        capabilities,
        validation_enabled,
    ))
}

/// Reads capability bits, falling back to extension enumeration below 1.3.
fn probe_capabilities(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    api_version: u32,
) -> Result<Probe> {
    if !at_least(api_version, 1, 3) {
        return Ok(Probe {
            synchronization2: has_device_extension(instance, physical_device, SYNC_2),
            timeline_semaphore: has_device_extension(instance, physical_device, TIMELINE_SEMAPHORE),
            dynamic_rendering: has_device_extension(instance, physical_device, DYNAMIC_RENDERING),
            descriptor_indexing: has_device_extension(instance, physical_device, DESCRIPTOR_INDEXING),
        });
    }

    // The chain nodes must outlive the query they are handed to.
    let mut v12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut v13 = vk::PhysicalDeviceVulkan13Features::default();
    let mut features2 = vk::PhysicalDeviceFeatures2::default();
    unsafe {
        v13.p_next = (&raw mut v12).cast::<c_void>();
        features2.p_next = (&raw mut v13).cast::<c_void>();
        instance.get_physical_device_features2(physical_device, &mut features2);
    }

    Ok(Probe {
        synchronization2: v13.synchronization2 == vk::TRUE,
        timeline_semaphore: v12.timeline_semaphore == vk::TRUE,
        dynamic_rendering: v13.dynamic_rendering == vk::TRUE,
        descriptor_indexing: v12.descriptor_indexing == vk::TRUE,
    })
}

fn layer_available(entry: &Entry, name: &str) -> bool {
    unsafe { entry.enumerate_instance_layer_properties() }
        .map(|layers| layers.iter().any(|l| name_of(l.layer_name.as_ptr()) == name))
        .unwrap_or(false)
}

fn has_device_extension(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    name: &str,
) -> bool {
    unsafe { instance.enumerate_device_extension_properties(physical_device) }
        .map(|extensions| {
            extensions
                .iter()
                .any(|e| name_of(e.extension_name.as_ptr()) == name)
        })
        .unwrap_or(false)
}

fn name_of(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Vulkan stores the device name as a fixed NUL-padded `[c_char; 256]`.
fn read_name(chars: &[i8; 256]) -> String {
    chars
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8 as char)
        .collect()
}

/// True when a Vulkan loader is present.
///
/// Lets the test suite skip rather than fail on a machine with no driver, which
/// is the same reason the workspace has `default-members`.
pub fn device_available() -> bool {
    unsafe { Entry::load() }.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_api_version_is_valid() {
        let api = requested_api_version();
        assert_eq!(version_major(api), 1, "major must be 1, not 0");
        assert_eq!(version_minor(api), 1, "minor must be 1");
        // 0.1 is the value that broke instance creation, and it is easy to
        // hand-encode wrong.
        assert!(api > vk::make_api_version(0, 1, 0, 0));
    }

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
