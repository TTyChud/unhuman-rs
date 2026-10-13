//! Slang source to a driver object, on real hardware.
//!
//! This is the seam between `forge` and `stormbreaker`. If it is wrong, nothing
//! renders and every symptom points somewhere else, so it is worth testing
//! against a driver rather than only against types.

use ash::vk::Handle;
use forge::cache::ShaderCache;
use forge::{CompileRequest, Forge, Slangc, Stage};
use stormbreaker::vulkan_shader::shader_stage_flags;
use stormbreaker::{create_device, device_available, BackendError, DeviceOptions};
use std::path::{Path, PathBuf};

fn shader() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/shaders/Triangle.slang")
}

fn open_device() -> Option<stormbreaker::Context> {
    if !device_available() {
        eprintln!("skipping: no Vulkan loader");
        return None;
    }
    match create_device(DeviceOptions::new(c"stormbreaker-shaders")) {
        Ok(context) => Some(context),
        Err(BackendError::Loader(why)) => {
            eprintln!("skipping: loader unavailable: {why}");
            None
        }
        Err(other) => panic!("device creation failed: {other}"),
    }
}

fn forge() -> Option<Forge> {
    let compiler = forge::slangc::SlangcCompiler::new(Slangc::default()).ok()?;
    Some(Forge::new(compiler, ShaderCache::disabled()))
}

#[test]
fn slang_source_becomes_a_real_shader_module() {
    let Some(context) = open_device() else { return };
    let Some(forge) = forge() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let scratch = std::env::temp_dir().join("stormbreaker-shader.vert.spv");
    let mut shaders = stormbreaker::vulkan_shader::Shaders::new();

    let handle = shaders
        .load_slang(
            &context.device,
            &forge,
            CompileRequest::new(shader(), "vs_main", Stage::Vertex),
            &scratch,
        )
        .expect("Slang source compiles to a module the driver accepts");

    let resource = shaders.get(handle).expect("handle resolves");
    assert_eq!(resource.stage(), ash::vk::ShaderStageFlags::VERTEX);
    assert!(
        !resource.module().is_null(),
        "the driver handed back a real VkShaderModule",
    );
    assert_eq!(shaders.len(), 1);

    let (compiled, reused) = shaders.stats();
    assert_eq!(compiled, 1, "one module created");
    assert_eq!(reused, 0, "and none reused on the first request");
}

/// The same SPIR-V for two stages is two modules: Vulkan's dedup key includes
/// the stage, and collapsing them would hand the vertex stage to a fragment
/// pipeline that then fails deep inside the driver.
#[test]
fn one_source_yields_two_distinct_stage_modules() {
    let Some(context) = open_device() else { return };
    let Some(forge) = forge() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let directory = std::env::temp_dir().join("stormbreaker-shader-pair");
    std::fs::create_dir_all(&directory).expect("scratch");
    let mut shaders = stormbreaker::vulkan_shader::Shaders::new();

    let vertex = shaders
        .load_slang(
            &context.device,
            &forge,
            CompileRequest::new(shader(), "vs_main", Stage::Vertex),
            &directory.join("a.vert.spv"),
        )
        .expect("vertex stage");
    let fragment = shaders
        .load_slang(
            &context.device,
            &forge,
            CompileRequest::new(shader(), "fs_main", Stage::Fragment),
            &directory.join("a.frag.spv"),
        )
        .expect("fragment stage");

    assert_ne!(vertex, fragment, "two handles for two stages");
    assert_eq!(shaders.len(), 2);
    assert_eq!(shaders.get(vertex).unwrap().stage(), ash::vk::ShaderStageFlags::VERTEX);
    assert_eq!(
        shaders.get(fragment).unwrap().stage(),
        ash::vk::ShaderStageFlags::FRAGMENT
    );
}

/// A bad shader must be refused with the compiler's own message, pointing at a
/// line of shader source. A generic "module creation failed" here would send
/// someone looking at the backend instead of the shader.
#[test]
fn a_rejected_shader_reports_the_compilers_own_diagnostic() {
    let _ = open_device(); // A driver is not needed to reject a shader.
    let Some(forge) = forge() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let broken = std::env::temp_dir().join("stormbreaker-broken.slang");
    std::fs::write(
        &broken,
        "float4 fs_main() : SV_Target0 { return does_not_exist; }\n[shader(\"fragment\")]\n",
    )
    .expect("write");

    let error = forge
        .compile(
            &CompileRequest::new(&broken, "fs_main", Stage::Fragment),
            &std::env::temp_dir().join("stormbreaker-broken.spv"),
        )
        .expect_err("an undefined identifier is an error");
    let text = error.to_string();
    assert!(
        text.contains("does_not_exist"),
        "the message names the identifier, got {text:?}",
    );
}

/// ash 0.38 does not expose mesh or ray tracing stages in core, and this backend
/// does not gate them. Reporting them as bindable would create a pipeline that
/// succeeds and draws nothing, so they map to nothing instead.
#[test]
fn stages_the_driver_cannot_bind_map_to_nothing() {
    assert_eq!(shader_stage_flags(Stage::Vertex), Some(ash::vk::ShaderStageFlags::VERTEX));
    assert_eq!(
        shader_stage_flags(Stage::Fragment),
        Some(ash::vk::ShaderStageFlags::FRAGMENT)
    );
    assert_eq!(
        shader_stage_flags(Stage::Compute),
        Some(ash::vk::ShaderStageFlags::COMPUTE)
    );
    for stage in [
        Stage::Task,
        Stage::Mesh,
        Stage::Amplification,
        Stage::Raygen,
        Stage::Miss,
        Stage::Callable,
        Stage::Intersection,
    ] {
        assert_eq!(
            shader_stage_flags(stage),
            None,
            "{stage} has no core VkShaderStageFlag here, and must not map to an empty flag",
        );
    }
}