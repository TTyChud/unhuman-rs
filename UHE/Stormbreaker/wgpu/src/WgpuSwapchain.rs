//! Swapchain negotiation.
//!
//! ## Why this file is not the swapchain object
//!
//! In C++ it is: `VulkanSwapChain.cpp` creates a `vk::SwapchainKHR`, keeps its
//! images and views, and implements four `choose*` functions by hand. wgpu owns the
//! swapchain itself -- `Surface::configure` builds it and the images belong to the
//! driver -- so what is left for a file named after that one is the *negotiation*:
//! which format, which present mode, what size, how many images, and what an
//! out-of-date acquire means. [`WgpuSurface`](crate::wgpu_surface::WgpuSurface) is
//! the object.
//!
//! Everything here is a pure function of what the surface reported, so every case is
//! testable with no device at all -- which is the only way the "you choose" rule and
//! the out-of-date path can be tested on a machine with no display.

use std::fmt;

use chassis::{Extent2d, TextureFormat};

use crate::wgpu_device::to_wgpu_format;
use crate::wgpu_error::WgpuError;

/// The C++ `chooseSwapMinImageCount`: three images.
///
/// Three is what makes the CPU never wait on the image the GPU is drawing -- one
/// image means it waits every frame, two means it waits whenever the frame is late.
/// wgpu states the same thing as a latency: "the number of frames in the swapchain
/// is `desired_maximum_frame_latency + 1`, clamped to the supported range", and the
/// clamp is the `minImageCount`/`maxImageCount` half the C++ does by hand.
pub const IMAGE_COUNT: u32 = 3;

/// The latency that asks wgpu for `images` swapchain images.
pub fn latency_for(images: u32) -> u32 {
    images.saturating_sub(1)
}

/// The images a latency asks for: [`latency_for`]'s inverse, and what
/// `Swapchain::image_count` reports.
pub fn images_for(latency: u32) -> u32 {
    latency + 1
}

/// The C++ `chooseSwapSurfaceFormat`: sRGB BGRA when the surface offers it, the
/// surface's own first format otherwise.
///
/// A *requested* format the surface does not offer is an error rather than a
/// substitution. `Surface::configure` **panics** on an unsupported format, so the
/// check has to happen somewhere, and an image in a format nobody asked for is a
/// colour bug that reads as a shader bug.
pub fn choose_format(
    formats: &[wgpu::TextureFormat],
    requested: Option<TextureFormat>,
) -> Result<wgpu::TextureFormat, WgpuError> {
    if let Some(requested) = requested {
        if let Some(wanted) = to_wgpu_format(requested).filter(|wanted| formats.contains(wanted)) {
            return Ok(wanted);
        }
        return Err(WgpuError::Unsupported {
            what: format!("a swapchain in {requested:?}"),
            available: list(formats),
        });
    }
    if formats.contains(&wgpu::TextureFormat::Bgra8UnormSrgb) {
        return Ok(wgpu::TextureFormat::Bgra8UnormSrgb);
    }
    formats.first().copied().ok_or(WgpuError::IncompatibleSurface)
}

/// The C++ `chooseSwapPresentMode`: mailbox when offered, fifo otherwise.
///
/// Fifo is the one mode both APIs guarantee, and the one that cannot tear; mailbox
/// is the C++'s choice for the latency on top of it. A requested mode that is not
/// offered is an error because wgpu documents `FifoRelaxed`, `Immediate` and
/// `Mailbox` as "crash if unsupported": substituting fifo would be a frame-pacing
/// change reported as a success.
pub fn choose_present_mode(
    modes: &[wgpu::PresentMode],
    requested: Option<wgpu::PresentMode>,
) -> Result<wgpu::PresentMode, WgpuError> {
    if let Some(requested) = requested {
        if modes.contains(&requested) {
            return Ok(requested);
        }
        return Err(WgpuError::Unsupported {
            what: format!("{requested:?} presentation"),
            available: list(modes),
        });
    }
    if modes.contains(&wgpu::PresentMode::Mailbox) {
        return Ok(wgpu::PresentMode::Mailbox);
    }
    Ok(wgpu::PresentMode::Fifo)
}

/// The C++ passes `COMPOSITE_ALPHA_OPAQUE`. wgpu guarantees `Opaque` *or*
/// `Inherit`, so the opaque one is taken when offered.
pub fn choose_alpha_mode(modes: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    if modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
        return wgpu::CompositeAlphaMode::Opaque;
    }
    // wgpu documents at least one entry, so the fallback is for a list that
    // contradicts its own documentation -- and it is a value rather than an index
    // that would panic on exactly that.
    modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto)
}

/// A size `Surface::configure` will accept.
///
/// The C++ `chooseSwapExtent` clamps into the surface's min/max image extent and
/// handles `currentExtent == UINT32_MAX`, its "you choose" sentinel. wgpu resolves
/// that rule inside its own Vulkan layer (`wgpu-hal/src/vulkan/swapchain/native.rs`)
/// and exposes no `current_extent` on `SurfaceCapabilities` at all, so the size the
/// caller reports is the only one there is -- and `configure` panics on a zero one
/// and refuses anything above `max_texture_dimension_2d`, so both ends are clamped
/// here instead. A window larger than the device is a size the device cannot make,
/// not a reason to fail a frame.
pub fn choose_extent(requested: Extent2d, max: u32) -> Extent2d {
    let max = max.max(1);
    Extent2d {
        width: requested.width.clamp(1, max),
        height: requested.height.clamp(1, max),
    }
}

/// What an acquire produced, without the image.
///
/// [`wgpu::CurrentSurfaceTexture`] has the same seven outcomes, and two of them
/// carry the image -- so no test without a device could name them, and no caller
/// wants the image before deciding what to do with it. This is that half, and it is
/// the wgpu spelling of what the C++ swapchain reports as `VK_ERROR_OUT_OF_DATE_KHR`
/// and its neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireStatus {
    /// The swapchain matches the surface: draw and present.
    Success,
    /// Acquired, but the configuration no longer fits the surface. Usable, and
    /// worth a reconfigure before the next frame.
    Suboptimal,
    /// Nothing was ready within the backend's own timeout. Skip the frame.
    Timeout,
    /// The window is not visible. Skip the frame.
    Occluded,
    /// The surface changed: rebuild the configuration and acquire again.
    Outdated,
    /// The surface is gone and only its owner can recreate it.
    Lost,
    /// wgpu raised a validation error inside the acquire.
    Validation,
}

impl AcquireStatus {
    /// The image is in hand and the frame can be drawn.
    pub fn acquired(self) -> bool {
        matches!(self, Self::Success | Self::Suboptimal)
    }

    /// The frame is skipped and nothing is wrong with the swapchain.
    pub fn skips_frame(self) -> bool {
        matches!(self, Self::Timeout | Self::Occluded)
    }

    /// The configuration has to be rebuilt before the next acquire.
    pub fn needs_reconfigure(self) -> bool {
        matches!(self, Self::Outdated | Self::Lost)
    }
}

impl fmt::Display for AcquireStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcquireStatus::Success => write!(f, "acquired"),
            AcquireStatus::Suboptimal => {
                write!(f, "acquired, but the surface configuration is out of date")
            }
            AcquireStatus::Timeout => write!(f, "no image was ready in time"),
            AcquireStatus::Occluded => write!(f, "the window is not visible"),
            AcquireStatus::Outdated => write!(f, "the surface is out of date"),
            AcquireStatus::Lost => write!(f, "the surface was lost"),
            AcquireStatus::Validation => write!(f, "the acquire raised a validation error"),
        }
    }
}

/// The verdict behind an acquire result.
pub fn classify(result: &wgpu::CurrentSurfaceTexture) -> AcquireStatus {
    match result {
        wgpu::CurrentSurfaceTexture::Success(_) => AcquireStatus::Success,
        wgpu::CurrentSurfaceTexture::Suboptimal(_) => AcquireStatus::Suboptimal,
        wgpu::CurrentSurfaceTexture::Timeout => AcquireStatus::Timeout,
        wgpu::CurrentSurfaceTexture::Occluded => AcquireStatus::Occluded,
        wgpu::CurrentSurfaceTexture::Outdated => AcquireStatus::Outdated,
        wgpu::CurrentSurfaceTexture::Lost => AcquireStatus::Lost,
        wgpu::CurrentSurfaceTexture::Validation => AcquireStatus::Validation,
    }
}

/// What a surface offers, as one line for an error message.
fn list<T: fmt::Debug>(values: &[T]) -> String {
    if values.is_empty() {
        return "nothing".to_owned();
    }
    values
        .iter()
        .map(|value| format!("{value:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}
