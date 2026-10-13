//! Swapchain.
//!
//! Mirrors `VulkanSwapChain.cpp`.
//!
//! Takes an existing `VkSurfaceKHR` rather than creating one. That matters
//! because ash has no generic `vkCreateSurface`: surface creation is
//! platform-specific (wayland, xlib, win32, android) and belongs to `eyes`, the
//! windowing crate. What is here is the part that is the same either way.
//!
//! For tests there is `create_headless_surface`, using `VK_EXT_headless_surface`,
//! which the driver reports on any machine with a real ICD. That means the
//! swapchain path — present modes, format selection, out-of-date handling,
//! acquire and present — is testable with no window at all, which is also what
//! the headless robotics mode needs.

use ash::vk::Handle;
use ash::{vk, Instance};
use chassis::TextureFormat;

use crate::vulkan_error::{BackendError, Result};

/// What the surface says it can do.
#[derive(Clone, Debug)]
pub struct SurfaceCapabilities {
    pub min_image_count: u32,
    pub current_extent: Option<(u32, u32)>,
    pub max_extent: (u32, u32),
    pub present_modes: Vec<vk::PresentModeKHR>,
    pub formats: Vec<vk::SurfaceFormatKHR>,
}

pub struct Swapchain {
    instance_loader: ash::khr::surface::Instance,
    device_loader: ash::khr::swapchain::Device,
    surface: vk::SurfaceKHR,
    swapchain: vk::SwapchainKHR,
    format: vk::Format,
    extent: (u32, u32),
    image_count: u32,
    present_mode: vk::PresentModeKHR,
    /// Set when the surface reported `VK_ERROR_OUT_OF_DATE_KHR` on acquire. The
    /// frame loop recreates on the next `frame_begin` and skips the frame.
    out_of_date: bool,
    /// The surface still works but no longer matches the swapchain exactly.
    suboptimal: bool,
    /// Index of the acquired image. Present names an image by index, and the
    /// handle is only ever used to make the view.
    acquired_index: Option<u32>,
}

impl Swapchain {
    /// Builds against a surface owned by someone else — `eyes` for a window.
    pub fn new(
        entry: &ash::Entry,
        instance: &Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        prefer_vsync: bool,
    ) -> Result<Self> {
        let instance_loader = ash::khr::surface::Instance::new(entry, instance);
        let device_loader = ash::khr::swapchain::Device::new(instance, device);

        let capabilities = unsafe { instance_loader.get_physical_device_surface_capabilities(physical_device, surface) }
            .map_err(|e| BackendError::vk("vkGetPhysicalDeviceSurfaceCapabilitiesKHR", e))?;
        let formats = unsafe { instance_loader.get_physical_device_surface_formats(physical_device, surface) }
            .map_err(|e| BackendError::vk("vkGetPhysicalDeviceSurfaceFormatsKHR", e))?;
        let present_modes =
            unsafe { instance_loader.get_physical_device_surface_present_modes(physical_device, surface) }
                .map_err(|e| BackendError::vk("vkGetPhysicalDeviceSurfacePresentModesKHR", e))?;

        if formats.is_empty() {
            return Err(BackendError::EmptyUsage { what: "surface formats" });
        }

        // Prefer 8-bit sRGB for a swapchain unless the surface offers none.
        let chosen_format = choose_format(&formats)?;

        // FIFO is guaranteed by the spec; anything else is a preference, and a
        // machine that only offers FIFO still gets a swapchain.
        let present_mode = if prefer_vsync {
            vk::PresentModeKHR::FIFO
        } else {
            [
                vk::PresentModeKHR::MAILBOX,
                vk::PresentModeKHR::IMMEDIATE,
                vk::PresentModeKHR::FIFO,
            ]
            .into_iter()
            .find(|m| present_modes.contains(m))
            .unwrap_or(vk::PresentModeKHR::FIFO)
        };

        // One more than the minimum: lets the CPU record the next frame while the
        // GPU is still on this one, which is the whole point of having images in
        // flight at all.
        let image_count = (capabilities.min_image_count + 1).max(2);

        // A zero current extent means the window is minimised or has not been
        // shown yet. There is nothing to render into, and a zero-sized image is
        // a driver error, so this is reported rather than created.
        let extent = capabilities.current_extent;
        if extent.width == 0 || extent.height == 0 {
            return Err(BackendError::EmptyUsage { what: "surface extent" });
        }

        let queue_family_indices = [0u32];
        let create_info = vk::SwapchainCreateInfoKHR {
            surface,
            min_image_count: image_count,
            image_format: chosen_format,
            image_color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            image_extent: vk::Extent2D {
                width: extent.width,
                height: extent.height,
            },
            image_array_layers: 1,
            image_usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_DST,
            image_sharing_mode: vk::SharingMode::EXCLUSIVE,
            queue_family_index_count: queue_family_indices.len() as u32,
            p_queue_family_indices: queue_family_indices.as_ptr(),
            pre_transform: vk::SurfaceTransformFlagsKHR::IDENTITY,
            composite_alpha: vk::CompositeAlphaFlagsKHR::OPAQUE,
            present_mode,
            // ash models Vulkan 1.0 Bool32 as u32.
            clipped: vk::TRUE,
            old_swapchain: vk::SwapchainKHR::null(),
            ..Default::default()
        };

        let swapchain =
            unsafe { device_loader.create_swapchain(&create_info, None) }
                .map_err(|e| BackendError::vk("vkCreateSwapchainKHR", e))?;

        let images = unsafe { device_loader.get_swapchain_images(swapchain) }
            .map_err(|e| BackendError::vk("vkGetSwapchainImagesKHR", e))?;

        Ok(Self {
            instance_loader,
            device_loader,
            surface,
            swapchain,
            format: chosen_format,
            extent: (extent.width, extent.height),
            image_count: images.len() as u32,
            present_mode,
            out_of_date: false,
            suboptimal: false,
            acquired_index: None,
        })
    }

    pub fn format(&self) -> vk::Format {
        self.format
    }

    pub fn extent(&self) -> (u32, u32) {
        self.extent
    }

    pub fn image_count(&self) -> u32 {
        self.image_count
    }

    pub fn present_mode(&self) -> vk::PresentModeKHR {
        self.present_mode
    }

    pub fn handle(&self) -> vk::SwapchainKHR {
        self.swapchain
    }

    pub fn surface(&self) -> vk::SurfaceKHR {
        self.surface
    }

    pub fn swapchain_images(&self) -> Result<Vec<vk::Image>> {
        unsafe { self.device_loader.get_swapchain_images(self.swapchain) }
            .map_err(|e| BackendError::vk("vkGetSwapchainImagesKHR", e))
    }

    pub fn surface_capabilities(
        &self,
        physical_device: vk::PhysicalDevice,
    ) -> Result<SurfaceCapabilities> {
        let caps = unsafe {
            self.instance_loader
                .get_physical_device_surface_capabilities(physical_device, self.surface)
        }
        .map_err(|e| BackendError::vk("vkGetPhysicalDeviceSurfaceCapabilitiesKHR", e))?;
        let formats = unsafe {
            self.instance_loader
                .get_physical_device_surface_formats(physical_device, self.surface)
        }
        .unwrap_or_default();
        let present_modes = unsafe {
            self.instance_loader
                .get_physical_device_surface_present_modes(physical_device, self.surface)
        }
        .unwrap_or_default();

        Ok(SurfaceCapabilities {
            min_image_count: caps.min_image_count,
            current_extent: if caps.current_extent.width == 0 || caps.current_extent.height == 0 {
                None
            } else {
                Some((caps.current_extent.width, caps.current_extent.height))
            },
            max_extent: (caps.max_image_extent.width, caps.max_image_extent.height),
            present_modes,
            formats,
        })
    }

    /// True when the surface has no current size — minimised, or not yet shown.
    pub fn is_minimised(&self) -> bool {
        self.extent.0 == 0 || self.extent.1 == 0
    }

    /// Takes the next image. `None` means out of date, and the caller should
    /// skip the frame and recreate.
    pub fn acquire(
        &mut self,
        timeout: std::time::Duration,
        fence: vk::Fence,
        semaphore: vk::Semaphore,
    ) -> Result<vk::Image> {
        let result = unsafe {
            self.device_loader.acquire_next_image(
                self.swapchain,
                u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX),
                semaphore,
                fence,
            )
        };

        match result {
            // ash returns `(image, suboptimal)`. Suboptimal is not out of date:
            // it means the surface is still usable but no longer the best match,
            // so the frame is drawn and the swapchain is recreated afterwards.
            Ok((index, suboptimal)) => {
                self.suboptimal = suboptimal;
                self.out_of_date = false;
                self.acquired_index = Some(index);
                Ok(vk::Image::from_raw(index as u64))
            }
            // Out of date means the window resized or the surface changed. Not an
            // error: the frame is skipped and the swapchain recreated.
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.out_of_date = true;
                Err(BackendError::SurfaceOutOfDate)
            }
            Err(e) => Err(BackendError::vk("vkAcquireNextImageKHR", e)),
        }
    }

    /// True when the last acquire reported the surface as out of date. The frame
    /// is skipped and the swapchain recreated.
    pub fn is_out_of_date(&self) -> bool {
        self.out_of_date
    }

    /// True when the surface is still usable but no longer an exact match, which
    /// is a signal to recreate after this frame rather than before it.
    pub fn is_suboptimal(&self) -> bool {
        self.suboptimal
    }

    /// Presents the acquired image. No fence parameter: `vkQueuePresentKHR` has
    /// none, and the caller waits on the frame's own fence separately.
    pub fn present(&mut self, queue: vk::Queue, wait: &[vk::Semaphore]) -> Result<()> {
        let semaphores = [self.swapchain];
        let index = self.acquired_index.unwrap_or_default();
        let info = vk::PresentInfoKHR {
            wait_semaphore_count: wait.len() as u32,
            p_wait_semaphores: wait.as_ptr(),
            swapchain_count: 1,
            p_swapchains: semaphores.as_ptr(),
            p_image_indices: &index,
            ..Default::default()
        };

        match unsafe { self.device_loader.queue_present(queue, &info) } {
            Ok(_) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.out_of_date = true;
                Err(BackendError::SurfaceOutOfDate)
            }
            Err(e) => Err(BackendError::vk("vkQueuePresentKHR", e)),
        }
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        // Device first, then the surface, which the swapchain references.
        unsafe {

            self.device_loader.destroy_swapchain(self.swapchain, None);
        }
        unsafe { self.instance_loader.destroy_surface(self.surface, None) };
    }
}

/// A surface with no window behind it, for headless rendering and tests.
///
/// `VK_EXT_headless_surface` is what makes the swapchain path testable on a
/// machine with no display.
pub fn create_headless_surface(entry: &ash::Entry, instance: &Instance) -> Result<vk::SurfaceKHR> {
    let loader = ash::ext::headless_surface::Instance::new(entry, instance);
    let info = vk::HeadlessSurfaceCreateInfoEXT::default();
    unsafe { loader.create_headless_surface(&info, None) }
        .map_err(|e| BackendError::vk("vkCreateHeadlessSurfaceEXT", e))
}

fn choose_format(formats: &[vk::SurfaceFormatKHR]) -> Result<vk::Format> {
    for candidate in [
        vk::Format::B8G8R8A8_SRGB,
        vk::Format::R8G8B8A8_SRGB,
        vk::Format::B8G8R8A8_UNORM,
        vk::Format::R8G8B8A8_UNORM,
    ] {
        if formats.iter().any(|f| f.format == candidate) {
            return Ok(candidate);
        }
    }
    formats
        .first()
        .map(|f| f.format)
        .ok_or(BackendError::EmptyUsage { what: "surface formats" })
}

/// The `chassis` format for a chosen Vulkan surface format, so a caller does not
/// have to hold both enums.
pub fn chassis_format(vk_format: vk::Format) -> Option<TextureFormat> {
    Some(match vk_format {
        vk::Format::R8G8B8A8_SRGB => TextureFormat::Rgba8Srgb,
        vk::Format::B8G8R8A8_SRGB => TextureFormat::Bgra8Srgb,
        vk::Format::R8G8B8A8_UNORM => TextureFormat::Rgba8Unorm,
        vk::Format::B8G8R8A8_UNORM => TextureFormat::Bgra8Unorm,
        vk::Format::R8_UNORM => TextureFormat::R8Unorm,
        vk::Format::D32_SFLOAT => TextureFormat::Depth32Float,
        vk::Format::D24_UNORM_S8_UINT => TextureFormat::Depth24Stencil8,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_srgb_format_is_preferred_over_a_linear_one() {
        let formats = [
            vk::SurfaceFormatKHR {
                format: vk::Format::R8G8B8A8_UNORM,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
            vk::SurfaceFormatKHR {
                format: vk::Format::B8G8R8A8_SRGB,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
        ];
        assert_eq!(choose_format(&formats).unwrap(), vk::Format::B8G8R8A8_SRGB);
    }

    #[test]
    fn a_formats_slice_with_no_preference_still_yields_something() {
        let only = vk::SurfaceFormatKHR {
            format: vk::Format::R16G16B16A16_SFLOAT,
            color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        };
        assert_eq!(choose_format(&[only]).unwrap(), vk::Format::R16G16B16A16_SFLOAT);
    }

    #[test]
    fn no_formats_is_an_error() {
        assert!(choose_format(&[]).is_err());
    }

    #[test]
    fn surface_formats_map_back_to_chassis() {
        assert_eq!(
            chassis_format(vk::Format::B8G8R8A8_SRGB),
            Some(TextureFormat::Bgra8Srgb)
        );
        assert_eq!(chassis_format(vk::Format::R32G32B32A32_SFLOAT), None);
    }
}