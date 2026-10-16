//! The renderer's decisions that need no device: the frame's graph, the cache
//! keys, the caches themselves through a recorded fake RHI, the shaders, and the
//! one call a frame makes into a backend.
//!
//! There is deliberately **no `#[ignore]`d GPU test here**. `bifrost` never
//! depends on a backend crate, so it cannot construct a real device; a
//! device-facing test belongs to the backend suites and to
//! `UHE/src/engine/tests/backend_parity.rs`, which exist for exactly that. What a
//! frame asks of a backend is `Gpu::record_frame`, whose arguments are pure data,
//! so that seam is tested here against a fake `Gpu` that records the call.

use std::sync::Arc;

use bifrost::material::{
    self, Material, MaterialKey, MaterialTextures, MaterialVariant, TextureCache,
};
use bifrost::mesh::{self, MeshCache, MeshKey, VertexLayoutKey};
use bifrost::passes::pipeline::{self, PipelineCache, PipelineKey, ShaderSet, ShaderVariant};
use bifrost::{Bifrost, EditorCamera, ViewSource};
use chassis::graph::types::{ErrorKind, PassDesc, Queue};
use chassis::graph::{self, ImageState, RenderGraph};
use chassis::{
    BindGroup, BindGroupTag, Buffer, BufferDesc, BufferTag, DescriptorKind,
    DescriptorSetLayoutDesc, ImageView, ImageViewTag, Pipeline, PipelineDesc, PipelineTag, Registry,
    Resources, Sampler, SamplerDesc, SamplerTag, Shader, ShaderTag, Texture, TextureDesc,
    TextureTag,
};
use vault::AssetId;

// ── a recorded fake RHI ─────────────────────────────────────────────────────

/// A `Resources` that records what it was asked for.
///
/// Handles are real generational handles from `chassis::Registry`, so a destroy
/// bumps the generation exactly as a backend's would and a re-creation is
/// observably a different handle — which is what the mesh-cache test asserts.
#[derive(Default)]
struct FakeResources {
    buffers: Registry<(), BufferTag>,
    textures: Registry<(), TextureTag>,
    views: Registry<(), ImageViewTag>,
    shaders: Registry<(), ShaderTag>,
    pipelines: Registry<(), PipelineTag>,
    groups: Registry<(), BindGroupTag>,
    samplers: Registry<(), SamplerTag>,

    buffer_creates: usize,
    buffer_destroys: Vec<Buffer>,
    texture_destroys: Vec<Texture>,
    /// Every `upload_texture` call: the texture, the byte count, and the extent
    /// it was told to fill.
    uploads: Vec<(Texture, usize, u32, u32)>,
    pipeline_creates: usize,
    group_destroys: usize,
    sampler_creates: usize,
}

impl FakeResources {
    fn new() -> Self {
        Self::default()
    }

    fn shader(&mut self) -> Shader {
        self.shaders.create(())
    }

    fn buffer(&mut self) -> Buffer {
        self.buffers.create(())
    }

    fn view(&mut self) -> ImageView {
        self.views.create(())
    }

    fn sampler(&mut self) -> Sampler {
        self.samplers.create(())
    }

    fn texture_handle(&mut self) -> Texture {
        self.textures.create(())
    }
}

impl Resources for FakeResources {
    fn create_buffer(
        &mut self,
        _desc: BufferDesc,
        _initial: Option<&[u8]>,
    ) -> chassis::Result<Buffer> {
        self.buffer_creates += 1;
        Ok(self.buffers.create(()))
    }

    fn create_texture(&mut self, _desc: TextureDesc) -> chassis::Result<Texture> {
        Ok(self.textures.create(()))
    }

    fn create_image_view(&mut self, _texture: &Texture) -> chassis::Result<ImageView> {
        Ok(self.views.create(()))
    }

    fn create_shader(&mut self, _label: &str, _spirv: &[u8]) -> chassis::Result<Shader> {
        Ok(self.shaders.create(()))
    }

    fn create_pipeline(&mut self, _desc: PipelineDesc) -> chassis::Result<Pipeline> {
        self.pipeline_creates += 1;
        Ok(self.pipelines.create(()))
    }

    fn create_bind_group(
        &mut self,
        _layout: &DescriptorSetLayoutDesc,
        _buffers: &[(u32, Buffer, u64)],
        _images: &[(u32, ImageView)],
        _samplers: &[(u32, Sampler)],
    ) -> chassis::Result<BindGroup> {
        Ok(self.groups.create(()))
    }

    fn create_sampler(&mut self, _desc: &SamplerDesc) -> chassis::Result<Sampler> {
        self.sampler_creates += 1;
        Ok(self.samplers.create(()))
    }

    fn destroy_buffer(&mut self, buffer: Buffer) {
        self.buffer_destroys.push(buffer);
        let _ = self.buffers.remove(buffer);
    }

    fn destroy_texture(&mut self, texture: Texture) {
        self.texture_destroys.push(texture);
        let _ = self.textures.remove(texture);
    }

    fn upload_texture(
        &mut self,
        texture: &Texture,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> chassis::Result<()> {
        self.uploads.push((*texture, data.len(), width, height));
        Ok(())
    }

    fn destroy_pipeline(&mut self, pipeline: Pipeline) {
        let _ = self.pipelines.remove(pipeline);
    }

    fn destroy_shader(&mut self, shader: Shader) {
        let _ = self.shaders.remove(shader);
    }

    fn destroy_bind_group(&mut self, group: BindGroup) {
        self.group_destroys += 1;
        let _ = self.groups.remove(group);
    }

    fn destroy_sampler(&mut self, sampler: Sampler) {
        let _ = self.samplers.remove(sampler);
    }
}

/// Eleven distinct shader handles, which is all a `ShaderSet` is.
fn shaders(resources: &mut FakeResources) -> ShaderSet {
    ShaderSet {
        model_vertex: resources.shader(),
        model_fragment: resources.shader(),
        depth_vertex: resources.shader(),
        ssao_vertex: resources.shader(),
        ssao_fragment: resources.shader(),
        grid_vertex: resources.shader(),
        grid_fragment: resources.shader(),
        line_vertex: resources.shader(),
        line_fragment: resources.shader(),
        composite_vertex: resources.shader(),
        composite_fragment: resources.shader(),
    }
}

fn asset(index: u32) -> AssetId {
    AssetId::from_parts(index, 1)
}

fn triangle() -> vault::MeshData {
    vault::MeshData {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        ..vault::MeshData::default()
    }
}

// ── the frame's graph ───────────────────────────────────────────────────────

/// The frame's shape, compiled with no device: a depth prepass, the geometry,
/// and the composition into an imported target.
///
/// The composition writes an *import*; that is what makes it a culling root, and
/// without a root `compiler::cull` drops the whole frame (`compiler.rs:296-330`).
#[test]
fn a_three_pass_frame_compiles_in_declaration_order() {
    let mut graph = RenderGraph::new();

    let backbuffer =
        graph.import_texture("backbuffer", ImageState::Undefined, ImageState::Present);
    let color = graph.import_texture("color", ImageState::Undefined, ImageState::ShaderRead);
    let depth = graph.import_texture("depth", ImageState::Undefined, ImageState::DepthAttachment);
    let shadow = graph.import_texture("shadow", ImageState::Undefined, ImageState::ShaderRead);

    graph.pass("shadow").depth(shadow, 1.0);
    graph
        .pass("opaque")
        .color_load(color)
        .depth(depth, 1.0)
        .read_texture(shadow);
    graph
        .pass("composite")
        .color(backbuffer, [0.0, 0.0, 0.0, 1.0])
        .read_texture(color);

    assert!(
        graph.validate().is_empty(),
        "a frame with no read-write conflict must validate: {}",
        graph.dump()
    );

    let compiled = graph.compile().expect("compiles");
    let order: Vec<&str> = compiled.passes.iter().map(|pass| pass.name.as_str()).collect();
    assert_eq!(order, ["shadow", "opaque", "composite"]);

    // Declaration order is already topological: a write binds the version it
    // produces and a read binds the latest, so the frame is a chain.
    assert_eq!(compiled.find("opaque").map(|pass| pass.index), Some(1));
    assert_eq!(compiled.passes[0].queue, Queue::Graphics);

    // The prepass owns the transition into the shadow map it writes, which is
    // what lets every pass record on its own with no barrier API in sight.
    assert!(
        !compiled.passes[0].pre_barriers.is_empty(),
        "the first writer of a texture must transition it"
    );
}

/// A cycle is refused, and it takes hand-written pass descriptions to build one:
/// the fluent builder binds every read to the version that exists when it is
/// declared, so its declaration order is already a topological order.
#[test]
fn a_cycle_is_refused_rather_than_ordered() {
    let mut resources = chassis::graph::resources::Resources::new();
    let a = resources.create_texture("a", ImageState::Undefined, ImageState::ShaderRead);
    let b = resources.create_texture("b", ImageState::Undefined, ImageState::ShaderRead);

    // Version 1 of `a` is produced by pass 1, and version 1 of `b` by pass 0.
    resources.push_version(
        chassis::graph::ResourceAccess::Texture(a, 0),
        1,
        ImageState::ShaderRead,
    );
    resources.push_version(
        chassis::graph::ResourceAccess::Texture(b, 0),
        0,
        ImageState::ShaderRead,
    );

    let passes = vec![
        PassDesc {
            name: "a".to_owned(),
            reads: vec![chassis::graph::ResourceAccess::Texture(a, 1)],
            ..PassDesc::default()
        },
        PassDesc {
            name: "b".to_owned(),
            reads: vec![chassis::graph::ResourceAccess::Texture(b, 1)],
            ..PassDesc::default()
        },
    ];

    // Both passes are individually valid: they read versions that exist and
    // write nothing, so only the ordering can refuse them.
    assert!(graph::validate(&passes, &resources).is_empty());

    match graph::compile(&passes, &resources) {
        Ok(_) => panic!("two passes reading each other's output is a cycle"),
        Err(errors) => {
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert_eq!(errors[0].kind, ErrorKind::Cycle);
        }
    }
}

/// A pass cannot read and write one texture: the C++'s `PassStateConflict`, and
/// the reason the grid and the gizmo lines are one pass here rather than two
/// colour-writing passes.
#[test]
fn a_pass_that_reads_what_it_writes_is_refused() {
    let mut graph = RenderGraph::new();
    let color = graph.import_texture("color", ImageState::Undefined, ImageState::ShaderRead);
    graph
        .pass("grid")
        .color_load(color)
        .read_texture(color);

    let errors = graph.validate();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind, ErrorKind::PassStateConflict);
}

// ── pipelines ───────────────────────────────────────────────────────────────

/// Every field that changes the pipeline is in the key, and the four model
/// variants differ from one another.
#[test]
fn a_pipeline_key_covers_the_state_a_pipeline_is_built_from() {
    use chassis::{SampleCount, TextureFormat};

    let model = PipelineKey::model(
        ShaderVariant::Model,
        TextureFormat::Rgba8Srgb,
        TextureFormat::Depth32Float,
        SampleCount::X1,
    );
    let same = PipelineKey::model(
        ShaderVariant::Model,
        TextureFormat::Rgba8Srgb,
        TextureFormat::Depth32Float,
        SampleCount::X1,
    );
    let double_sided = PipelineKey::model(
        ShaderVariant::ModelDoubleSided,
        TextureFormat::Rgba8Srgb,
        TextureFormat::Depth32Float,
        SampleCount::X1,
    );
    let other_format = PipelineKey::model(
        ShaderVariant::Model,
        TextureFormat::Rgba8Unorm,
        TextureFormat::Depth32Float,
        SampleCount::X1,
    );

    assert_eq!(model, same);
    assert_ne!(model, double_sided);
    assert_ne!(model, other_format);
    // The shadow key shares the vertex layout, so a mesh is one upload for both
    // passes, and differs in everything else.
    assert_ne!(
        model,
        PipelineKey::shadow(TextureFormat::Depth32Float, SampleCount::X1, &mesh::vertex_layout())
    );
}

/// The description a key produces: which stages run, what the state is, and how
/// many sets the pipeline declares.
#[test]
fn a_pipeline_description_matches_its_key() {
    use chassis::{SampleCount, ShaderStage, TextureFormat};

    let mut fake = FakeResources::new();
    let shaders = shaders(&mut fake);

    let model = pipeline::pipeline_desc(
        &PipelineKey::model(
            ShaderVariant::Model,
            TextureFormat::Rgba8Srgb,
            TextureFormat::Depth32Float,
            SampleCount::X1,
        ),
        &shaders,
    );
    assert_eq!(model.shaders.len(), 2, "vertex and fragment");
    assert_eq!(model.shaders[0].stage, ShaderStage::Vertex);
    assert_eq!(model.set_layouts.len(), 3, "frame, material, draw");
    assert!(!model.blend.enabled);
    assert_eq!(model.primitive.cull_mode, 1, "back-face culled");
    assert_eq!(model.depth.format, Some(TextureFormat::Depth32Float));
    assert!(model.depth.write_enabled);
    assert_eq!(model.color_targets.len(), 1);

    // The double-sided variant is the same pipeline with culling off, which is
    // exactly what `Renderer3D::Init` builds a second one for.
    let double_sided = pipeline::pipeline_desc(
        &PipelineKey::model(
            ShaderVariant::ModelDoubleSided,
            TextureFormat::Rgba8Srgb,
            TextureFormat::Depth32Float,
            SampleCount::X1,
        ),
        &shaders,
    );
    assert_eq!(double_sided.primitive.cull_mode, 0);
    assert_eq!(double_sided.shaders, model.shaders);

    // Blending is pipeline state, so the transparent variant turns it on.
    let transparent = pipeline::pipeline_desc(
        &PipelineKey::model(
            ShaderVariant::ModelTransparent,
            TextureFormat::Rgba8Srgb,
            TextureFormat::Depth32Float,
            SampleCount::X1,
        ),
        &shaders,
    );
    assert!(transparent.blend.enabled);

    // A depth-only prepass: one stage, no colour target.
    let shadow = pipeline::pipeline_desc(
        &PipelineKey::shadow(
            TextureFormat::Depth32Float,
            SampleCount::X1,
            &mesh::vertex_layout(),
        ),
        &shaders,
    );
    assert_eq!(shadow.shaders.len(), 1);
    assert!(shadow.color_targets.is_empty());
    assert_eq!(shadow.set_layouts.len(), 2, "shadow, draw");

    // A full-screen pass has no vertex buffer at all.
    let ssao = pipeline::pipeline_desc(
        &PipelineKey::fullscreen(
            ShaderVariant::Ssao,
            TextureFormat::R8Unorm,
            SampleCount::X1,
        ),
        &shaders,
    );
    assert_eq!(ssao.vertex.stride, 0);
    assert!(ssao.vertex.attributes.is_empty());
    assert!(ssao.index_format.is_none(), "drawn with no index buffer");
}

/// One pipeline per key, created once, destroyed on request.
#[test]
fn the_pipeline_cache_creates_one_pipeline_per_key() {
    use chassis::{SampleCount, TextureFormat};

    let mut fake = FakeResources::new();
    let shaders = shaders(&mut fake);
    let mut cache = PipelineCache::new();

    let key = PipelineKey::model(
        ShaderVariant::Model,
        TextureFormat::Rgba8Srgb,
        TextureFormat::Depth32Float,
        SampleCount::X1,
    );
    let first = cache
        .get_or_create(&mut fake, key.clone(), &shaders)
        .expect("creates");
    let second = cache
        .get_or_create(&mut fake, key.clone(), &shaders)
        .expect("hit");
    assert_eq!(first, second);
    assert_eq!(fake.pipeline_creates, 1, "a hit must not create anything");
    assert_eq!(cache.len(), 1);

    let other = PipelineKey::grid(TextureFormat::Rgba8Srgb, SampleCount::X1);
    assert_ne!(
        cache.get_or_create(&mut fake, other, &shaders).expect("creates"),
        first
    );
    assert_eq!(fake.pipeline_creates, 2);

    assert!(cache.destroy(&mut fake, &key));
    assert!(!cache.destroy(&mut fake, &key), "destroying twice is not a destroy");
    assert_eq!(cache.len(), 1);
}

// ── materials ───────────────────────────────────────────────────────────────

/// A material key is the textures it binds and every value the shader reads, so
/// two materials that differ anywhere are two entries.
#[test]
fn a_material_key_covers_textures_and_factors() {
    let mut fake = FakeResources::new();
    let texture = fake.texture_handle();

    let textures = MaterialTextures {
        albedo: Some(texture),
        ..MaterialTextures::default()
    };
    let material = Material::default();
    let key = MaterialKey::of(&material, textures, MaterialVariant::Opaque);

    assert_eq!(
        key,
        MaterialKey::of(&Material::default(), textures, MaterialVariant::Opaque)
    );

    let rough = Material {
        roughness_factor: 0.5,
        ..material
    };
    assert_ne!(
        key,
        MaterialKey::of(&rough, textures, MaterialVariant::Opaque),
        "a factor the shader reads is part of the key"
    );

    let white = MaterialTextures {
        albedo: Some(texture),
        normal: Some(texture),
        ..MaterialTextures::default()
    };
    assert_ne!(
        key,
        MaterialKey::of(&material, white, MaterialVariant::Opaque),
        "so is a texture"
    );

    assert_ne!(
        key,
        MaterialKey::of(&material, textures, MaterialVariant::Transparent),
        "and the variant, because culling and blending are pipeline state"
    );
}

/// A depth texture is declared as one, not as a sampled colour image.
///
/// Two of the renderer's sets read a `texture_depth_2d`: the frame set's shadow
/// map and the SSAO set's depth buffer. wgpu maps a depth view to
/// `TextureSampleType::Depth` and refuses a bind group whose view does not match
/// its layout's sample type by name, so declaring either `SampledImage` would make
/// the default backend fail at the frame's first bind group. Vulkan takes one
/// `SAMPLED_IMAGE` descriptor for both, which is why this went unnoticed there.
#[test]
fn the_two_depth_reads_are_declared_as_depth() {
    let kind_of = |layout: &DescriptorSetLayoutDesc, binding: u32| {
        layout
            .bindings
            .iter()
            .find(|b| b.binding == binding)
            .unwrap_or_else(|| panic!("binding {binding} of {}", layout.label))
            .kind
    };

    assert_eq!(
        kind_of(&pipeline::frame_layout(), 2),
        DescriptorKind::SampledDepth,
        "the shadow map, at the binding the frame's set is built with",
    );
    assert_eq!(
        kind_of(&pipeline::ssao_layout(), 1),
        DescriptorKind::SampledDepth,
        "the depth buffer SSAO reads",
    );

    // Nothing else samples depth: the sets that read a render target read colour
    // textures, and the rest are uniform buffers and samplers.
    for layout in [pipeline::material_layout(), pipeline::composite_layout()] {
        assert!(
            layout
                .bindings
                .iter()
                .all(|b| b.kind != DescriptorKind::SampledDepth),
            "{} declares no depth texture",
            layout.label,
        );
    }
    assert_eq!(
        kind_of(&pipeline::composite_layout(), 0),
        DescriptorKind::SampledImage,
        "the scene colour stays a sampled image",
    );
}

/// `FillMaterialGPU` puts every value in the slot the shader reads it from.
#[test]
fn a_material_packs_into_the_shader_layout() {
    let mut uv_transforms = [material::UVTransform::IDENTITY; material::SLOT_COUNT];
    uv_transforms[material::SLOT_ALBEDO] = material::UVTransform {
        cos_rotation: 0.0,
        sin_rotation: 1.0,
        scale: [2.0, 3.0],
        offset: [0.5, 0.25],
    };
    let material = Material {
        base_color_factor: [0.25, 0.5, 0.75, 1.0],
        metallic_factor: 0.9,
        roughness_factor: 0.1,
        normal_scale: 2.0,
        occlusion_strength: 0.5,
        emissive_factor: [1.0, 0.0, 0.0],
        emissive_strength: 3.0,
        alpha: material::AlphaMode::Mask,
        alpha_cutoff: 0.25,
        unlit: true,
        uv_transforms,
        ..Material::default()
    };

    let gpu = material::fill_gpu(&material);

    assert_eq!(gpu.base_color_factor, [0.25, 0.5, 0.75, 1.0]);
    assert_eq!(gpu.emissive_factor_strength, [1.0, 0.0, 0.0, 3.0]);
    assert_eq!(gpu.factors, [0.9, 0.1, 2.0, 0.5]);
    assert_eq!(gpu.params, [0.25, 1.0, 1.0, 0.0]);
    assert_eq!(gpu.uv_transform[material::SLOT_ALBEDO], [0.0, 1.0, 2.0, 3.0]);
    assert_eq!(gpu.uv_offset[material::SLOT_ALBEDO], [0.5, 0.25, 0.0, 0.0]);
    // A slot with no transform is the identity, so an absent
    // KHR_texture_transform is not a zero scale.
    assert_eq!(gpu.uv_transform[material::SLOT_EMISSIVE], [1.0, 0.0, 1.0, 1.0]);
}

/// A material is created once per key and is destroyed with its texture: a bind
/// group naming a destroyed texture is a descriptor pointing at freed memory.
#[test]
fn destroying_a_texture_drops_every_material_that_bound_it() {
    let mut fake = FakeResources::new();
    let texture = fake.texture_handle();
    let view = fake.view();
    let sampler = fake.sampler();

    let textures = MaterialTextures {
        albedo: Some(texture),
        ..MaterialTextures::default()
    };
    let key = MaterialKey::of(&Material::default(), textures, MaterialVariant::Opaque);
    let layout = pipeline::material_layout();

    let mut cache = MaterialCache::new();
    let first = cache
        .get_or_insert(
            &mut fake,
            key,
            &Material::default(),
            &[(2, view)],
            &[(1, sampler)],
            &layout,
        )
        .expect("creates");
    let second = cache
        .get_or_insert(
            &mut fake,
            key,
            &Material::default(),
            &[(2, view)],
            &[(1, sampler)],
            &layout,
        )
        .expect("hit");
    assert_eq!(first.group, second.group);
    assert_eq!(cache.len(), 1);

    assert_eq!(cache.invalidate_texture(&mut fake, texture), 1);
    assert!(cache.is_empty());
    assert_eq!(fake.group_destroys, 1);
    assert_eq!(fake.buffer_destroys.len(), 1, "the parameter buffer went too");

    assert_eq!(
        cache.invalidate_texture(&mut fake, texture),
        0,
        "already gone"
    );
}

// ── textures ────────────────────────────────────────────────────────────────

/// A texture's bytes reach the GPU through `Resources::upload_texture`.
///
/// This is the test that would have caught the version that set
/// `TextureDesc::initial_data` instead: `stormglass` never reads that field, so
/// the texture would have been created empty and every material would have
/// sampled black on the default backend, with no error anywhere. Asserting on
/// the call — its handle, its byte count and its extent — is what makes the
/// difference observable without a device.
#[test]
fn a_texture_upload_goes_through_upload_texture_with_the_image_bytes() {
    let mut fake = FakeResources::new();
    let mut cache = TextureCache::new();
    let image = vault::Image {
        width: 2,
        height: 1,
        rgba8: vec![1, 2, 3, 4, 5, 6, 7, 8],
        srgb: true,
    };

    let first = cache
        .get_or_upload(&mut fake, asset(3), &image)
        .expect("uploads");
    assert_eq!(fake.uploads.len(), 1, "one upload, and it is issued");
    let (handle, bytes, width, height) = fake.uploads[0];
    assert_eq!(handle, first.texture, "the bytes go into the texture it made");
    assert_eq!(bytes, image.rgba8.len(), "all of them");
    assert_eq!((width, height), (image.width, image.height));

    // The second call for one asset is a cache hit, so no bytes move again.
    let hit = cache
        .get_or_upload(&mut fake, asset(3), &image)
        .expect("hit");
    assert_eq!(hit.texture, first.texture);
    assert_eq!(fake.uploads.len(), 1, "a hit uploads nothing");

    // And a destroyed texture is uploaded again rather than handed back stale.
    assert!(cache.destroy(&mut fake, asset(3)).is_some());
    assert_eq!(fake.texture_destroys.len(), 1);
    let again = cache
        .get_or_upload(&mut fake, asset(3), &image)
        .expect("re-uploads");
    assert_eq!(fake.uploads.len(), 2);
    assert_ne!(
        again.texture, first.texture,
        "a generation bump, not the old handle"
    );
    assert_eq!(again.sampler, first.sampler, "the sampler is shared, not re-made");
    assert_eq!(fake.sampler_creates, 1);
}

/// The 1x1 stand-ins a material with no map samples: white for the factors, and
/// `(0.5, 0.5, 1)` for a normal map, which is not white.
#[test]
fn the_neutral_maps_are_one_texel_and_the_normal_is_not_white() {
    let mut fake = FakeResources::new();
    let mut cache = TextureCache::new();

    let white = cache.white(&mut fake).expect("white");
    let neutral = cache.neutral_normal(&mut fake).expect("neutral");
    assert_eq!(fake.uploads.len(), 2);
    assert_eq!(fake.uploads[0].1, 4, "one RGBA texel");
    assert_eq!(fake.uploads[0].2, 1);

    // Same call twice is the same texture, not a second upload.
    assert_eq!(cache.white(&mut fake).expect("white").texture, white.texture);
    assert_eq!(fake.uploads.len(), 2);

    assert_ne!(white.texture, neutral.texture);
    assert_eq!(white.sampler, neutral.sampler);
}

// ── meshes ──────────────────────────────────────────────────────────────────

/// The same lookup twice is one upload, and a destroyed mesh is uploaded again
/// rather than handed back as a stale handle.
#[test]
fn a_mesh_cache_returns_one_handle_and_reuploads_after_a_destroy() {
    let mut fake = FakeResources::new();
    let mut cache = MeshCache::new();
    let key = MeshKey::new(asset(7), VertexLayoutKey::of(&mesh::vertex_layout()));
    let data = triangle();

    let first = cache
        .get_or_upload(&mut fake, key, &data)
        .expect("uploads");
    assert_eq!(first.index_count, 3);
    assert_eq!(first.vertex_count, 3);
    assert_eq!(fake.buffer_creates, 2, "one vertex buffer and one index");

    let second = cache.get_or_upload(&mut fake, key, &data).expect("hit");
    assert_eq!(first, second);
    assert_eq!(fake.buffer_creates, 2, "a hit uploads nothing");

    assert!(cache.destroy(&mut fake, &key));
    assert_eq!(fake.buffer_destroys.len(), 2);
    assert!(!cache.destroy(&mut fake, &key));

    let third = cache
        .get_or_upload(&mut fake, key, &data)
        .expect("uploads again");
    assert_eq!(fake.buffer_creates, 4);
    // A generation bump, not the same handle: a handle that outlived its
    // resource is what the generational registry exists to catch.
    assert_ne!(third.vertex, first.vertex);
    assert_eq!(third.index_count, first.index_count);
}

/// Two meshes with different vertex layouts never share a buffer, and the key
/// says so.
#[test]
fn a_mesh_key_includes_the_vertex_layout() {
    let layout = mesh::vertex_layout();
    let same = VertexLayoutKey::of(&layout);
    assert_eq!(same, VertexLayoutKey::of(&mesh::vertex_layout()));

    let mut other = layout.clone();
    other.stride += 4;
    assert_ne!(same, VertexLayoutKey::of(&other));

    assert_ne!(
        MeshKey::new(asset(1), same),
        MeshKey::new(asset(2), same),
        "and the asset"
    );
}

/// Interleaving fills a missing attribute with the C++ default rather than
/// dropping the vertex, and an absent normal map's neutral is not white.
#[test]
fn interleaving_uses_the_defaults_for_absent_attributes() {
    let data = vault::MeshData {
        normals: vec![[0.0, 1.0, 0.0]],
        colors: vec![[1.0, 0.0, 0.0, 1.0]],
        ..triangle()
    };

    let vertices = mesh::interleave(&data);
    assert_eq!(vertices.len(), 3);
    assert_eq!(vertices[0].normal, [0.0, 1.0, 0.0]);
    assert_eq!(vertices[0].color, [1.0, 0.0, 0.0, 1.0]);

    assert_eq!(vertices[1].normal, [0.0, 0.0, 1.0], "the C++ default");
    assert_eq!(vertices[1].uv, [0.0, 0.0]);
    assert_eq!(vertices[1].tangent, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(vertices[1].color, [1.0; 4], "white, so the multiply is neutral");

    // The layout's offsets are the struct's, computed rather than written down.
    let layout = mesh::vertex_layout();
    assert_eq!(layout.stride as usize, std::mem::size_of::<mesh::Vertex>());
    assert_eq!(layout.attributes.len(), 5);
    assert_eq!(
        layout.attributes[0].offset,
        std::mem::offset_of!(mesh::Vertex, position) as u32
    );
}

// ── shaders ─────────────────────────────────────────────────────────────────

/// Every embedded shader parses, validates, and compiles to SPIR-V for each
/// entry point it declares.
///
/// `forge::wgsl` validates with naga's validator, which is the same rule set
/// wgpu enforces at shader-module creation, so a shader that passes here cannot
/// fail on either backend for a *shader* reason. Running it means the renderer's
/// six shaders are checked without a device — which is the point of compiling
/// WGSL in process instead of shelling out to Slang.
#[test]
fn every_embedded_shader_compiles_for_its_entry_points() {
    let shaders = [
        ("model", pipeline::MODEL_WGSL),
        ("depth", pipeline::DEPTH_WGSL),
        ("ssao", pipeline::SSAO_WGSL),
        ("grid", pipeline::GRID_WGSL),
        ("line", pipeline::LINE_WGSL),
        ("composite", pipeline::COMPOSITE_WGSL),
    ];

    for (label, source) in shaders {
        forge::wgsl::check(source, label).unwrap_or_else(|error| panic!("{label}: {error}"));

        let entries = forge::wgsl::entry_points(source, label)
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        assert!(
            !entries.is_empty(),
            "{label} declares no entry point, so nothing would be compiled"
        );

        for (entry, stage) in entries {
            let spirv = forge::wgsl::to_spirv_entry(source, label, &entry, stage)
                .unwrap_or_else(|error| panic!("{label}/{entry}: {error}"));
            assert_eq!(
                spirv.len() % 4,
                0,
                "{label}/{entry}: SPIR-V is a word stream"
            );
            assert!(!spirv.is_empty());
        }
    }

    // Every graphics shader the vertex/fragment pairs are built from declares
    // the two entry points the pipelines ask for by name.
    for (label, source) in [
        ("model", pipeline::MODEL_WGSL),
        ("ssao", pipeline::SSAO_WGSL),
        ("grid", pipeline::GRID_WGSL),
        ("line", pipeline::LINE_WGSL),
        ("composite", pipeline::COMPOSITE_WGSL),
    ] {
        let entries = forge::wgsl::entry_points(source, label).expect("parses");
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&pipeline::VS_MAIN), "{label}: {names:?}");
        assert!(names.contains(&pipeline::FS_MAIN), "{label}: {names:?}");
    }
}

// ── the frame loop's one call ───────────────────────────────────────────────

/// A `Gpu` that records what a frame handed it, with no device behind it.
///
/// `record_frame` takes pure data -- a compiled graph, a slot table, a recorder --
/// so the hand-off is testable without a driver. What the passes *mean* is the
/// backends' business; what matters here is that a frame with passes reaches one,
/// and that a refusal comes back to the caller rather than being logged away.
struct FakeGpu {
    resources: FakeResources,
    info: chassis::GpuInfo,
    graph: FakeGraph,
    frames: usize,
    passes: usize,
    slots: usize,
}

impl FakeGpu {
    fn new() -> Self {
        Self {
            resources: FakeResources::new(),
            info: chassis::GpuInfo {
                name: "fake".to_owned(),
                sync_tier: chassis::SyncTier::Legacy,
                max_texture_size: 8192,
                frames_in_flight: 1,
            },
            graph: FakeGraph,
            frames: 0,
            passes: 0,
            slots: 0,
        }
    }
}

impl chassis::Gpu for FakeGpu {
    fn info(&self) -> &chassis::GpuInfo {
        &self.info
    }

    fn swapchain(&self) -> Option<&dyn chassis::Swapchain> {
        None
    }

    fn graph(&mut self) -> &mut dyn chassis::RenderGraph {
        &mut self.graph
    }

    fn resources(&mut self) -> &mut dyn Resources {
        &mut self.resources
    }

    fn record_frame(
        &mut self,
        frame: &graph::CompiledGraph,
        slots: &graph::SlotTable,
        _recorder: Arc<dyn graph::PassRecorder>,
    ) -> chassis::Result<()> {
        self.frames += 1;
        self.passes = frame.passes.len();
        self.slots = slots.texture_count();
        Ok(())
    }

    fn frame_begin(&mut self) -> chassis::Result<()> {
        Ok(())
    }

    fn submit(&mut self) -> chassis::Result<()> {
        Ok(())
    }

    fn wait_idle(&mut self) {}
}

/// The `RenderGraph` trait, which a frame never declares through.
///
/// The frame declares into the fluent builder (`chassis::graph::RenderGraph`);
/// this exists only because `Gpu::graph` hands one out.
#[derive(Default)]
struct FakeGraph;

impl chassis::RenderGraph for FakeGraph {
    fn reset(&mut self) {}

    fn add_pass(&mut self, _desc: chassis::RenderPassDesc) -> chassis::PassId {
        0
    }

    fn add_dependency(&mut self, _producer: chassis::PassId, _consumer: chassis::PassId) {}

    fn compile(&mut self) -> chassis::Result<Vec<chassis::PassId>> {
        Ok(Vec::new())
    }

    fn execute(&mut self, _jobs: &grind::Jobs) -> chassis::Result<()> {
        Ok(())
    }
}

/// A `Gpu` that leaves `record_frame` at its default: what a backend that has not
/// adopted the method runs. The refusal is the difference between a frame that
/// fails loudly and a frame that quietly did nothing.
struct NoFrameGpu {
    resources: FakeResources,
    info: chassis::GpuInfo,
    graph: FakeGraph,
}

impl NoFrameGpu {
    fn new() -> Self {
        Self {
            resources: FakeResources::new(),
            info: chassis::GpuInfo {
                name: "no-frame".to_owned(),
                sync_tier: chassis::SyncTier::Legacy,
                max_texture_size: 8192,
                frames_in_flight: 1,
            },
            graph: FakeGraph,
        }
    }
}

impl chassis::Gpu for NoFrameGpu {
    fn info(&self) -> &chassis::GpuInfo {
        &self.info
    }

    fn swapchain(&self) -> Option<&dyn chassis::Swapchain> {
        None
    }

    fn graph(&mut self) -> &mut dyn chassis::RenderGraph {
        &mut self.graph
    }

    fn resources(&mut self) -> &mut dyn Resources {
        &mut self.resources
    }

    fn frame_begin(&mut self) -> chassis::Result<()> {
        Ok(())
    }

    fn submit(&mut self) -> chassis::Result<()> {
        Ok(())
    }

    fn wait_idle(&mut self) {}
}

fn renderer_for(gpu: &mut dyn chassis::Gpu) -> Bifrost {
    let mut assets = vault::Assets::new(".");
    Bifrost::new(&mut *gpu, &mut assets).expect("the frame's own resources need no device")
}

fn editor_camera() -> EditorCamera {
    EditorCamera::new(45.0, 16.0 / 9.0, 0.1, 1000.0)
}

/// A frame with passes reaches `record_frame`, once, carrying the slots the
/// renderer minted for them.
#[test]
fn a_frame_with_passes_reaches_record_frame() {
    let mut gpu = FakeGpu::new();
    let mut renderer = renderer_for(&mut gpu);
    let jobs = grind::Jobs::new();
    let scene = meats::Scene::new();
    let camera = editor_camera();

    renderer
        .render(&mut gpu, &jobs, &scene, ViewSource::Editor, Some(&camera))
        .expect("a fake device takes the whole frame");

    assert_eq!(gpu.frames, 1, "one frame, one call");
    assert!(
        gpu.passes >= 5,
        "the frame's passes reached the backend, not just the graph: {}",
        gpu.passes
    );
    assert!(
        gpu.slots >= 4,
        "the handles the renderer minted reached the backend: {}",
        gpu.slots
    );
}

/// A backend that cannot execute a frontend frame is an error out of `render`,
/// which is the branch the renderer used to swallow with a warning.
#[test]
fn an_unsupported_backend_surfaces_from_render() {
    let mut gpu = NoFrameGpu::new();
    let mut renderer = renderer_for(&mut gpu);
    let jobs = grind::Jobs::new();
    let scene = meats::Scene::new();
    let camera = editor_camera();

    let error = renderer
        .render(&mut gpu, &jobs, &scene, ViewSource::Editor, Some(&camera))
        .expect_err("the trait's default refuses a frontend frame");
    assert!(
        matches!(error, chassis::Error::Unsupported(_)),
        "a refusal, named: {error:?}"
    );
}
