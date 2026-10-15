//! The renderer frontend: the frame, and everything the frame owns.
//!
//! Port of `UHE/Renderer3D/Renderer3D.cpp` — init, scene begin, submission —
//! with the C++'s file-static `s_Data3D` replaced by fields on [`Bifrost`].
//! The spine's failure table names that struct first: nothing could be
//! constructed in isolation while it was a static, so `tests/scene` had to
//! recompile engine sources to have a renderer at all.
//!
//! ## What a frame does
//!
//! 1. Resolve the camera ([`ViewSource`]) or take the editor camera.
//! 2. Declare the frame's passes into its own render graph — reads and writes
//!    only, because the graph owns every barrier (`chassis::graph`).
//! 3. Compile the graph, prepare the uniform sets and the draw list, and hand
//!    the compiled frame plus one [`PassRecorder`] to the backend.
//!
//! ## Who runs the frame
//!
//! `chassis::Gpu::record_frame`: the renderer declares the frame, then hands the
//! compiled graph, the slot table its textures were minted into, and one
//! [`PassRecorder`] to the backend. Recording, submitting and presenting stay the
//! backend's -- it owns the executor, the command lists and the swapchain, and a
//! frontend that recorded its own frame would have to name a driver to do it.

use std::collections::HashMap;
use std::sync::Arc;

use chassis::graph::{CompiledGraph, ImageState, PassRecorder, RenderGraph, SlotTable};
use chassis::{
    BindGroup, BufferDesc, BufferUsage, Error, Extent, Extent2d, Gpu, ImageView, Resources, Sampler,
    SamplerDesc, Texture, TextureDesc, TextureFormat, TextureUsage,
};
use glam::{Mat4, Vec3};

use crate::camera::{CameraInput, EditorCamera};
use crate::light::{self, LightBuffer, LightData};
use crate::material::{
    self, CachedTexture, Material, MaterialBinding, MaterialCache, MaterialKey, MaterialTextures,
    MaterialVariant, TextureCache,
};
use crate::mesh::{self, GpuMesh, MeshCache, MeshKey, VertexLayoutKey};
use crate::passes::{
    self, uniforms, CompositePass, DrawArena, EditorPass, FrameRecorder, FrameTargets,
    GeometryPass, GridPass, GridUniforms, ImageSet, LineBatch, LinePass, LineUniforms,
    MaterialPipelines, OverlayPasses, PipelineCache, PipelineKey, PreparedDraw, ShadowPass,
    ShaderSet, ShaderVariant, SsaoPass, SsaoUniforms,
};

/// Where the frame's camera comes from.
///
/// The C++ has two `BeginScene` overloads — one taking the editor camera, one
/// taking a scene camera plus its transform — and this is that choice as a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewSource {
    /// A scene camera entity: its `Camera` component's projection, and the
    /// entity's world transform as the view.
    Scene(meats::Entity),
    /// The editor's orbit camera, passed alongside this to `render`.
    Editor,
}

/// The shadow map's edge, in texels.
pub const SHADOW_MAP_SIZE: u32 = 2048;

/// Half the extent of the directional light's orthographic box, in world units.
///
/// The C++ has no shadow map, so this is the one number a shadow needs that
/// nothing in the tree suggests.
pub const SHADOW_EXTENT: f32 = 40.0;

/// The clear colour, which the C++'s editor theme supplies.
pub const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.12, 1.0];

/// The frame's own textures, recreated when the extent changes.
struct FrameTextures {
    color: Texture,
    color_view: ImageView,
    depth: Texture,
    depth_view: ImageView,
    shadow: Texture,
    shadow_view: ImageView,
    ssao: Texture,
    ssao_view: ImageView,
    /// Clamped linear: a render target's edge should clamp, not wrap.
    sampler: Sampler,
}

impl FrameTextures {
    /// Null handles, so a live set can be swapped out and destroyed. Same trick
    /// as a graph slot with no resource behind it.
    fn null() -> Self {
        Self {
            color: Texture::null(),
            color_view: ImageView::null(),
            depth: Texture::null(),
            depth_view: ImageView::null(),
            shadow: Texture::null(),
            shadow_view: ImageView::null(),
            ssao: Texture::null(),
            ssao_view: ImageView::null(),
            sampler: Sampler::null(),
        }
    }

    fn create(
        resources: &mut dyn Resources,
        extent: Extent2d,
        shadow_size: u32,
    ) -> Result<Self, Error> {
        let width = extent.width.max(1);
        let height = extent.height.max(1);

        let color = resources.create_texture(TextureDesc::new_2d(
            Extent::new(width, height),
            // The C++ model pipeline's first colour format is `RGBA8_SRGB`.
            TextureFormat::Rgba8Srgb,
            TextureUsage::COLOR_ATTACHMENT | TextureUsage::SAMPLED,
        ))?;
        let color_view = resources.create_image_view(&color)?;

        // Depth32Float rather than the C++'s `D24_UNORM_S8`: SSAO samples the
        // depth buffer, and a depth-stencil format is not sampleable on wgpu.
        let depth = resources.create_texture(TextureDesc::new_2d(
            Extent::new(width, height),
            TextureFormat::Depth32Float,
            TextureUsage::DEPTH_ATTACHMENT | TextureUsage::SAMPLED,
        ))?;
        let depth_view = resources.create_image_view(&depth)?;

        let shadow = resources.create_texture(TextureDesc::new_2d(
            Extent::new(shadow_size, shadow_size),
            TextureFormat::Depth32Float,
            TextureUsage::DEPTH_ATTACHMENT | TextureUsage::SAMPLED,
        ))?;
        let shadow_view = resources.create_image_view(&shadow)?;

        let ssao = resources.create_texture(TextureDesc::new_2d(
            Extent::new(width, height),
            TextureFormat::R8Unorm,
            TextureUsage::COLOR_ATTACHMENT | TextureUsage::SAMPLED,
        ))?;
        let ssao_view = resources.create_image_view(&ssao)?;

        let sampler = resources.create_sampler(&SamplerDesc::clamped())?;

        Ok(Self {
            color,
            color_view,
            depth,
            depth_view,
            shadow,
            shadow_view,
            ssao,
            ssao_view,
            sampler,
        })
    }

    fn destroy(self, resources: &mut dyn Resources) {
        resources.destroy_texture(self.color);
        resources.destroy_texture(self.depth);
        resources.destroy_texture(self.shadow);
        resources.destroy_texture(self.ssao);
        resources.destroy_sampler(self.sampler);
    }
}

/// The renderer frontend. Owned by `BifrostPlugin`, never by `Engine`.
pub struct Bifrost {
    extent: Extent2d,
    swapchain_format: TextureFormat,
    textures: FrameTextures,

    shaders: ShaderSet,
    pipelines: PipelineCache,
    meshes: MeshCache,
    materials: MaterialCache,
    textures_by_asset: TextureCache,
    /// Material descriptions by asset, for `MeshSample::material` to resolve.
    material_assets: HashMap<vault::AssetId, Material>,
    lights: LightBuffer,

    graph: RenderGraph,
    targets: Option<FrameTargets>,

    frame_set: uniforms::BindSet,
    shadow_set: uniforms::BindSet,
    ssao_set: uniforms::BindSet,
    /// The grid's set and the gizmo lines' set: two, because each holds its own
    /// matrices and a pass that bound one buffer for both would draw one of them
    /// through the other's matrix.
    grid_set: uniforms::BindSet,
    line_set: uniforms::BindSet,
    image_set: ImageSet,
    draw_arena: DrawArena,

    mesh_layout: VertexLayoutKey,
    line_batch: Option<LineBatch>,
    show_grid: bool,

    overlay: Option<Arc<dyn OverlayPasses>>,
    frame_index: u64,
}

impl Bifrost {
    /// Builds the renderer against a live device.
    ///
    /// `assets` is taken because `Renderer3D::Init` loaded its shaders through
    /// the file system (`Renderer3D.cpp:63-70`); here the sources are embedded
    /// and compiled in process, and the registry is where a caller's meshes and
    /// images come from.
    pub fn new(gpu: &mut dyn Gpu, assets: &mut vault::Assets) -> Result<Self, Error> {
        let _ = assets;

        let extent = gpu.swapchain().map(|swapchain| swapchain.extent()).unwrap_or(Extent2d {
            width: 1280,
            height: 720,
        });
        let swapchain_format = gpu
            .swapchain()
            .map(|swapchain| swapchain.format())
            .unwrap_or(TextureFormat::Bgra8Srgb);

        let resources = gpu.resources();
        let shaders = ShaderSet::load(resources)?;
        let textures = FrameTextures::create(resources, extent, SHADOW_MAP_SIZE)?;

        let mut bifrost = Self {
            extent,
            swapchain_format,
            textures,
            shaders,
            pipelines: PipelineCache::new(),
            meshes: MeshCache::new(),
            materials: MaterialCache::new(),
            textures_by_asset: TextureCache::new(),
            material_assets: HashMap::new(),
            lights: LightBuffer::new(),
            graph: RenderGraph::new(),
            targets: None,
            frame_set: uniforms::BindSet::new(),
            shadow_set: uniforms::BindSet::new(),
            ssao_set: uniforms::BindSet::new(),
            grid_set: uniforms::BindSet::new(),
            line_set: uniforms::BindSet::new(),
            image_set: ImageSet::new(),
            draw_arena: DrawArena::new(),
            mesh_layout: VertexLayoutKey::of(&mesh::vertex_layout()),
            line_batch: None,
            show_grid: true,
            overlay: None,
            frame_index: 0,
        };

        // The 1x1 textures a material with no map of a kind samples. Made once,
        // as the C++ makes one `WhiteTexture` at `Init`.
        bifrost.textures_by_asset.white(gpu.resources())?;
        bifrost.textures_by_asset.neutral_normal(gpu.resources())?;

        Ok(bifrost)
    }

    /// Installs the 2D and text slice's passes into the frame.
    pub fn set_overlay(&mut self, overlay: Arc<dyn OverlayPasses>) {
        self.overlay = Some(overlay);
    }

    pub fn overlay(&self) -> Option<&Arc<dyn OverlayPasses>> {
        self.overlay.as_ref()
    }

    /// Resizes the frame's own targets.
    ///
    /// The textures are replaced by the next `render`, which has the device;
    /// this is called from a window event, where there is not. A zero extent is
    /// skipped rather than clamped, which is what `Framebuffer::Resize` does.
    pub fn resize(&mut self, extent: Extent2d) {
        if extent.width == 0 || extent.height == 0 {
            return;
        }
        self.extent = extent;
    }

    pub fn extent(&self) -> Extent2d {
        self.extent
    }

    pub fn meshes(&self) -> &MeshCache {
        &self.meshes
    }

    pub fn materials(&self) -> &MaterialCache {
        &self.materials
    }

    pub fn textures(&self) -> &TextureCache {
        &self.textures_by_asset
    }

    pub fn pipelines(&self) -> &PipelineCache {
        &self.pipelines
    }

    pub fn lights(&self) -> &LightBuffer {
        &self.lights
    }

    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// The pass slots of the last frame, which a caller can inspect or draw into.
    pub fn targets(&self) -> Option<FrameTargets> {
        self.targets
    }

    pub fn show_grid(&mut self, show: bool) {
        self.show_grid = show;
    }

    /// Registers a material description under an asset id, so a
    /// `MeshSample::material` resolves to it.
    ///
    /// The C++'s loader filled `RD3d::Material` outright; this is where that
    /// result lands in the Rust split.
    pub fn set_material(&mut self, id: vault::AssetId, material: Material) {
        self.material_assets.insert(id, material);
    }

    /// Uploads a mesh if it is not resident. `None` when the asset has no
    /// CPU-side mesh, which is not an error: a path that failed to load has no
    /// mesh to draw.
    pub fn ensure_mesh(
        &mut self,
        gpu: &mut dyn Gpu,
        assets: &vault::Assets,
        id: vault::AssetId,
    ) -> Result<Option<GpuMesh>, Error> {
        let key = MeshKey::new(id, self.mesh_layout);
        if let Some(mesh) = self.meshes.get(&key) {
            return Ok(Some(mesh));
        }
        let Some(data) = assets.mesh(id) else {
            return Ok(None);
        };
        Ok(Some(self.meshes.get_or_upload(gpu.resources(), key, data)?))
    }

    /// Drops every GPU resource a mesh holds.
    pub fn release_mesh(&mut self, gpu: &mut dyn Gpu, id: vault::AssetId) -> bool {
        self.meshes
            .destroy(gpu.resources(), &MeshKey::new(id, self.mesh_layout))
    }

    /// Uploads an image as a texture. A repeated call is a cache hit.
    pub fn ensure_texture(
        &mut self,
        gpu: &mut dyn Gpu,
        assets: &vault::Assets,
        id: vault::AssetId,
    ) -> Result<(), Error> {
        if self.textures_by_asset.get(id).is_some() {
            return Ok(());
        }
        let Some(image) = assets.image(id).cloned() else {
            return Ok(());
        };
        self.textures_by_asset.get_or_upload(gpu.resources(), id, &image)?;
        Ok(())
    }

    /// Destroys a texture **and every material that bound it**.
    ///
    /// A bind group naming a destroyed texture is a descriptor pointing at freed
    /// memory, so the two cannot be dropped independently.
    pub fn release_texture(&mut self, gpu: &mut dyn Gpu, id: vault::AssetId) -> bool {
        let resources = gpu.resources();
        let Some(texture) = self.textures_by_asset.destroy(resources, id) else {
            return false;
        };
        self.materials.invalidate_texture(resources, texture);
        true
    }

    /// Replaces the gizmo line batch.
    pub fn set_lines(
        &mut self,
        gpu: &mut dyn Gpu,
        vertices: &[passes::LineVertex],
        indices: &[u32],
    ) -> Result<(), Error> {
        let resources = gpu.resources();
        if let Some(batch) = self.line_batch.take() {
            resources.destroy_buffer(batch.vertex);
            resources.destroy_buffer(batch.index);
        }
        if vertices.is_empty() || indices.is_empty() {
            return Ok(());
        }

        let (vertex_bytes, index_bytes) = passes::lines::pack(vertices, indices);
        let vertex = resources.create_buffer(
            BufferDesc::new(vertex_bytes.len() as u64, BufferUsage::VERTEX),
            Some(&vertex_bytes),
        )?;
        let index = resources.create_buffer(
            BufferDesc::new(index_bytes.len() as u64, BufferUsage::INDEX),
            Some(&index_bytes),
        )?;
        self.line_batch = Some(LineBatch {
            vertex,
            index,
            index_count: indices.len() as u32,
        });
        Ok(())
    }

    /// Advances an editor camera by one frame's input.
    ///
    /// The C++ read `Input::` globals in `EditorCamera::OnUpdate`; this takes the
    /// state, so a caller with a window owns the only copy of it.
    pub fn update_editor_camera(&self, camera: &mut EditorCamera, input: &CameraInput) {
        camera.update(input);
        camera.on_scroll(input.scroll.1);
    }

    /// Renders one frame.
    pub fn render(
        &mut self,
        gpu: &mut dyn Gpu,
        jobs: &grind::Jobs,
        scene: &meats::Scene,
        view: ViewSource,
        editor_camera: Option<&EditorCamera>,
    ) -> Result<(), Error> {
        let _ = jobs;

        let extent = gpu
            .swapchain()
            .map(|swapchain| swapchain.extent())
            .unwrap_or(self.extent);
        if extent.width == 0 || extent.height == 0 {
            // A minimised window. `Framebuffer::Resize` says the same: a zero
            // extent is not a frame to draw, it is a frame to skip.
            return Ok(());
        }
        if extent != self.extent {
            let resources = gpu.resources();
            let textures = FrameTextures::create(resources, extent, SHADOW_MAP_SIZE)?;
            let old = std::mem::replace(&mut self.textures, textures);
            old.destroy(resources);
            self.extent = extent;
        }
        if let Some(format) = gpu.swapchain().map(|swapchain| swapchain.format()) {
            self.swapchain_format = format;
        }

        let (view_projection, camera_position) = resolve_view(scene, view, editor_camera)?;
        let light_data = light::extract(scene);
        let shadow_matrix = shadow_view_projection(&light_data, camera_position);

        // ── declare the frame ──────────────────────────────────────────────
        let targets = self.declare_frame();
        self.targets = Some(targets);

        let compiled = self
            .graph
            .compile()
            .map_err(|errors| Error::InvalidDescriptor(describe(&errors)))?;

        // ── prepare what the recorders read ───────────────────────────────
        let resources = gpu.resources();

        let mut slots = SlotTable::default();
        slots.set_texture(targets.scene_color, self.textures.color);
        slots.set_texture(targets.scene_depth, self.textures.depth);
        slots.set_texture(targets.shadow, self.textures.shadow);
        slots.set_texture(targets.ssao, self.textures.ssao);

        let (light_buffer, light_count) = self.lights.ensure(resources, &light_data)?;
        let frame_uniforms = uniforms::FrameUniforms::new(
            view_projection.to_cols_array_2d(),
            shadow_matrix.to_cols_array_2d(),
            camera_position.to_array(),
            light_count,
            true,
        );
        let frame_group = self.frame_set.rebuild(
            resources,
            &passes::pipeline::frame_layout(),
            &frame_uniforms,
            &[(1, light_buffer, 0)],
            &[(2, self.textures.shadow_view)],
            &[(3, self.textures.sampler)],
        )?;

        let draws = self.prepare_draws(resources, scene, camera_position)?;
        let recorder = self.build_recorders(
            resources,
            &compiled,
            shadow_matrix,
            view_projection,
            &frame_group,
            draws,
        )?;

        // ── run it ────────────────────────────────────────────────────────
        self.frame_index += 1;
        // One trait object: the backend owns the command lists and may record the
        // passes on worker threads.
        let recorder: Arc<dyn PassRecorder> = recorder;
        gpu.record_frame(&compiled, &slots, recorder)?;

        Ok(())
    }

    /// Declares this frame's resources and passes.
    fn declare_frame(&mut self) -> FrameTargets {
        let graph = &mut self.graph;
        graph.reset();

        // Imports, not transients: these textures outlive the frame, and imports
        // are the graph's culling roots (`compiler::cull`). `Undefined` as the
        // entry state is the C++ habit of `oldLayout = eUndefined`
        // (`VulkanCommandBuffer.cpp:104,153`) and the state from which the first
        // transition is free.
        let scene_color =
            graph.import_texture("scene.color", ImageState::Undefined, ImageState::ShaderRead);
        let scene_depth =
            graph.import_texture("scene.depth", ImageState::Undefined, ImageState::DepthAttachment);
        let shadow = graph.import_texture("shadow.map", ImageState::Undefined, ImageState::ShaderRead);
        let ssao = graph.import_texture("ssao.ao", ImageState::Undefined, ImageState::ShaderRead);
        let backbuffer =
            graph.import_texture("backbuffer", ImageState::Undefined, ImageState::Present);

        // The shadow map: depth only, cleared to the far plane.
        graph
            .pass(passes::PASS_SHADOW)
            .kind(chassis::graph::PassType::Graphics)
            .depth(shadow, 1.0);

        // Opaque geometry, then the blended draws (`GeometryPass`) — and the
        // clear, because it is this pass's first writer. A separate clear pass
        // would be culled: `compiler::cull` keeps a pass only when a live pass
        // reads what it produced, and a `LoadOp::Load` is not a read in that
        // model, so a clear whose target the next pass overwrites looks dead.
        // `Renderer3D` clears in the same pass it draws for the same reason.
        graph
            .pass(passes::PASS_OPAQUE)
            .color(scene_color, CLEAR_COLOR)
            .depth(scene_depth, 1.0)
            .read_texture(shadow);

        // Ambient occlusion, from the depth the opaque pass wrote.
        graph
            .pass(passes::PASS_SSAO)
            .color(ssao, [1.0, 1.0, 1.0, 1.0])
            .read_texture(scene_depth);

        // The editor's grid and gizmos, blended, after the scene.
        graph.pass(passes::PASS_EDITOR).color_load(scene_color);

        // The 2D and text slice's passes, which draw into the scene colour
        // after the 3D scene does. Declared here, before the composition, so
        // the composition is still the last writer of the backbuffer.
        if let Some(overlay) = &self.overlay {
            overlay.declare(graph, &targets);
        }

        // The composition: the scene colour and the occlusion into the
        // backbuffer, whose slot the executor resolves.
        graph
            .pass(passes::PASS_COMPOSITE)
            .color(backbuffer, [0.0, 0.0, 0.0, 1.0])
            .read_texture(scene_color)
            .read_texture(ssao);

        FrameTargets {
            scene_color,
            scene_depth,
            shadow,
            ssao,
            backbuffer,
        }
    }

    /// Turns the scene's meshes into draws, and uploads their uniforms.
    fn prepare_draws(
        &mut self,
        resources: &mut dyn Resources,
        scene: &meats::Scene,
        camera_position: Vec3,
    ) -> Result<Vec<PreparedDraw>, Error> {
        struct Pending {
            mesh: GpuMesh,
            material: Option<MaterialBinding>,
            variant: MaterialVariant,
            distance: f32,
            block: usize,
        }

        let mut blocks: Vec<Vec<u8>> = Vec::new();
        let mut pending: Vec<Pending> = Vec::new();

        for sample in scene.meshes() {
            let Some(mesh) = self
                .meshes
                .get(&MeshKey::new(sample.mesh, self.mesh_layout))
            else {
                // Not uploaded: `ensure_mesh` has not seen it. The C++ skips a
                // primitive with a null buffer for the same reason.
                log::trace!("bifrost: mesh {:?} has no GPU buffers yet", sample.mesh);
                continue;
            };

            let material = sample
                .material
                .and_then(|id| self.material_assets.get(&id))
                .copied()
                .unwrap_or_default();
            let variant = MaterialVariant::of(
                &material,
                material.alpha == material::AlphaMode::Blend,
            );

            let material_binding = self.material_binding(resources, &material, variant)?;

            let uniforms = uniforms::DrawUniforms::new(
                sample.transform.to_cols_array_2d(),
                sample.tint.to_array(),
                entity_id(sample.entity),
            );
            let block = blocks.len();
            blocks.push(bytemuck::bytes_of(&uniforms).to_vec());

            pending.push(Pending {
                mesh,
                material: material_binding,
                variant,
                distance: (sample.transform.w_axis.truncate() - camera_position).length(),
                block,
            });
        }

        self.draw_arena
            .begin(resources, &passes::pipeline::draw_layout(), &blocks)?;

        // Opaque first, then the blended draws back to front. The C++ draws in
        // submission order and says a sorted pass is what transmission needs
        // (`MaterialExtensions.h:29-36`).
        pending.sort_by(|a, b| {
            a.variant
                .blends()
                .cmp(&b.variant.blends())
                .then_with(|| b.distance.total_cmp(&a.distance))
        });

        Ok(pending
            .into_iter()
            .filter_map(|draw| {
                Some(PreparedDraw {
                    mesh: draw.mesh,
                    uniforms: self.draw_arena.group(draw.block)?,
                    material: draw.material.map(|binding| binding.group),
                    variant: draw.variant,
                })
            })
            .collect())
    }

    /// One material's bind group, created once per distinct [`MaterialKey`].
    fn material_binding(
        &mut self,
        resources: &mut dyn Resources,
        material: &Material,
        variant: MaterialVariant,
    ) -> Result<Option<MaterialBinding>, Error> {
        let white = self.textures_by_asset.white(resources)?;
        let neutral = self.textures_by_asset.neutral_normal(resources)?;

        let mut resolved: [CachedTexture; material::SLOT_COUNT] = [white; material::SLOT_COUNT];
        let mut images: Vec<(u32, ImageView)> = Vec::with_capacity(material::SLOT_COUNT);

        for slot in 0..material::SLOT_COUNT {
            let cached = material
                .texture(slot)
                .and_then(|id| self.textures_by_asset.get(id))
                .unwrap_or(if slot == material::SLOT_NORMAL {
                    neutral
                } else {
                    white
                });
            resolved[slot] = cached;
            images.push((2 + slot as u32, cached.view));
        }

        let handles = MaterialTextures {
            albedo: Some(resolved[material::SLOT_ALBEDO].texture),
            metallic_roughness: Some(resolved[material::SLOT_METALLIC_ROUGHNESS].texture),
            normal: Some(resolved[material::SLOT_NORMAL].texture),
            occlusion: Some(resolved[material::SLOT_OCCLUSION].texture),
            emissive: Some(resolved[material::SLOT_EMISSIVE].texture),
        };

        let key = MaterialKey::of(material, handles, variant);
        let layout = passes::pipeline::material_layout();
        let samplers = [(1u32, self.textures.sampler)];
        Ok(Some(self.materials.get_or_insert(
            resources,
            key,
            material,
            &images,
            &samplers,
            &layout,
        )?))
    }

    /// Builds every pass's recorder for this frame.
    fn build_recorders(
        &mut self,
        resources: &mut dyn Resources,
        compiled: &CompiledGraph,
        shadow_matrix: Mat4,
        view_projection: Mat4,
        frame_group: &BindGroup,
        draws: Vec<PreparedDraw>,
    ) -> Result<Arc<FrameRecorder>, Error> {
        let samples = chassis::SampleCount::X1;
        let color = TextureFormat::Rgba8Srgb;
        let depth = TextureFormat::Depth32Float;
        let mut recorder = FrameRecorder::new();

        if compiled.find(passes::PASS_SHADOW).is_some() {
            let pipeline = self.pipelines.get_or_create(
                resources,
                PipelineKey::shadow(depth, samples, &mesh::vertex_layout()),
                &self.shaders,
            )?;
            let uniforms = uniforms::ShadowUniforms::new(shadow_matrix.to_cols_array_2d());
            let group = self.shadow_set.rebuild(
                resources,
                &passes::pipeline::shadow_layout(),
                &uniforms,
                &[],
                &[],
                &[],
            )?;
            // Blended draws are left out: a transparent surface in a shadow map
            // writes the depth of its whole quad, not of the part the blend lets
            // through, and the C++ has no shadow map that would say either way.
            let shadow_draws: Vec<PreparedDraw> = draws
                .iter()
                .copied()
                .filter(|draw| !draw.transparent())
                .collect();
            recorder.shadow = Some(ShadowPass::new(pipeline, group, shadow_draws));
        }

        if compiled.find(passes::PASS_OPAQUE).is_some() {
            let pipelines = MaterialPipelines {
                opaque: self.pipelines.get_or_create(
                    resources,
                    PipelineKey::model(ShaderVariant::Model, color, depth, samples),
                    &self.shaders,
                )?,
                opaque_double_sided: self.pipelines.get_or_create(
                    resources,
                    PipelineKey::model(ShaderVariant::ModelDoubleSided, color, depth, samples),
                    &self.shaders,
                )?,
                transparent: self.pipelines.get_or_create(
                    resources,
                    PipelineKey::model(ShaderVariant::ModelTransparent, color, depth, samples),
                    &self.shaders,
                )?,
                transparent_double_sided: self.pipelines.get_or_create(
                    resources,
                    PipelineKey::model(
                        ShaderVariant::ModelTransparentDoubleSided,
                        color,
                        depth,
                        samples,
                    ),
                    &self.shaders,
                )?,
            };
            recorder.geometry = Some(GeometryPass::new(pipelines, *frame_group, draws));
        }

        if compiled.find(passes::PASS_SSAO).is_some() {
            let pipeline = self.pipelines.get_or_create(
                resources,
                PipelineKey::fullscreen(ShaderVariant::Ssao, TextureFormat::R8Unorm, samples),
                &self.shaders,
            )?;
            let uniforms = SsaoUniforms::new(
                &passes::ssao::fixed_kernel(0x9E37_79B9_7F4A_7C15),
                view_projection.to_cols_array_2d(),
                view_projection.inverse().to_cols_array_2d(),
                passes::ssao::DEFAULT_RADIUS,
                passes::ssao::DEFAULT_BIAS,
                passes::ssao::DEFAULT_POWER,
            );
            let group = self.ssao_set.rebuild(
                resources,
                &passes::pipeline::ssao_layout(),
                &uniforms,
                &[],
                &[(1, self.textures.depth_view)],
                &[(2, self.textures.sampler)],
            )?;
            recorder.ssao = Some(SsaoPass::new(pipeline, group));
        }

        if compiled.find(passes::PASS_EDITOR).is_some() {
            let grid = if self.show_grid {
                let pipeline = self.pipelines.get_or_create(
                    resources,
                    PipelineKey::grid(color, samples),
                    &self.shaders,
                )?;
                let uniforms = GridUniforms::new(view_projection.to_cols_array_2d());
                let group = self.grid_set.rebuild(
                    resources,
                    &passes::pipeline::grid_layout(),
                    &uniforms,
                    &[],
                    &[],
                    &[],
                )?;
                Some(GridPass::new(pipeline, group))
            } else {
                None
            };

            let lines = match self.line_batch {
                Some(batch) => {
                    let pipeline = self.pipelines.get_or_create(
                        resources,
                        PipelineKey::lines(color, samples),
                        &self.shaders,
                    )?;
                    let uniforms = LineUniforms::new(
                        view_projection.to_cols_array_2d(),
                        Mat4::IDENTITY.to_cols_array_2d(),
                    );
                    let group = self.line_set.rebuild(
                        resources,
                        &passes::pipeline::line_layout(),
                        &uniforms,
                        &[],
                        &[],
                        &[],
                    )?;
                    Some(LinePass::new(pipeline, group, batch))
                }
                None => None,
            };

            if grid.is_some() || lines.is_some() {
                recorder.editor = Some(EditorPass::new(grid, lines));
            }
        }

        if compiled.find(passes::PASS_COMPOSITE).is_some() {
            let pipeline = self.pipelines.get_or_create(
                resources,
                PipelineKey::fullscreen(ShaderVariant::Composite, self.swapchain_format, samples),
                &self.shaders,
            )?;
            let group = self.image_set.rebuild(
                resources,
                &passes::pipeline::composite_layout(),
                &[
                    (0, self.textures.color_view),
                    (1, self.textures.ssao_view),
                ],
                &[(2, self.textures.sampler)],
            )?;
            recorder.composite = Some(CompositePass::new(pipeline, group));
        }

        recorder.overlay = self.overlay.clone();
        Ok(Arc::new(recorder))
    }

    /// Drops every GPU resource this renderer owns.
    pub fn destroy(&mut self, gpu: &mut dyn Gpu) {
        let resources = gpu.resources();
        self.meshes.clear(resources);
        self.materials.clear(resources);
        self.textures_by_asset.clear(resources);
        self.pipelines.clear(resources);
        self.lights.destroy(resources);
        self.frame_set.destroy(resources);
        self.shadow_set.destroy(resources);
        self.ssao_set.destroy(resources);
        self.grid_set.destroy(resources);
        self.line_set.destroy(resources);
        self.image_set.destroy(resources);
        self.draw_arena.destroy(resources);
        if let Some(batch) = self.line_batch.take() {
            resources.destroy_buffer(batch.vertex);
            resources.destroy_buffer(batch.index);
        }
        self.shaders.destroy(resources);
        let textures = std::mem::replace(&mut self.textures, FrameTextures::null());
        textures.destroy(resources);
    }
}

/// The view-projection and the camera's world position for this frame.
fn resolve_view(
    scene: &meats::Scene,
    view: ViewSource,
    editor_camera: Option<&EditorCamera>,
) -> Result<(Mat4, Vec3), Error> {
    match view {
        ViewSource::Editor => {
            let camera = editor_camera.ok_or_else(|| {
                Error::InvalidDescriptor(
                    "ViewSource::Editor with no editor camera passed to render".to_owned(),
                )
            })?;
            Ok((camera.view_projection(), camera.position()))
        }
        ViewSource::Scene(entity) => {
            let transform = scene.world_transform(entity);
            let reference = scene.get(entity).ok_or_else(|| {
                Error::InvalidDescriptor(format!("view entity {entity:?} is not in the scene"))
            })?;
            let camera = reference.get::<meats::Camera>().ok_or_else(|| {
                Error::InvalidDescriptor(format!("view entity {entity:?} has no Camera component"))
            })?;
            // The C++ `BeginScene(camera, transform, lights)` overload:
            // `projection * inverse(transform)`.
            Ok((
                camera.projection() * transform.inverse(),
                transform.w_axis.truncate(),
            ))
        }
    }
}

/// A directional light's shadow matrix, in the camera's neighbourhood.
///
/// The C++ has no shadow map, so there is nothing to port: this places an
/// orthographic box on the camera and looks along the first directional light.
/// With no directional light the identity is returned, and the fragment shader's
/// out-of-range uv test then leaves every surface unshadowed.
fn shadow_view_projection(lights: &[LightData], camera_position: Vec3) -> Mat4 {
    let Some(light) = lights
        .iter()
        .find(|light| light.light_type() == light::TYPE_DIRECTIONAL)
    else {
        return Mat4::IDENTITY;
    };

    let direction = Vec3::new(
        light.position_or_direction[0],
        light.position_or_direction[1],
        light.position_or_direction[2],
    );
    let direction = if direction.length_squared() > 0.0 {
        direction.normalize()
    } else {
        Vec3::NEG_Y
    };

    let eye = camera_position - direction * SHADOW_EXTENT;
    let view = Mat4::look_at_rh(eye, camera_position, Vec3::Y);
    let projection = Mat4::orthographic_rh(
        -SHADOW_EXTENT,
        SHADOW_EXTENT,
        -SHADOW_EXTENT,
        SHADOW_EXTENT,
        0.1,
        SHADOW_EXTENT * 4.0,
    );
    projection * view
}

/// The id an editor picks with.
///
/// The C++ casts the entity to an `int`, which is entt's low half: the slot
/// index. `Entity::index` is the same half, and it is what identifies the live
/// entity a pick buffer names. The persistent identity is the `Id` component's
/// uuid, which a pick buffer has no use for.
fn entity_id(entity: meats::Entity) -> i32 {
    entity.index() as i32
}

fn describe(errors: &[chassis::graph::types::ValidationError]) -> String {
    errors
        .iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}
