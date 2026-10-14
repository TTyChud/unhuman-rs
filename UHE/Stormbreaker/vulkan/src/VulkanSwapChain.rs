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

use ash::{vk, Instance};

use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_instance;

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
    /// Kept for creating and destroying image views. The loader below can make
    /// swapchain calls but not device ones.
    device: ash::Device,
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
    /// Index of the acquired image, and the image itself.
    ///
    /// Both, because they are needed for different things: present names the
    /// image by index, and the layout transition names it by handle. Keeping only
    /// the handle means `get_swapchain_images()[index]` on every present; keeping
    /// only the index means the transition has nothing to name.
    acquired_index: Option<u32>,
    acquired_image: Option<vk::Image>,
    /// The swapchain's images, read once at creation.
    ///
    /// `vkGetSwapchainImagesKHR` returns indices, not images, and the images
    /// themselves are stable for the lifetime of the swapchain. Asking again on
    /// every acquire is a driver round trip to learn the same array.
    images: Vec<vk::Image>,
    /// One view per swapchain image, made on first use.
    ///
    /// A swapchain owns its images, so the views over them belong to whoever
    /// made them -- this struct. They are destroyed in `Drop`, before the images
    /// go with the swapchain, and before the device, because this is a field of
    /// `VulkanGpu` declared above `context`.
    ///
    /// Made lazily rather than in `new` because most runs are headless and a
    /// window nobody has drawn to should not have allocated anything.
    image_views: Vec<vk::ImageView>,
}

impl Swapchain {
    /// Builds a swapchain against `surface`, **taking ownership of it**.
    ///
    /// ## Ownership
    ///
    /// The surface is destroyed in `Drop`, so a caller that also destroys it is
    /// double-freeing. That was ambiguous here: `Drop` destroyed it, the comment
    /// said "owned by someone else", and a test that believed the comment leaked
    /// a `VkSurfaceKHR` against `vkDestroyInstance`.
    ///
    /// On any error the surface is destroyed before returning. Otherwise a failed
    /// `new` -- a maximised window on a display that cannot do it, a format
    /// nobody supports -- leaks it, and the failure path is exactly where a leak
    /// hides.
    pub fn new_owning_surface(
        entry: &ash::Entry,
        instance: &Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        desired_extent: (u32, u32),
        prefer_vsync: bool,
    ) -> Result<Self> {
        match Self::new(
            entry,
            instance,
            device,
            physical_device,
            surface,
            desired_extent,
            prefer_vsync,
        ) {
            Ok(swapchain) => Ok(swapchain),
            Err(error) => {
                // Nothing was built, so `Drop` will not run, so the surface is
                // still ours to free.
                unsafe {
                    ash::khr::surface::Instance::new(entry, instance)
                        .destroy_surface(surface, None);
                }
                Err(error)
            }
        }
    }

    /// Builds a swapchain against a surface the caller keeps.
    ///
    /// Prefer [`Swapchain::new_owning_surface`]. This exists for the case where
    /// something else -- a window system — really does own the surface and
    /// outlives the swapchain.
    pub fn new(
        entry: &ash::Entry,
        instance: &Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        desired_extent: (u32, u32),
        prefer_vsync: bool,
    ) -> Result<Self> {
        let instance_loader = ash::khr::surface::Instance::new(entry, instance);
        let device_loader = ash::khr::swapchain::Device::new(instance, device);

        let capabilities = unsafe {
            instance_loader.get_physical_device_surface_capabilities(physical_device, surface)
        }
        .map_err(|e| BackendError::vk("vkGetPhysicalDeviceSurfaceCapabilitiesKHR", e))?;
        let formats = unsafe {
            instance_loader.get_physical_device_surface_formats(physical_device, surface)
        }
        .map_err(|e| BackendError::vk("vkGetPhysicalDeviceSurfaceFormatsKHR", e))?;
        let present_modes = unsafe {
            instance_loader.get_physical_device_surface_present_modes(physical_device, surface)
        }
        .map_err(|e| BackendError::vk("vkGetPhysicalDeviceSurfacePresentModesKHR", e))?;

        if formats.is_empty() {
            return Err(BackendError::EmptyUsage {
                what: "surface formats",
            });
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

        let extent = choose_extent(
            capabilities.current_extent,
            desired_extent,
            capabilities.min_image_extent,
            capabilities.max_image_extent,
        )?;

        let queue_family_indices = [0u32];
        let create_info = vk::SwapchainCreateInfoKHR {
            surface,
            min_image_count: image_count,
            image_format: chosen_format,
            image_color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            image_extent: vk::Extent2D {
                width: extent.0,
                height: extent.1,
            },
            image_array_layers: 1,
            image_usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST,
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

        let swapchain = unsafe { device_loader.create_swapchain(&create_info, None) }
            .map_err(|e| BackendError::vk("vkCreateSwapchainKHR", e))?;

        let images = unsafe { device_loader.get_swapchain_images(swapchain) }
            .map_err(|e| BackendError::vk("vkGetSwapchainImagesKHR", e))?;

        Ok(Self {
            instance_loader,
            device: device.clone(),
            device_loader,
            surface,
            swapchain,
            format: chosen_format,
            extent,
            image_count: images.len() as u32,
            present_mode,
            out_of_date: false,
            suboptimal: false,
            acquired_index: None,
            acquired_image: None,
            images,
            image_views: Vec::with_capacity(image_count as usize),
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

    /// One view per swapchain image, made on first use and kept.
    ///
    /// ## Why views at all
    ///
    /// `vkQueuePresentKHR` names an image; a render pass names a *view*. The
    /// bridging object has to exist somewhere, and the swapchain is the only
    /// thing that outlives both the images and the frame.
    ///
    /// ## Why they are cached
    ///
    /// They depend only on the image and the format, both fixed for the lifetime
    /// of a swapchain. Recreating one per frame is allocation in the frame loop
    /// to produce an identical handle, and it is a `vkCreateImageView` that can
    /// fail on the one frame that had something else wrong.
    ///
    /// ## Why lazily
    ///
    /// A swapchain that has never been acquired has nothing to draw into.
    pub fn image_views(&mut self) -> Result<&[vk::ImageView]> {
        if self.image_views.is_empty() {
            let images = self.swapchain_images()?;
            for image in &images {
                let info = vk::ImageViewCreateInfo::default()
                    .image(*image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(self.format)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    });
                let view = unsafe { self.device.create_image_view(&info, None) }
                    .map_err(|error| BackendError::vk("vkCreateImageView", error))?;
                self.image_views.push(view);
            }
        }
        Ok(&self.image_views)
    }

    /// The acquired image itself, if any.
    ///
    /// `acquire` hands back a `vk::Image`, and this is where it is kept. Present
    /// needs the image by name for the layout transition; the index alone is not
    /// enough, because the graph has never heard of it.
    pub fn acquired_image(&self) -> Option<vk::Image> {
        self.acquired_image
    }

    /// Which image is acquired, if any.
    pub fn acquired_image_index(&self) -> Option<u32> {
        self.acquired_index
    }

    /// Give the acquired image back without presenting it.
    ///
    /// There is no Vulkan call for this -- an acquired image is released by
    /// presenting it or by destroying the swapchain. So it only clears the local
    /// record, which is what "released" means to the frame loop: the next
    /// `acquire` is free to hand out a different image.
    ///
    /// The out-of-date path needs this. On `ERROR_OUT_OF_DATE_KHR` the driver has
    /// already invalidated the swapchain, and leaving a stale index set would make
    /// `present` aim at an image that no longer exists.
    pub fn release_acquired(&mut self) {
        self.acquired_index = None;
        self.acquired_image = None;
    }

    /// The view of the image currently acquired, if one is.
    ///
    /// `None` before `acquire` and after `present`, and a null handle is a
    /// worse answer than that: it names no image and a caller who used it would
    /// write into nothing, later, somewhere else.
    pub fn acquired_image_view(&mut self) -> Result<Option<vk::ImageView>> {
        let Some(index) = self.acquired_index else {
            return Ok(None);
        };
        let views = self.image_views()?;
        views.get(index as usize).copied().map(Some).ok_or_else(|| {
            BackendError::vk("vkAcquireNextImageKHR", vk::Result::ERROR_OUT_OF_DATE_KHR)
        })
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
                // `acquire_next_image` hands back an *index*, not an image. The
                // image is `get_swapchain_images()[index]`, and it has to be that
                // exact handle: `vkImage::from_raw(index)` looks plausible and is
                // not an object the driver knows, and the next barrier to name it
                // is `pDependencyInfo->pImageMemoryBarriers[0].image is
                // VK_NULL_HANDLE` or a segfault.
                let image = *self.images.get(index as usize).ok_or_else(|| {
                    BackendError::vk("vkAcquireNextImageKHR", vk::Result::ERROR_OUT_OF_DATE_KHR)
                })?;
                self.acquired_image = Some(image);
                Ok(image)
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
        // Views, then the swapchain, then the surface.
        //
        // The order is three constraints, not a preference: a view must not
        // outlive the image it looks at, the swapchain must not outlive the
        // surface it presents to, and `destroy_swapchain` must not run after the
        // device that owns it. `Swapchain` is a field of `VulkanGpu` declared
        // above `context`, so all three hold.
        unsafe {
            for view in self.image_views.drain(..) {
                self.device.destroy_image_view(view, None);
            }
            self.device_loader.destroy_swapchain(self.swapchain, None);
            self.instance_loader.destroy_surface(self.surface, None);
        }
    }
}

/// A surface with no window behind it, for tests and offscreen runs.
///
/// ## Why the extension is checked first
///
/// `ash::ext::headless_surface::Instance::new` **panics** when the extension was
/// not enabled at instance creation. It does not return an error and it does not
/// panic on a driver that lacks the extension *by name* -- it resolves entry
/// points and aborts if they are missing. So the check has to happen here, and it
/// has to be a `Result`, because a caller on a machine without the extension is
/// an ordinary situation and not a reason to kill the process.
/// The extent to build the swapchain at.
///
/// ## `current_extent == u32::MAX` is not an extent
///
/// `VkSurfaceCapabilitiesKHR::currentExtent` is `UINT32_MAX` to mean **"you
/// choose"**. It is how every headless surface answers, and it is not a sentinel
/// this function invented -- it is in the spec. Passing it straight to
/// `vkCreateSwapchainKHR` asks for a 4294967295-wide image, which validation
/// rejects with `VUID-VkSwapchainCreateInfoKHR-pNext-07781`.
///
/// ## Why it is clamped rather than trusted
///
/// `currentExtent` can also be *outside* `maxImageExtent` when a window is
/// larger than the display allows. The spec requires the swapchain extent to be
/// inside `minImageExtent..maxImageExtent` regardless, so even the reported value
/// is clamped. A window larger than the monitor should letterbox, not fail.
///
/// ## Zero
///
/// Zero means the window is minimised or has not been shown. There is nothing to
/// render into, and a zero-sized image is a driver error. So this is reported.
///
/// Not clamped up to `minImageExtent`. Clamping turns "the window is not there"
/// into a 1x1 swapchain that presents a 1x1 image, which is worse than an error:
/// it looks like it works. A `desired_extent` does not rescue it either -- the
/// caller wanting 640x480 says nothing about whether the window is showing.
///
/// Pure, so every case is tested with no device.
pub fn choose_extent(
    current: vk::Extent2D,
    desired: (u32, u32),
    min: vk::Extent2D,
    max: vk::Extent2D,
) -> Result<(u32, u32)> {
    const CHOOSE_YOURSELF: u32 = u32::MAX;

    let source = if current.width == CHOOSE_YOURSELF && current.height == CHOOSE_YOURSELF {
        vk::Extent2D {
            width: desired.0,
            height: desired.1,
        }
    } else {
        if current.width == 0 || current.height == 0 {
            return Err(BackendError::EmptyUsage {
                what: "surface extent",
            });
        }
        current
    };

    let clamp = |value: u32, low: u32, high: u32| value.clamp(low, high);
    let width = clamp(source.width, min.width, max.width);
    let height = clamp(source.height, min.height, max.height);

    if width == 0 || height == 0 {
        return Err(BackendError::EmptyUsage {
            what: "surface extent",
        });
    }
    Ok((width, height))
}

/// Destroy a surface this backend did not create.
///
/// `vkDestroySurfaceKHR` lives on the surface loader, not on `Instance`, so a
/// caller who made a surface has to build a loader to get rid of it. That is two
/// lines of ceremony at every call site and one place to get the cast wrong, so
/// it is a function here.
///
/// **Order matters.** The swapchain must be destroyed before the surface. A
/// `Swapchain` borrows its surface rather than owning it, so dropping the
/// `Swapchain` before calling this is what makes it safe.
pub fn destroy_surface(entry: &ash::Entry, instance: &Instance, surface: vk::SurfaceKHR) {
    let loader = ash::khr::surface::Instance::new(entry, instance);
    unsafe { loader.destroy_surface(surface, None) };
}

pub fn create_headless_surface(entry: &ash::Entry, instance: &Instance) -> Result<vk::SurfaceKHR> {
    if !vulkan_instance::instance_extension_available(entry, c"VK_EXT_headless_surface")
        && !vulkan_instance::instance_extension_available(entry, c"VK_KHR_headless_surface")
    {
        return Err(BackendError::ExtensionUnavailable {
            name: "VK_EXT_headless_surface".to_owned(),
            required_for: "a surface with no window",
        });
    }
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
        .ok_or(BackendError::EmptyUsage {
            what: "surface formats",
        })
}

/// The `chassis` format for a chosen Vulkan surface format, so a caller does not
/// have to hold both enums.
/// The swapchain's format as a `chassis` one.
///
/// A re-export, not a second mapping. There were two: this one and
/// `VulkanTypes::from_vk_format`, each with its own subset of formats, and they
/// disagreed -- a `D32_SFLOAT` swapchain reported a depth format here and `None`
/// there. Three copies is how a swapchain ends up one format away from the
/// pipeline's.
pub use crate::vulkan_types::from_vk_format as chassis_format;

#[cfg(test)]
mod tests {
    use super::*;
    use chassis::TextureFormat;

    /// `currentExtent == UINT32_MAX` means "you choose", not "4294967295 wide".
    ///
    /// It is what every headless surface answers, so it is the common case here,
    /// not an edge case. The old code passed it through and validation rejected
    /// the swapchain.
    #[test]
    fn the_you_choose_sentinel_is_not_an_extent() {
        let choose_yourself = vk::Extent2D {
            width: u32::MAX,
            height: u32::MAX,
        };
        let limits = (
            vk::Extent2D {
                width: 1,
                height: 1,
            },
            vk::Extent2D {
                width: 16384,
                height: 16384,
            },
        );
        assert_eq!(
            choose_extent(choose_yourself, (800, 600), limits.0, limits.1).unwrap(),
            (800, 600),
            "the caller picks",
        );
        assert_eq!(
            choose_extent(choose_yourself, (99999, 99999), limits.0, limits.1).unwrap(),
            (16384, 16384),
            "and still inside the maximum, which the spec requires",
        );
    }

    #[test]
    fn a_window_extent_is_clamped_rather_than_trusted() {
        let window = vk::Extent2D {
            width: 30000,
            height: 30000,
        };
        let min = vk::Extent2D {
            width: 1,
            height: 1,
        };
        let max = vk::Extent2D {
            width: 16384,
            height: 16384,
        };
        assert_eq!(
            choose_extent(window, (0, 0), min, max).unwrap(),
            (16384, 16384),
            "a window larger than the display letterboxes instead of failing",
        );
    }

    #[test]
    fn a_minimised_window_is_an_error_rather_than_a_zero_image() {
        let zero = vk::Extent2D {
            width: 0,
            height: 0,
        };
        let min = vk::Extent2D {
            width: 1,
            height: 1,
        };
        let max = vk::Extent2D {
            width: 16384,
            height: 16384,
        };
        assert!(
            choose_extent(zero, (0, 0), min, max).is_err(),
            "nothing to render into is reported, not created",
        );
        // Not even a wanted extent rescues it, and not a clamp up to the
        // minimum. Both turn "the window is not there" into a swapchain that
        // presents something.
        assert!(
            choose_extent(zero, (640, 480), min, max).is_err(),
            "wanting an extent does not make a minimised window presentable",
        );
    }

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
        assert_eq!(
            choose_format(&[only]).unwrap(),
            vk::Format::R16G16B16A16_SFLOAT
        );
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
        // Was `None`, because `chassis_format` and `VulkanTypes::from_vk_format`
        // were two mappings with different coverage and this one was the shorter.
        // Consolidated onto the longer one, so a surface asking for an RGBA32F
        // image now resolves instead of failing.
        assert_eq!(
            chassis_format(vk::Format::R32G32B32A32_SFLOAT),
            Some(TextureFormat::Rgba32Float)
        );
        assert_eq!(
            chassis_format(vk::Format::R16G16B16A16_SFLOAT),
            Some(TextureFormat::Rgba16Float)
        );
        // Something genuinely unmapped still says so rather than guessing.
        assert_eq!(chassis_format(vk::Format::R64G64B64A64_SFLOAT), None);
    }
}
