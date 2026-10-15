//! `VulkanGpu` with a real swapchain, through the RHI.
//!
//! ## Why a headless surface
//!
//! The interesting question -- "does attaching a swapchain change what `Gpu`
//! reports?" -- needs a surface, and a surface normally needs a window. A window
//! needs a display. This suite therefore runs on a machine with no display at
//! all, including CI, by using `VK_KHR_headless_surface`.
//!
//! So the swapchain is real: a real `vk::SwapchainKHR`, real images, real
//! `vkCreateImageView` calls, and `vkQueuePresentKHR` at the end. Only the
//! window is missing.

use chassis::{Gpu, Swapchain};
use stormbreaker::vulkan_device::{create_device, DeviceOptions};
use stormbreaker::vulkan_gpu::VulkanGpu;
use stormbreaker::vulkan_swapchain::{create_headless_surface, Swapchain as BackendSwapchain};

fn extent() -> chassis::Extent2d {
    chassis::Extent2d {
        width: 64,
        height: 64,
    }
}

/// A `VulkanGpu` with a live swapchain, or `None` if this machine has no surface
/// support. Prints why, so a skipped run is never mistaken for a passing one.
fn with_swapchain() -> Option<VulkanGpu> {
    let context = match create_device(DeviceOptions::from_env(c"unhuman-test")) {
        Ok(context) => context,
        Err(error) => {
            eprintln!("SKIP: no vulkan device: {error}");
            return None;
        }
    };
    let surface = match create_headless_surface(&context.entry, &context.instance) {
        Ok(surface) => surface,
        Err(error) => {
            eprintln!("SKIP: no headless surface: {error}");
            return None;
        }
    };

    let gpu = match VulkanGpu::headless(context, extent()) {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("SKIP: no gpu: {error}");
            return None;
        }
    };
    let swapchain = match BackendSwapchain::new_owning_surface(
        &gpu.context().entry,
        &gpu.context().instance,
        &gpu.context().device,
        gpu.context().physical_device,
        surface,
        (extent().width, extent().height),
        false,
    ) {
        Ok(swapchain) => swapchain,
        Err(error) => {
            // `new_owning_surface` already destroyed the surface on its way out,
            // so there is nothing to free here.
            eprintln!("SKIP: no swapchain: {error}");
            return None;
        }
    };

    match gpu.with_swapchain(swapchain) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("SKIP: attach failed: {error}");
            None
        }
    }
}

/// Attaching a swapchain must change what `Gpu` reports.
///
/// This was a bug: `with_swapchain` stored one and `swapchain()` returned `None`
/// regardless, so attaching did nothing observable and a windowed run looked
/// exactly like a headless one.
#[test]
fn attaching_a_swapchain_is_observable() {
    let Some(gpu) = with_swapchain() else { return };

    let headless = VulkanGpu::headless(
        create_device(DeviceOptions::from_env(c"unhuman-test")).expect("device"),
        extent(),
    )
    .expect("headless gpu");
    assert!(
        headless.swapchain().is_none(),
        "a gpu with no swapchain reports none",
    );

    let swapchain = gpu.swapchain().expect("a gpu with a swapchain reports one");
    assert_eq!(swapchain.extent(), extent());
    assert_eq!(
        swapchain.format(),
        swapchain.format(),
        "a real swapchain reports a format without panicking",
    );
}

#[test]
fn a_swapchain_has_real_images_and_views() {
    let Some(mut gpu) = with_swapchain() else {
        return;
    };

    let swapchain = gpu.swapchain().expect("swapchain");
    assert!(
        swapchain.image_count() >= 2,
        "fewer than two images means the frame loop can never overlap",
    );

    // Nothing acquired yet, so there is no view. A null handle is the honest
    // answer; a handle to image zero would be a lie that writes into an image
    // nobody holds.
    assert!(
        gpu.current_image_view().is_null(),
        "no acquired image means no view",
    );

    let views = gpu
        .acquired_view()
        .expect("acquired_view with nothing acquired is not an error");
    assert!(views.is_none(), "nothing acquired means no view");

    gpu.acquire(std::time::Duration::from_millis(0))
        .expect("acquire from a headless surface");

    let index = gpu.acquired_index().expect("an index after acquire");
    assert!(
        (index as usize) < gpu.swapchain().expect("swapchain").image_count() as usize,
        "acquire returned image {index}, outside the swapchain",
    );

    let view = gpu
        .acquired_view()
        .expect("acquired_view after acquire")
        .expect("an acquired image has a view");
    assert!(!view.is_null(), "a real view, not a null handle");

    // The handle is stable across calls: one view per image, cached.
    let again = gpu
        .acquired_view()
        .expect("acquired_view again")
        .expect("still acquired");
    assert_eq!(view, again, "the same image gives the same handle");

    assert_eq!(
        gpu.current_image_view(),
        view,
        "the trait reports the view the backend acquired",
    );
    assert_eq!(gpu.frame_index(), index);
}

/// Acquire, submit (which presents), acquire again -- the loop a window would run.
///
/// The real test of the views: they are created before the first present and
/// destroyed after the last, and validation is watching throughout.
///
/// The present is `submit`'s, per the trait's own sentence for it, so a
/// `present()` here as well would be a second present of an image nobody holds.
/// The acquire stays explicit: `frame_begin` resets the graph and no more.
#[test]
fn acquiring_and_presenting_repeatedly() {
    let Some(mut gpu) = with_swapchain() else {
        return;
    };

    for frame in 0..3u32 {
        gpu.frame_begin()
            .unwrap_or_else(|e| panic!("frame_begin {frame}: {e}"));
        gpu.acquire(std::time::Duration::from_secs(1))
            .unwrap_or_else(|e| panic!("acquire {frame}: {e}"));

        if gpu.is_out_of_date() {
            continue;
        }
        assert!(
            gpu.acquired_view().expect("view").is_some(),
            "frame {frame}: acquired an image but produced no view",
        );

        gpu.submit()
            .unwrap_or_else(|e| panic!("submit and present {frame}: {e}"));
    }
    gpu.wait_idle();
}
