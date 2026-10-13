//! Debug scopes.
//!
//! Mirrors the labelling half of `VulkanImGuiBackend.cpp` / the Tracy zones in
//! `UHE/src/UHE/Debug/TracySetup.h`.
//!
//! Everything here is gated on `VK_EXT_debug_utils` being enabled on the instance
//! and on the validation layer or a capture tool being attached. Without those,
//! every function is a no-op, because a debug name on a release build costs a
//! driver call to store a string nobody will read.
//!
//! The C++ engine keeps `TracySetup.h` and profiling macros wired separately from
//! Vulkan. Here the scopes are Vulkan objects, because that is what they name, and
//! because `VK_EXT_debug_utils` labels are the ones that survive into RenderDoc.

use ash::vk;

/// What the backend knows about the loader's debug support.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ScopesConfig {
    /// `VK_EXT_debug_utils` was enabled at instance creation.
    pub extension_enabled: bool,
    /// A validation layer or capture tool is attached, so labels will be read.
    pub consumer_attached: bool,
}

impl ScopesConfig {
    pub fn enabled(&self) -> bool {
        self.extension_enabled && self.consumer_attached
    }
}

/// Holds the extension loader. Cheap to construct, no-op when disabled.
pub struct Scopes {
    loader: Option<ash::ext::debug_utils::Device>,
    config: ScopesConfig,
    /// Count of scopes opened. A renderer that opens none is probably not
    /// instrumented at all, which is worth seeing.
    opened: u64,
}

impl Scopes {
    pub fn new(instance: &ash::Instance, device: &ash::Device, config: ScopesConfig) -> Self {
        // In this ash version debug utils is core-promoted, so there is no
        // separate loader: it is a method set on the ordinary Device.
        let loader = config
            .extension_enabled
            .then(|| ash::ext::debug_utils::Device::new(instance, device));
        Self {
            loader,
            config,
            opened: 0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.loader.is_some() && self.config.enabled()
    }

    pub fn opened(&self) -> u64 {
        self.opened
    }

    /// Names an object. Cheap enough on a debug build, and the only thing that
    /// makes a capture readable.
    pub fn name_object<T: ash::vk::Handle>(
        &mut self,
        handle: T,
        name: &str,
        object_type: vk::ObjectType,
    ) {
        if !self.enabled() || name.is_empty() {
            return;
        }
        let info = vk::DebugUtilsObjectNameInfoEXT {
            object_type,
            object_handle: handle.as_raw(),
            // Debug labels live only until the call returns, so the string is
            // borrowed rather than leaked.
            p_object_name: crate::vulkan_utils::name_ptr(name),
            ..Default::default()
        };
        let Some(loader) = &self.loader else {
            return;
        };
        unsafe {
            // A failure here means the extension went away under us, which is not
            // worth propagating: a missing debug label is not a frame error.
            let _ = loader.set_debug_utils_object_name(&info);
        }
        self.opened += 1;
    }

    /// A labelled region on a command buffer, opened and closed.
    pub fn scope<'a>(
        &'a mut self,
        command_buffer: vk::CommandBuffer,
        label: &'a str,
    ) -> Option<DebugScope<'a>> {
        if !self.enabled() || label.is_empty() {
            return None;
        }
        self.opened += 1;
        Some(DebugScope {
            loader: self.loader.as_ref(),
            command_buffer,
            label,
        })
    }
}

/// Opens a labelled region and closes it on drop, so an early return cannot leave
/// a scope dangling.
pub struct DebugScope<'a> {
    loader: Option<&'a ash::ext::debug_utils::Device>,
    command_buffer: vk::CommandBuffer,
    label: &'a str,
}

impl Drop for DebugScope<'_> {
    fn drop(&mut self) {
        let Some(loader) = self.loader else { return };
        unsafe {
            loader.cmd_end_debug_utils_label(self.command_buffer);
        }
        let _ = self.label;
    }
}