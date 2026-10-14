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

/// Pipelines and bind groups are not written yet, and saying so is the point.
#[test]
fn unwritten_paths_refuse_rather_than_pretending() {
    with_backend!(|resources| {
        let resources: &mut dyn Resources = &mut resources;

        let pipeline = resources.create_pipeline(chassis::PipelineDesc {
            label: "triangle".to_owned(),
            shaders: Vec::new(),
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
        });
        assert!(
            matches!(pipeline, Err(chassis::Error::Unsupported(_))),
            "an unwritten pipeline path must say so, got {pipeline:?}",
        );

        let group = resources.create_bind_group(&[0], &[]);
        assert!(
            matches!(group, Err(chassis::Error::Unsupported(_))),
            "bind groups need a declared layout on either backend",
        );
    });
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
