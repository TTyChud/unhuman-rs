//! Backend selection, and `chassis::Gpu` as a trait object.
//!
//! `chassis::Gpu` had **zero** implementations until now. These tests are the
//! ones that make it a seam rather than a description.

use chassis::{
    backend::{flag_value, BackendChoice, BackendSelection, BackendSource},
    Gpu, GpuInfo, Swapchain,
};
use stormglass::wgpu_device::Adapter;
use stormglass::wgpu_gpu::WgpuGpu;

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

// ── selection ───────────────────────────────────────────────────────────────

/// wgpu is the default.
///
/// A stability decision, not a performance one: when a frame goes wrong the
/// first question is "the backend or the engine", and the default should be the
/// one that usually works.
#[test]
fn wgpu_is_the_default_backend() {
    assert_eq!(BackendChoice::default(), BackendChoice::Wgpu);
    assert_eq!(BackendSelection::default().choice, BackendChoice::Wgpu);
    assert_eq!(
        BackendSelection::default().source,
        BackendSource::Default,
        "and it says it was a default, not a choice",
    );
}

/// The flag wins over everything.
///
/// The case that silently stops being true when someone adds a fourth source,
/// which is why it is a test rather than a comment.
#[test]
fn the_flag_beats_the_environment() {
    let selection = BackendSelection::resolve(&arguments(&["--backend", "vulkan"]), Some("wgpu"))
        .expect("both are valid");
    assert_eq!(selection.choice, BackendChoice::Vulkan);
    assert_eq!(selection.source, BackendSource::Flag);
}

/// Both flag spellings. A parser that takes one and not the other is a trap that
/// only shows up on the invocation nobody tested.
#[test]
fn both_flag_spellings_work() {
    for form in [
        arguments(&["--backend", "vulkan"]),
        arguments(&["--backend=vulkan"]),
    ] {
        let selection = BackendSelection::resolve(&form, None).expect("valid");
        assert_eq!(selection.choice, BackendChoice::Vulkan, "for {form:?}");
        assert_eq!(selection.source, BackendSource::Flag);
    }
}

#[test]
fn the_environment_is_used_when_there_is_no_flag() {
    let selection = BackendSelection::resolve(&[], Some("vulkan")).expect("valid");
    assert_eq!(selection.choice, BackendChoice::Vulkan);
    assert_eq!(selection.source, BackendSource::Environment);

    // An empty value is nobody saying anything, so the default stands rather than
    // an error -- an exported-but-blank variable is a normal accident.
    let blank = BackendSelection::resolve(&[], Some("  ")).expect("blank is not a choice");
    assert_eq!(blank.choice, BackendChoice::Wgpu);
    assert_eq!(blank.source, BackendSource::Default);
}

/// An unrecognised backend is an error, not a silent fallback.
///
/// A launcher that passes `--backend vlkan` and silently gets wgpu has a bug that
/// will be reported as "the wrong backend", and nothing would point here.
#[test]
fn a_misspelled_backend_is_an_error_naming_what_was_asked_for() {
    let error = BackendSelection::resolve(&arguments(&["--backend", "vlkan"]), None)
        .expect_err("a typo is not a backend");
    assert!(
        error.to_string().contains("vlkan"),
        "the message quotes what was asked for, got {error}",
    );
    // And it says what would have worked.
    assert!(error.to_string().contains("wgpu"));
    assert!(error.to_string().contains("vulkan"));

    // An empty flag value is nobody saying anything, and it must not parse as a
    // backend: `--backend ""` is a launcher bug, not a request for the default.
    assert!("".parse::<BackendChoice>().is_err());
}

#[test]
fn names_are_matched_case_insensitively_and_trimmed() {
    for spelling in ["vulkan", "VULKAN", " Vulkan ", "\tvk\n"] {
        let selection = BackendSelection::resolve(&arguments(&["--backend", spelling]), None)
            .unwrap_or_else(|e| panic!("{spelling:?} should parse: {e}"));
        assert_eq!(selection.choice, BackendChoice::Vulkan, "for {spelling:?}");
    }
}

/// A flag with no value after it falls through rather than panicking.
///
/// `--backend` alone is a launcher bug, and the useful response is to use the
/// default and say so -- not to crash before the window opens.
#[test]
fn a_flag_with_no_value_falls_through_to_the_default() {
    let selection =
        BackendSelection::resolve(&arguments(&["--backend"]), None).expect("does not panic");
    assert_eq!(selection.choice, BackendChoice::Wgpu);
}

#[test]
fn unrelated_arguments_are_left_alone() {
    assert_eq!(flag_value(&arguments(&["--width", "1280"])), None);
    assert_eq!(
        flag_value(&arguments(&["--backend", "vulkan", "--width", "1"])),
        Some("vulkan".into())
    );
}

// ── the trait object ────────────────────────────────────────────────────────

fn open_gpu() -> Option<WgpuGpu> {
    if Adapter::request(0).is_none() {
        eprintln!("skipping: no wgpu adapter");
        return None;
    }
    WgpuGpu::headless(chassis::Extent2d {
        width: 64,
        height: 64,
    })
    .ok()
}

/// The first implementation of `chassis::Gpu` in this project, reached as
/// `dyn Gpu`.
///
/// Everything above the RHI will hold this, not `WgpuGpu`. That is the claim
/// the whole two-backend design rests on and it has never compiled until now.
#[test]
fn a_wgpu_backend_is_usable_as_dyn_gpu() {
    let Some(gpu) = open_gpu() else { return };
    let mut gpu: Box<dyn Gpu> = Box::new(gpu);

    let info: &GpuInfo = gpu.info();
    assert!(!info.name.is_empty(), "the device reported a name");
    assert!(info.max_texture_size >= 2048);
    assert_eq!(
        info.frames_in_flight, 1,
        "one, until more are actually wired"
    );

    gpu.frame_begin().expect("frame begin");
    gpu.submit().expect("submit with no frontend attached");
    gpu.wait_idle();

    // Resources are reachable through the trait, and objects made through it are
    // real. A `dyn Resources` that cannot create anything is not a seam.
    let resources: &mut dyn chassis::Resources = gpu.resources();
    let buffer = resources
        .create_buffer(
            chassis::BufferDesc::new(256, chassis::BufferUsage::VERTEX),
            Some(&[3u8; 256]),
        )
        .expect("a buffer through dyn Resources");
    let texture = resources
        .create_texture(chassis::TextureDesc::new_2d(
            chassis::Extent {
                width: 32,
                height: 32,
                depth: 1,
            },
            chassis::TextureFormat::Rgba8Unorm,
            chassis::TextureUsage::COLOR_ATTACHMENT,
        ))
        .expect("a texture through dyn Resources");
    resources.destroy_texture(texture);
    resources.destroy_buffer(buffer);

    let _: &mut dyn chassis::RenderGraph = gpu.graph();
}

/// A headless backend has no swapchain and says so.
///
/// `Gpu::swapchain` returns an `Option` for exactly this, and this is the first
/// implementation that takes the opportunity: a dummy would make every caller
/// carry one.
#[test]
fn a_headless_backend_has_no_swapchain() {
    let Some(gpu) = open_gpu() else { return };
    let gpu: &dyn Gpu = &gpu;
    assert!(
        gpu.swapchain().is_none(),
        "headless is a real mode, not a degraded one",
    );
}

/// The same object is both `Gpu` and `Swapchain`, and `Gpu::swapchain` wins.
///
/// Trivial, and worth pinning: the two impls coexisting is what makes the
/// `Option` above possible without a wrapper type.
#[test]
fn the_concrete_type_is_both_a_gpu_and_a_swapchain() {
    let Some(gpu) = open_gpu() else { return };
    let as_swapchain: &dyn Swapchain = &gpu;
    assert_eq!(as_swapchain.extent().width, 64);
    assert!(
        !as_swapchain.is_out_of_date(),
        "a headless frame is never out of date"
    );
}

/// A handle made through `dyn Resources` must resolve inside the executor.
///
/// The bug this pins: `WgpuGpu` used to build one `WgpuResources` for itself
/// and hand a *second* to the executor, so every texture created through the
/// trait had a handle the executor could not resolve. Nothing caught it, because
/// the tests that created resources never recorded a pass with them -- the draw
/// test built its own single registry by hand.
#[test]
fn a_resource_made_through_the_trait_resolves_in_the_executor() {
    let Some(mut gpu) = open_gpu() else { return };
    let extent = chassis::Extent2d {
        width: 32,
        height: 32,
    };

    let texture = {
        let resources: &mut dyn chassis::Resources = gpu.resources();
        resources
            .create_texture(chassis::TextureDesc::new_2d(
                chassis::Extent {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                },
                chassis::TextureFormat::Rgba8Unorm,
                chassis::TextureUsage::COLOR_ATTACHMENT,
            ))
            .expect("a texture through dyn Resources")
    };

    // The same handle, asked of the executor. Different registries would give
    // `None` here, which is exactly the failure the two-registry bug produced.
    let resolved = gpu
        .executor()
        .resources()
        .get_texture(texture)
        .expect("the handle resolves in the executor");
    assert_eq!(resolved.extent().width, extent.width);
    assert_eq!(resolved.format(), chassis::TextureFormat::Rgba8Unorm);

    // And through the slot table the executor records from.
    gpu.slots_mut().set_texture(0, texture);
    assert_eq!(gpu.slots().texture(0), Some(texture));
    assert!(
        gpu.executor().resources().get_texture(texture).is_some(),
        "the slot the executor will look it up in holds the handle it can resolve",
    );
}
