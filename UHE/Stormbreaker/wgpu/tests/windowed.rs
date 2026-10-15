//! The windowed path: surface configuration, and the frame it hands out.
//!
//! Everything here that can be tested without a device is, because the alternative
//! is a suite that only runs on a machine with a display -- and the two decisions
//! this path is most likely to get wrong (a badly chosen extent, an out-of-date
//! acquire treated as an error) are decisions, not driver calls.

use chassis::{Extent2d, Swapchain, TextureFormat};
use stormglass::wgpu_swapchain::{
    choose_alpha_mode, choose_extent, choose_format, choose_present_mode, classify, images_for,
    latency_for, AcquireStatus, IMAGE_COUNT,
};
use stormglass::{WgpuError, WgpuGpu, WgpuSurface};

use wgpu::{CompositeAlphaMode, CurrentSurfaceTexture, PresentMode, TextureFormat as WgpuFormat};

// ── format, present mode, alpha mode ────────────────────────────────────────

/// The C++ `chooseSwapSurfaceFormat` preference, against wgpu's vocabulary.
///
/// BGRA8 sRGB rather than the surface's first format: it is the one both APIs
/// guarantee, and the C++ asks for `B8G8R8A8_SRGB` for the same reason.
#[test]
fn nothing_requested_prefers_srgb_bgra() {
    let formats = [WgpuFormat::Rgba8Unorm, WgpuFormat::Bgra8UnormSrgb];
    let chosen = choose_format(&formats, None).expect("bgra8 srgb is offered");
    assert_eq!(chosen, WgpuFormat::Bgra8UnormSrgb);
}

/// The surface's own first format when it offers no sRGB BGRA at all, which is the
/// C++'s `availableFormats[0]` fallback.
#[test]
fn a_surface_without_srgb_bgra_keeps_its_own_order() {
    let formats = [WgpuFormat::Rgba8UnormSrgb, WgpuFormat::Bgra8Unorm];
    let chosen = choose_format(&formats, None).expect("something is offered");
    assert_eq!(chosen, WgpuFormat::Rgba8UnormSrgb);
}

/// A requested format goes through the RHI's names and back, so a caller asks in
/// `chassis` terms and wgpu answers in its own.
#[test]
fn a_requested_format_is_honoured_when_the_surface_offers_it() {
    let formats = [WgpuFormat::Bgra8UnormSrgb, WgpuFormat::Rgba8Unorm];
    let plain = choose_format(&formats, Some(TextureFormat::Rgba8Unorm)).expect("offered");
    let srgb = choose_format(&formats, Some(TextureFormat::Bgra8Srgb)).expect("offered");
    assert_eq!(plain, WgpuFormat::Rgba8Unorm);
    assert_eq!(srgb, WgpuFormat::Bgra8UnormSrgb);
}

/// An unhonourable request is refused, not substituted.
///
/// `Surface::configure` **panics** on a format the surface does not offer, so this
/// check is the difference between an error and a crash -- and the message has to
/// name both sides, because "unsupported format" alone says nothing about which
/// display refused it.
#[test]
fn a_requested_format_the_surface_does_not_offer_is_an_error() {
    let formats = [WgpuFormat::Bgra8UnormSrgb];
    let error = choose_format(&formats, Some(TextureFormat::R16Float)).expect_err("not offered");
    let message = error.to_string();
    assert!(message.contains("R16Float"), "got {message}");
    assert!(message.contains("Bgra8UnormSrgb"), "got {message}");

    // The RHI's `Undefined` names no wgpu format either, so it cannot be offered.
    let undefined = choose_format(&formats, Some(TextureFormat::Undefined));
    assert!(undefined.is_err());
}

/// No formats at all is the surface saying it cannot present on this adapter.
#[test]
fn a_surface_with_no_formats_reports_itself_incompatible() {
    let error = choose_format(&[], None).expect_err("nothing to choose");
    assert_eq!(error, WgpuError::IncompatibleSurface);
}

/// Mailbox when offered, fifo otherwise -- the C++ `chooseSwapPresentMode`.
#[test]
fn mailbox_is_preferred_and_fifo_is_the_floor() {
    let both = [PresentMode::Fifo, PresentMode::Mailbox];
    let chosen = choose_present_mode(&both, None).expect("offered");
    assert_eq!(chosen, PresentMode::Mailbox);

    let fifo_only = [PresentMode::Fifo];
    let floor = choose_present_mode(&fifo_only, None).expect("fifo is always there");
    assert_eq!(floor, PresentMode::Fifo);
}

/// A requested mode that is not offered is refused.
///
/// wgpu documents `Mailbox`, `Immediate` and `FifoRelaxed` as "crash if
/// unsupported", so quietly taking fifo would be a frame-pacing change reported as
/// a success.
#[test]
fn a_requested_present_mode_the_surface_does_not_offer_is_an_error() {
    let fifo_only = [PresentMode::Fifo];
    let error = choose_present_mode(&fifo_only, Some(PresentMode::Immediate)).expect_err("fifo");
    assert!(error.to_string().contains("Immediate"), "got {error}");

    let mailbox = [PresentMode::Mailbox];
    let chosen = choose_present_mode(&mailbox, Some(PresentMode::Mailbox)).expect("offered");
    assert_eq!(chosen, PresentMode::Mailbox);
}

/// The C++ passes `COMPOSITE_ALPHA_OPAQUE`; wgpu guarantees `Opaque` or `Inherit`.
#[test]
fn opaque_is_preferred_and_a_contradictory_list_still_answers() {
    let both = [CompositeAlphaMode::Inherit, CompositeAlphaMode::Opaque];
    assert_eq!(choose_alpha_mode(&both), CompositeAlphaMode::Opaque);

    let inherit_only = [CompositeAlphaMode::Inherit];
    assert_eq!(choose_alpha_mode(&inherit_only), CompositeAlphaMode::Inherit);

    // wgpu documents at least one entry, so this cannot happen -- and it still
    // answers instead of indexing the empty list.
    assert_eq!(choose_alpha_mode(&[]), CompositeAlphaMode::Auto);
}

// ── extent ──────────────────────────────────────────────────────────────────

/// The C++ `chooseSwapExtent` clamp, which wgpu's `configure` enforces by panicking.
///
/// A zero-sized window is what a minimised one reports, and it is the case that
/// would panic: "`config.width` or `config.height` is zero" is in
/// `Surface::configure`'s panic list, not its error list.
#[test]
fn a_zero_extent_is_clamped_rather_than_handed_to_configure() {
    let clamped = choose_extent(Extent2d { width: 0, height: 0 }, 4096);
    assert_eq!(clamped, Extent2d { width: 1, height: 1 });
}

/// A window larger than the device is letterboxed by the C++ and clamped here.
///
/// `validate_surface_configuration` refuses anything above
/// `max_texture_dimension_2d`, so passing it through is a request the device cannot
/// answer.
#[test]
fn an_oversized_extent_is_clamped_to_the_device() {
    let big = Extent2d { width: 8000, height: 600 };
    let clamped = choose_extent(big, 4096);
    assert_eq!(clamped, Extent2d { width: 4096, height: 600 });
}

/// A size that fits is left alone: the clamp is a floor and a ceiling, not a rescale.
#[test]
fn an_extent_that_fits_is_unchanged() {
    let extent = Extent2d { width: 1280, height: 720 };
    assert_eq!(choose_extent(extent, 4096), extent);

    // A device that reports no texture size still gets a usable one rather than a
    // zero, which would clamp everything to nothing.
    assert_eq!(choose_extent(extent, 0), Extent2d { width: 1, height: 1 });
}

// ── image count ─────────────────────────────────────────────────────────────

/// The C++ `chooseSwapMinImageCount`, and wgpu's spelling of it.
///
/// Three images is latency 2, which is also wgpu's own default -- and the two
/// functions have to stay inverses, or `Swapchain::image_count` reports a number
/// nobody asked for.
#[test]
fn three_images_is_two_frames_of_latency() {
    assert_eq!(IMAGE_COUNT, 3);
    assert_eq!(latency_for(IMAGE_COUNT), 2);
    assert_eq!(images_for(latency_for(IMAGE_COUNT)), IMAGE_COUNT);

    // A latency of zero still asks for one image rather than underflowing.
    assert_eq!(latency_for(0), 0);
    assert_eq!(images_for(0), 1);
}

// ── acquire ─────────────────────────────────────────────────────────────────

/// Every verdict wgpu can return, named without a device.
///
/// Two of `CurrentSurfaceTexture`'s seven variants carry the image, and that is the
/// whole reason this mapping exists: the image is not what a frame loop decides on.
#[test]
fn every_acquire_verdict_is_named() {
    let verdicts = [
        (CurrentSurfaceTexture::Timeout, AcquireStatus::Timeout),
        (CurrentSurfaceTexture::Occluded, AcquireStatus::Occluded),
        (CurrentSurfaceTexture::Outdated, AcquireStatus::Outdated),
        (CurrentSurfaceTexture::Lost, AcquireStatus::Lost),
        (CurrentSurfaceTexture::Validation, AcquireStatus::Validation),
    ];
    for (verdict, expected) in &verdicts {
        assert_eq!(classify(verdict), *expected);
    }
}

/// The three questions a frame loop asks, over the whole set.
///
/// Out of date has to be *both* skippable and worth reconfiguring: the frame is
/// dropped and the swapchain rebuilt, in that order -- and treating it as an error
/// is what makes every window resize look like a crash.
#[test]
fn the_status_predicates_cover_the_set() {
    let all = [
        AcquireStatus::Success,
        AcquireStatus::Suboptimal,
        AcquireStatus::Timeout,
        AcquireStatus::Occluded,
        AcquireStatus::Outdated,
        AcquireStatus::Lost,
        AcquireStatus::Validation,
    ];
    for status in all {
        let acquired = matches!(status, AcquireStatus::Success | AcquireStatus::Suboptimal);
        let skipped = matches!(status, AcquireStatus::Timeout | AcquireStatus::Occluded);
        assert_eq!(status.acquired(), acquired, "{status}");
        assert_eq!(status.skips_frame(), skipped, "{status}");
    }

    // Out of date is a skip *and* a rebuild: the frame is dropped and the
    // configuration rebuilt, and only one of those is a decision this test can make
    // for the caller.
    assert!(AcquireStatus::Outdated.needs_reconfigure());
    assert!(AcquireStatus::Lost.needs_reconfigure());
    assert!(!AcquireStatus::Success.needs_reconfigure());
}

/// The real failures a frame loop must not ignore, in a message.
#[test]
fn the_acquire_status_reads_as_a_message() {
    assert!(AcquireStatus::Lost.to_string().contains("lost"));
    assert!(AcquireStatus::Validation.to_string().contains("validation"));

    // And the RHI only ever sees `Unsupported`, whatever the backend's own error was.
    let bridged: chassis::Error = WgpuError::NoSwapchain.into();
    assert!(matches!(bridged, chassis::Error::Unsupported(_)));
    assert!(bridged.to_string().contains("surface"));
}

// ── the seam ────────────────────────────────────────────────────────────────

/// Both the surface and the gpu are what the RHI asks for.
///
/// A compile-time claim, and the one the whole slice rests on: `Gpu::swapchain`
/// hands out `&dyn Swapchain`, so a backend whose surface is not one has no
/// windowed path however well it configures.
#[test]
fn the_surface_and_the_gpu_are_the_rhi_swapchain() {
    fn takes<T: Swapchain>() {}
    takes::<WgpuSurface>();
    takes::<WgpuGpu>();
}

// ── the device ──────────────────────────────────────────────────────────────

/// Headless is `None` from `swapchain`, and says so.
///
/// Needs an adapter, and the windowed half needs more than that: a `wgpu::Surface`
/// is created from a window handle, and this crate deliberately names no windowing
/// library, so the surface path cannot be reached from a test here at all. It is
/// driven by `bins/sandbox`, which has a window.
#[test]
#[ignore = "needs an adapter and a display; the surface path needs a window"]
fn a_gpu_with_no_surface_reports_no_swapchain() {
    let extent = Extent2d { width: 64, height: 64 };
    let Ok(mut gpu) = WgpuGpu::headless(extent) else {
        eprintln!("skipping: no wgpu adapter");
        return;
    };

    assert!(gpu.surface().is_none());
    assert!(gpu.swapchain().is_none(), "headless is not a lie: the option says so");
    assert_eq!(Swapchain::image_count(&gpu), 0);
    assert!(!Swapchain::is_out_of_date(&gpu));

    // Presenting with no surface is an error rather than a silent no-op: a caller
    // that presents headless asked the wrong thing.
    assert!(gpu.present().is_err());

    // Resizing headless has no swapchain to rebuild, so it only records the extent
    // the frame is recorded at.
    let smaller = Extent2d { width: 128, height: 96 };
    gpu.resize(smaller).expect("recording an extent cannot fail");
    assert_eq!(Swapchain::extent(&gpu), smaller);
}
