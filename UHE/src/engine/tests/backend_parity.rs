//! The same assertions, both backends, through `Box<dyn Gpu>` and nothing else.
//!
//! ## Why this file exists
//!
//! "Both backends implement `chassis::Gpu`" is a claim about types: the compiler
//! checked it when `Box<dyn Gpu>` was returned from `create_gpu`. It says nothing
//! about whether a texture created through the trait resolves inside the
//! executor, or whether `submit` on an empty graph succeeds on both.
//!
//! This is the claim about behaviour. One function, two backends, reached only
//! as `&mut dyn Gpu`. If a backend needs a concrete method to pass, that is a
//! finding, not a test failure to work around.
//!
//! ## Each case has to opt out explicitly
//!
//! No test here quietly returns on failure. A backend that cannot open prints
//! `SKIP` and says why, and the run's exit status still reflects the cases that
//! did run. A parity suite that reports green while having tested nothing is the
//! exact failure the validation gate once had.

use chassis::backend::{BackendChoice, BackendSelection, BackendSource};
use chassis::{Gpu, Resources};
use engine::{create_gpu, GpuRequest};

fn extent() -> chassis::Extent2d {
    chassis::Extent2d {
        width: 32,
        height: 32,
    }
}

/// Builds both backends. `None` names the one that would not open.
fn open(choice: BackendChoice) -> Option<Box<dyn Gpu>> {
    let gpu = create_gpu(GpuRequest {
        selection: BackendSelection {
            choice,
            source: BackendSource::Default,
        },
        engine_name: c"unhuman",
        extent: extent(),
    });
    match gpu {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("SKIP {choice:?}: {error}");
            None
        }
    }
}

macro_rules! both_backends {
    ($name:ident, $body:expr) => {
        #[test]
        fn $name() {
            let mut ran = 0;
            for choice in BackendChoice::all() {
                let Some(mut gpu) = open(choice) else {
                    continue;
                };
                let context = format!("{choice:?}");
                $body(&mut *gpu, &context);
                ran += 1;
            }
            assert!(
                ran > 0,
                "no backend opened, so {} proved nothing",
                stringify!($name),
            );
        }
    };
}

both_backends!(
    a_backend_reports_itself,
    |gpu: &mut dyn Gpu, _context: &str| {
        let info = gpu.info();
        assert!(
            !info.name.is_empty(),
            "a device with no name is a bug report"
        );
        assert!(info.max_texture_size > 0);
        assert!(
            info.frames_in_flight >= 1,
            "a frame loop with zero depth stalls"
        );
    }
);

both_backends!(
    a_headless_backend_has_no_swapchain,
    |gpu: &mut dyn Gpu, _context: &str| {
        assert!(
            gpu.swapchain().is_none(),
            "headless means no swapchain, and the Option is how that is said",
        );
    }
);

both_backends!(
    a_frame_with_no_passes_submits,
    |gpu: &mut dyn Gpu, _context: &str| {
        gpu.frame_begin().expect("frame_begin");
        gpu.submit().expect("an empty frame is a real frame");
        gpu.wait_idle();
    }
);

both_backends!(
    a_resource_made_through_the_trait_is_real,
    |gpu: &mut dyn Gpu, context: &str| {
        let texture = {
            let resources: &mut dyn Resources = gpu.resources();
            resources
                .create_texture(chassis::TextureDesc::new_2d(
                    chassis::Extent {
                        width: extent().width,
                        height: extent().height,
                        depth: 1,
                    },
                    chassis::TextureFormat::Rgba8Unorm,
                    chassis::TextureUsage::COLOR_ATTACHMENT,
                ))
                .unwrap_or_else(|error| panic!("{context}: texture through dyn Resources: {error}"))
        };
        let buffer = {
            let resources: &mut dyn Resources = gpu.resources();
            resources
                .create_buffer(
                    chassis::BufferDesc::new(256, chassis::BufferUsage::VERTEX),
                    None,
                )
                .unwrap_or_else(|error| panic!("{context}: buffer through dyn Resources: {error}"))
        };

        // Both must survive into a frame that uses them. A backend that kept its own
        // registry beside the trait's would create these happily and then fail at
        // record time, which is why this submits rather than just creates.
        gpu.frame_begin().expect("frame_begin");
        gpu.submit()
            .expect("submit after creating through the trait");
        gpu.wait_idle();
        assert!(!texture.is_null(), "{context}: texture handle");
        assert!(!buffer.is_null(), "{context}: buffer handle");
    }
);

both_backends!(
    a_graph_can_be_reached_and_reset,
    |gpu: &mut dyn Gpu, context: &str| {
        gpu.frame_begin().expect("frame_begin");
        gpu.graph().reset();
        gpu.submit().expect("submit after an explicit reset");
        gpu.wait_idle();
        let _ = context;
    }
);

both_backends!(two_frames_in_a_row, |gpu: &mut dyn Gpu, context: &str| {
    for frame in 0..2 {
        gpu.frame_begin()
            .unwrap_or_else(|e| panic!("{context} frame {frame}: {e}"));
        gpu.submit()
            .unwrap_or_else(|e| panic!("{context} submit {frame}: {e}"));
    }
    gpu.wait_idle();
});

/// Dropping the `Box` must destroy the device cleanly, for each backend.
///
/// Not written through the macro: that hands out a `&mut dyn Gpu`, and dropping
/// a reference drops nothing. The point is the `Box` -- the last owner of the
/// device -- going away.
///
/// This is where `VulkanGpu`'s field order got caught. With `context` declared
/// first, `vkDestroyDevice` ran before the recorder freed its command pools, and
/// all 64 command buffers leaked. Under validation, on whichever test happened to
/// build a device first -- so it blamed that test rather than the struct.
#[test]
fn the_device_survives_going_away() {
    let mut dropped = 0;
    for choice in BackendChoice::all() {
        let Some(gpu) = open(choice) else {
            continue;
        };
        drop(gpu);
        dropped += 1;
    }
    assert!(dropped > 0, "no backend opened, so teardown proved nothing");
}

/// Both names resolve, both are compiled in, and the default is wgpu.
#[test]
fn the_default_is_wgpu_and_both_backends_exist() {
    assert_eq!(BackendChoice::default(), BackendChoice::Wgpu);
    assert!(BackendChoice::Vulkan.is_compiled_in());
    assert!(BackendChoice::Wgpu.is_compiled_in());
}

/// The trait is object safe enough to be the only handle a caller holds.
#[test]
fn the_box_is_all_the_caller_has() {
    fn takes_only_the_trait(_: &mut dyn Gpu) {}
    let Some(mut gpu) = open(BackendChoice::Wgpu) else {
        eprintln!("SKIP Wgpu: no device");
        return;
    };
    takes_only_the_trait(&mut *gpu);
    let _: Option<&dyn chassis::Swapchain> = gpu.swapchain();
    let _: &mut dyn chassis::RenderGraph = gpu.graph();
    let _: &mut dyn Resources = gpu.resources();
    let info: &chassis::GpuInfo = gpu.info();
    assert!(info.frames_in_flight >= 1);
}
