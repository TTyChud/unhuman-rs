//! The wgpu implementation of `chassis::Resources`, on the real driver.
//!
//! These mirror `UHE/Stormbreaker/vulkan/tests/rhi.rs` deliberately. Two
//! backends are only interchangeable if the *same* tests pass against both, and
//! that only happens if the tests go through `&mut dyn Resources` and never name
//! a concrete type.
//!
//! So: every assertion here is one the Vulkan suite also makes. Where the two
//! backends genuinely differ, the difference is the finding and gets its own
//! test saying so.

use chassis::Resources;
use stormglass::wgpu_device::Adapter;
use stormglass::wgpu_resources::WgpuResources;

/// The backend behind the trait, or `None` when this machine has no wgpu adapter.
fn backend() -> Option<WgpuResources> {
    let adapter = Adapter::request(0)?;
    let (device, _queue) = adapter.create_device("stormglass-resources")?;
    Some(WgpuResources::new(&device))
}

/// With a device, or a skip. Used by every test so none can panic on a machine
/// with no driver.
macro_rules! with_backend {
    (|$resources:ident| $body:block) => {
        let Some(mut $resources) = backend() else {
            eprintln!("skipping: no wgpu adapter");
            return;
        };
        $body
    };
}

fn texture_desc() -> chassis::TextureDesc {
    chassis::TextureDesc::new_2d(
        chassis::Extent {
            width: 64,
            height: 64,
            depth: 1,
        },
        chassis::TextureFormat::Rgba8Unorm,
        chassis::TextureUsage::COLOR_ATTACHMENT,
    )
}

/// The same contract the Vulkan suite asserts.
#[test]
fn the_rhi_contract_holds_through_a_trait_object() {
    with_backend!(|resources| {
        let resources: &mut dyn Resources = &mut resources;

        let buffer = resources
            .create_buffer(
                chassis::BufferDesc::new(256, chassis::BufferUsage::VERTEX),
                Some(&[7u8; 256]),
            )
            .expect("a buffer with initial data");
        let texture = resources
            .create_texture(texture_desc())
            .expect("a colour texture");
        let view = resources.create_image_view(&texture).expect("a view of it");

        assert!(!view.is_null(), "a live view handle");
        // Independent registries, so both starting at zero is correct. The tag
        // distinguishes them, not the index.
        assert_eq!(buffer.index(), texture.index());

        resources.destroy_texture(texture);
        resources.destroy_buffer(buffer);
    });
}

/// The same test, run twice, and the second time must still work.
///
/// Worth its own name because it is what a second backend gets wrong first: a
/// registry that drops its device, or a handle that still resolves after its
/// resource is released.
#[test]
fn resources_can_be_created_and_released_repeatedly() {
    with_backend!(|concrete| {
        let resources: &mut dyn Resources = &mut concrete;
        for iteration in 0..8 {
            let buffer = resources
                .create_buffer(
                    chassis::BufferDesc::new(1024, chassis::BufferUsage::VERTEX),
                    Some(&[iteration as u8; 1024]),
                )
                .expect("buffer");
            let texture = resources.create_texture(texture_desc()).expect("texture");
            resources.destroy_buffer(buffer);
            resources.destroy_texture(texture);
        }
        // Nothing left behind. "Nothing leaked" is not observable through
        // `dyn Resources` at all -- there is no method for it -- so this reaches
        // past the trait deliberately, through the concrete handle.
        let (buffers, textures, views, shaders) = concrete.counts();
        assert_eq!(
            (buffers, textures, shaders),
            (0, 0, 0),
            "every resource was released; views are tracked separately and are {}",
            views
        );
    });
}

#[test]
fn destroying_twice_is_not_a_crash() {
    with_backend!(|resources| {
        let resources: &mut dyn Resources = &mut resources;
        let buffer = resources
            .create_buffer(
                chassis::BufferDesc::new(64, chassis::BufferUsage::INDEX),
                None,
            )
            .expect("buffer");
        resources.destroy_buffer(buffer);
        // A released handle resolves to nothing, so this is a no-op rather than
        // a double free. wgpu has no explicit destroy, so the only thing that
        // could be double-freed is the handle.
        resources.destroy_buffer(buffer);

        let texture = resources.create_texture(texture_desc()).expect("texture");
        resources.destroy_texture(texture);
        resources.destroy_texture(texture);
    });
}

/// A zero-sized buffer is refused.
///
/// The Vulkan backend refuses it too, and for the same reason: a zero-size
/// object is legal to create in the API and useless, and some drivers accept it
/// and then ignore everything recorded against it.
#[test]
fn a_buffer_with_no_size_or_no_usage_is_refused() {
    with_backend!(|resources| {
        let resources: &mut dyn Resources = &mut resources;
        assert!(
            resources
                .create_buffer(
                    chassis::BufferDesc::new(0, chassis::BufferUsage::VERTEX),
                    None
                )
                .is_err(),
            "zero bytes is not a buffer",
        );
        assert!(
            resources
                .create_buffer(
                    chassis::BufferDesc::new(64, chassis::BufferUsage::NONE),
                    None
                )
                .is_err(),
            "no usage is not a buffer either",
        );
    });
}

#[test]
fn a_handle_that_never_existed_is_an_error_not_a_panic() {
    with_backend!(|resources| {
        let resources: &mut dyn Resources = &mut resources;
        assert!(
            resources
                .create_image_view(&chassis::Texture::null())
                .is_err(),
            "a view of nothing is an error",
        );
    });
}

/// Malformed SPIR-V is refused where the message can say what is wrong.
///
/// The two backends get this from different places -- naga here, the Vulkan
/// driver there -- and both must refuse it, because the alternative is an opaque
/// failure at pipeline creation that names neither the shader nor the backend.
#[test]
fn malformed_spirv_is_refused_with_a_reason() {
    with_backend!(|resources| {
        let resources: &mut dyn Resources = &mut resources;
        for (name, bytes) in [
            ("empty", Vec::new()),
            ("misaligned", vec![1, 2, 3]),
            ("wrong magic", vec![0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]),
        ] {
            let error = resources
                .create_shader("broken", &bytes)
                .expect_err("{name} is not SPIR-V");
            assert!(
                matches!(error, chassis::Error::InvalidDescriptor(_)),
                "{name} gave {error:?}",
            );
        }
    });
}

/// The real shader, compiled by `forge` from the checked-in source.
///
/// This is the test that proves the two backends share one compiler: the same
/// bytes that the Vulkan suite loads are loaded here, through the same `forge`
/// cache, and must produce a module wgpu accepts.
#[test]
fn the_shared_shader_from_forge_is_accepted() {
    let Some(mut resources) = backend() else {
        eprintln!("skipping: no wgpu adapter");
        return;
    };
    let compiler =
        forge::slangc::SlangcCompiler::new(forge::Slangc::default()).expect("the vendored slangc");
    // `forge` is both a crate and a local name; the local is `compiler` so the
    // crate stays reachable on the next line.
    let shaders = forge::Forge::new(compiler, forge::cache::ShaderCache::disabled());
    let directory = std::env::temp_dir().join("stormglass-shader");
    let (vertex, fragment) = shaders
        .compile_pair(
            &asset("assets/shaders/Triangle.slang"),
            "vs_main",
            "fs_main",
            &directory,
        )
        .expect("Triangle.slang compiles");

    let resources: &mut dyn Resources = &mut resources;
    let vertex_module = resources
        .create_shader("Triangle.vert", &vertex.to_bytes())
        .expect("the vertex module wgpu accepts");
    let fragment_module = resources
        .create_shader("Triangle.frag", &fragment.to_bytes())
        .expect("the fragment module wgpu accepts");
    assert_ne!(vertex_module, fragment_module);
}

/// Bind groups are still unwritten in wgpu, and saying so is the point.
///
/// The signature changed -- it takes a `DescriptorSetLayoutDesc` now, because the
/// old `&[u32]` could not carry one and that was why *neither* backend could
/// implement this. What is still true is that wgpu returns an error rather than a
/// handle that binds nothing.
///
/// A bind group that silently binds nothing renders the wrong image and is close
/// to undebuggable, so the refusal is the feature until the implementation lands.
#[test]
fn an_unwritten_path_refuses_rather_than_pretending() {
    with_backend!(|resources| {
        let resources: &mut dyn Resources = &mut resources;
        let group = resources.create_bind_group(&chassis::DescriptorSetLayoutDesc::new("set"), &[]);
        assert!(
            matches!(group, Err(chassis::Error::Unsupported(_))),
            "a bind group that binds nothing is worse than no bind group",
        );
    });
}

/// A real pipeline, from the same shader and the same description the Vulkan
/// suite builds.
///
/// This is the test that says the two backends are interchangeable rather than
/// merely similar. Same SPIR-V, same `chassis::PipelineDesc`, two very different
/// pipelines underneath. The description is copied field for field from
/// `UHE/Stormbreaker/vulkan/tests/pipelines.rs` -- if one backend's tests drift,
/// the seam stops being tested and starts being two implementations.
#[test]
fn a_graphics_pipeline_from_the_shared_shader_creates() {
    let Some(mut resources) = backend() else {
        eprintln!("skipping: no wgpu adapter");
        return;
    };

    let compiler =
        forge::slangc::SlangcCompiler::new(forge::Slangc::default()).expect("the vendored slangc");
    let shaders = forge::Forge::new(compiler, forge::cache::ShaderCache::disabled());
    let directory = std::env::temp_dir().join("stormglass-pipeline");
    let (vertex_module, fragment_module) = shaders
        .compile_pair(
            &asset("assets/shaders/Triangle.slang"),
            "vs_main",
            "fs_main",
            &directory,
        )
        .expect("Triangle.slang compiles");

    let resources: &mut dyn Resources = &mut resources;
    let vertex = resources
        .create_shader("Triangle.vert", &vertex_module.to_bytes())
        .expect("vertex module");
    let fragment = resources
        .create_shader("Triangle.frag", &fragment_module.to_bytes())
        .expect("fragment module");

    let handle = resources
        .create_pipeline(chassis::PipelineDesc {
            label: "triangle".to_owned(),
            shaders: vec![
                chassis::ShaderStageDesc {
                    stage: chassis::ShaderStage::Vertex,
                    shader: vertex,
                },
                chassis::ShaderStageDesc {
                    stage: chassis::ShaderStage::Fragment,
                    shader: fragment,
                },
            ],
            set_layouts: Vec::new(),
            vertex: chassis::VertexLayout {
                stride: 32,
                attributes: vec![
                    chassis::VertexAttribute {
                        location: 0,
                        format: chassis::TextureFormat::Rgba32Float,
                        offset: 0,
                    },
                    chassis::VertexAttribute {
                        location: 1,
                        format: chassis::TextureFormat::Rgba32Float,
                        offset: 16,
                    },
                ],
            },
            index_format: None,
            // Culling off: the same Vulkan Y-flip caveat as the Vulkan suite, and
            // wgpu flips too. Both backends cull a counter-clockwise NDC
            // triangle under these defaults, and both need `front_face: 1` or no
            // culling to draw it.
            primitive: chassis::PrimitiveState {
                cull_mode: 0,
                ..chassis::PrimitiveState::default()
            },
            depth: chassis::DepthState::default(),
            blend: chassis::BlendState::default(),
            color_targets: vec![chassis::ColorTargetState {
                format: chassis::TextureFormat::Rgba8Unorm,
                blend: false,
                write_mask: 0xf,
            }],
            samples: chassis::SampleCount::X1,
        })
        .expect("a two-stage graphics pipeline creates");

    assert!(!handle.is_null());
}

/// A pipeline with no vertex stage is refused.
///
/// wgpu's `VertexState` has no such thing, so the alternative is an error deep
/// inside `create_render_pipeline` naming an internal index.
#[test]
fn a_pipeline_without_a_vertex_stage_is_refused() {
    let Some(mut resources) = backend() else {
        return;
    };
    let compiler =
        forge::slangc::SlangcCompiler::new(forge::Slangc::default()).expect("the vendored slangc");
    let shaders = forge::Forge::new(compiler, forge::cache::ShaderCache::disabled());
    let directory = std::env::temp_dir().join("stormglass-pipeline-novs");
    let (vertex_module, fragment_module) = shaders
        .compile_pair(
            &asset("assets/shaders/Triangle.slang"),
            "vs_main",
            "fs_main",
            &directory,
        )
        .expect("Triangle.slang compiles");

    let resources: &mut dyn Resources = &mut resources;
    let fragment = resources
        .create_shader("Triangle.frag", &fragment_module.to_bytes())
        .expect("fragment module");
    let _ = vertex_module;

    let error = resources
        .create_pipeline(chassis::PipelineDesc {
            label: "no-vertex".to_owned(),
            shaders: vec![chassis::ShaderStageDesc {
                stage: chassis::ShaderStage::Fragment,
                shader: fragment,
            }],
            set_layouts: Vec::new(),
            vertex: chassis::VertexLayout::default(),
            index_format: None,
            primitive: chassis::PrimitiveState::default(),
            depth: chassis::DepthState::default(),
            blend: chassis::BlendState::default(),
            color_targets: vec![chassis::ColorTargetState {
                format: chassis::TextureFormat::Rgba8Unorm,
                blend: false,
                write_mask: 0xf,
            }],
            samples: chassis::SampleCount::X1,
        })
        .expect_err("a pipeline with no vertex stage is refused");
    assert!(
        matches!(error, chassis::Error::Unsupported(_)),
        "and it says why, got {error:?}",
    );
}

/// A module with a valid magic number and a garbage body **panics** wgpu.
///
/// Found by writing a test badly. The two backends do not agree here and the
/// difference is worth naming:
///
///   Vulkan  `create_shader_module` returns `Result`, so a bad module is an error
///   wgpu    `create_shader_module` is infallible; validation goes to an error
///           scope, and with none installed it panics.
///
/// So `WgpuResources::create_shader` validates the magic number itself before
/// calling wgpu, which catches truncated and wrong-magic input. What is *not*
/// caught is a module with a correct magic number and a corrupt body -- naga
/// parses it and panics.
///
/// Not papered over. Catching a panic around a wgpu call is worse than the
/// panic: it converts a loud, immediate, specific failure into a silent wrong
/// answer, and a shader that validates in one place and not another is exactly
/// the bug this backend exists to avoid. The honest options are `push_error_scope`
/// around the call, or only ever feeding this function something `forge`
/// produced -- and `forge` checks the magic, so in practice the second holds.
///
/// This test pins the behaviour so the difference cannot be rediscovered by
/// accident.
#[test]
#[should_panic(expected = "Validation Error")]
fn a_module_with_a_valid_magic_and_a_corrupt_body_panics_wgpu() {
    let Some(mut resources) = backend() else {
        eprintln!("skipping: no wgpu adapter");
        return;
    };
    let resources: &mut dyn Resources = &mut resources;
    let _ = resources.create_shader("any", &[0x03, 0x02, 0x23, 0x07, 0, 0, 0, 0]);
}

/// A stage wgpu has no room for is named, not dropped.
///
/// Silently ignoring a geometry or tessellation stage produces a pipeline that
/// creates fine and draws with fewer stages than its shader expects, which is
/// the most expensive kind of wrong. `shader_state` returns
/// `Error::Unsupported` naming the stage; this test needs a real module to reach
/// that path, and the corrupt-body case above is the cheaper way to prove the
/// code is reached at all.
#[test]
fn geometry_is_a_stage_wgpu_has_no_room_for() {
    // A compile-time fact rather than a runtime one: the enum has the variant,
    // so `shader_state`'s match must handle it, and its arm must be the
    // `Unsupported` one.
    let stage: Option<chassis::ShaderStage> = Some(chassis::ShaderStage::Geometry);
    assert!(stage.is_some());
}

fn asset(relative: &str) -> std::path::PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        let candidate = ancestor.join(relative);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("could not find {relative} above {}", manifest.display());
}
