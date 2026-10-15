//! The 2D sprite batch.
//!
//! One texture at a time. The C++ packed up to 32 textures into a descriptor
//! array and flushed when the slots ran out (`UHE/src/UHE/Renderer/Renderer2D.cpp:60`),
//! so which texture a vertex sampled was a *number in a lookup table* shared by
//! the whole frame — and a batch could be drawn with the wrong table. Here a
//! texture change is the flush, so a draw call binds exactly the texture its
//! vertices were recorded against and there is no table to disagree with.
//!
//! Vertex positions are built by transforming the C++ quad
//! (`Renderer2D.cpp:399-402`) with the caller's `Mat4`; the index pattern is the
//! C++ one (`Renderer2D.cpp:79-92`). They are the spec for the quad and its
//! winding, and a second convention for either would be a second source of
//! truth.

use glam::{Mat4, Vec2, Vec3, Vec4};

use chassis::{
    BindingDesc, BlendState, Buffer, BufferDesc, BufferUsage, ColorTargetState, DepthState,
    DescriptorKind, DescriptorSetLayoutDesc, IndexFormat, Pipeline, PipelineDesc, PrimitiveState,
    Resources, SampleCount, ShaderStage, ShaderStageDesc, StageMask, Texture, TextureFormat,
    VertexAttribute, VertexLayout,
};

pub const SPRITE_WGSL: &str = include_str!("../../../assets/shaders/sprite.wgsl");

/// A unit quad about the origin, counter-clockwise from the bottom left.
const QUAD_POSITIONS: [[f32; 3]; 4] = [
    [-0.5, -0.5, 0.0],
    [0.5, -0.5, 0.0],
    [0.5, 0.5, 0.0],
    [-0.5, 0.5, 0.0],
];

const QUAD_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// `QuadVertex` (`Renderer2D.cpp:16-24`) without `TexIndex`: a batch holds one
/// texture, so the index would be a constant 0 in every vertex — and without
/// `EntityID`, which the C++ wrote to a second `R32_SINT` colour target that
/// `chassis::TextureFormat` has no counterpart for.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
    pub tiling_factor: f32,
}

/// One indexed draw: every quad between two texture changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawBatch {
    /// The texture bound for this draw. `None` is an untextured quad; the pass
    /// binds a 1x1 white texel for it, which is what the C++ texture-slot 0 was
    /// (`Renderer2D.cpp:30`, `127-131`).
    pub texture: Option<Texture>,
    pub index_offset: u32,
    pub index_count: u32,
}

/// `Renderer2D::Statistics` (`Renderer2D.h:33-43`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Statistics {
    pub draw_calls: u32,
    pub quad_count: u32,
}

impl Statistics {
    pub fn total_vertex_count(&self) -> u32 {
        self.quad_count * 4
    }

    pub fn total_index_count(&self) -> u32 {
        self.quad_count * 6
    }
}

/// `translate(position) * scale(size)`, the transform the C++
/// position/size overloads built (`Renderer2D.cpp:376-379`).
pub fn quad_transform(position: Vec3, size: Vec2) -> Mat4 {
    Mat4::from_translation(position) * Mat4::from_scale(Vec3::new(size.x, size.y, 1.0))
}

#[derive(Clone, Debug, Default)]
pub struct Batch2D {
    vertices: Vec<SpriteVertex>,
    indices: Vec<u32>,
    texture: Option<Texture>,
    /// Indices written into the batch that is still open.
    open: u32,
    batches: Vec<DrawBatch>,
    stats: Statistics,
}

impl Batch2D {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-size for `quads`, so a frame's first batch does not grow its way up.
    pub fn with_capacity(quads: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(quads * 4),
            indices: Vec::with_capacity(quads * 6),
            ..Self::default()
        }
    }

    /// Start a frame. `Statistics` survives, as it did in the C++ where only
    /// `ResetStats` cleared it.
    pub fn begin(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.batches.clear();
        self.texture = None;
        self.open = 0;
    }

    pub fn reset_stats(&mut self) {
        self.stats = Statistics::default();
    }

    pub fn stats(&self) -> Statistics {
        self.stats
    }

    pub fn push_quad(&mut self, transform: Mat4, color: Vec4) {
        self.push_sprite(None, transform, color, 1.0);
    }

    pub fn push_sprite(
        &mut self,
        texture: Option<Texture>,
        transform: Mat4,
        color: Vec4,
        tiling_factor: f32,
    ) {
        if self.texture != texture {
            self.close();
            self.texture = texture;
        }

        let base = self.vertices.len() as u32;
        for corner in 0..4 {
            let local = Vec4::new(
                QUAD_POSITIONS[corner][0],
                QUAD_POSITIONS[corner][1],
                QUAD_POSITIONS[corner][2],
                1.0,
            );
            let position = transform * local;
            self.vertices.push(SpriteVertex {
                position: [position.x, position.y, position.z],
                color: color.to_array(),
                uv: QUAD_UV[corner],
                tiling_factor,
            });
        }
        for offset in [0u32, 1, 2, 2, 3, 0] {
            self.indices.push(base + offset);
        }

        self.open += 6;
        self.stats.quad_count += 1;
    }

    /// Close the open batch. A `push_*` with a different texture calls this
    /// itself; calling it by hand ends the frame's last draw.
    pub fn flush(&mut self) {
        self.close();
    }

    pub fn batches(&self) -> &[DrawBatch] {
        &self.batches
    }

    pub fn vertices(&self) -> &[SpriteVertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn vertex_count(&self) -> u32 {
        self.vertices.len() as u32
    }

    pub fn index_count(&self) -> u32 {
        self.indices.len() as u32
    }

    pub fn vertex_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.vertices)
    }

    pub fn index_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.indices)
    }

    fn close(&mut self) {
        if self.open == 0 {
            return;
        }
        let index_offset = self.indices.len() as u32 - self.open;
        self.batches.push(DrawBatch {
            texture: self.texture,
            index_offset,
            index_count: self.open,
        });
        self.stats.draw_calls += 1;
        self.open = 0;
    }
}

/// Set 0 for both 2D pipelines: the texture, its sampler, and the globals.
///
/// Separate image and sampler bindings, not a combined one, because wgpu has no
/// combined descriptor at all — the same constraint `assets/shaders/Textured.slang`
/// documents at length.
pub fn quad_set_layout() -> DescriptorSetLayoutDesc {
    let fragment = StageMask::of(ShaderStage::Fragment);
    let both = StageMask::of(ShaderStage::Vertex).union(fragment);
    DescriptorSetLayoutDesc::new("bifrost.2d")
        .with(BindingDesc::new(0, DescriptorKind::SampledImage, fragment))
        .with(BindingDesc::new(1, DescriptorKind::Sampler, fragment))
        .with(BindingDesc::new(2, DescriptorKind::UniformBuffer, both))
}

pub fn sprite_vertex_layout() -> VertexLayout {
    VertexLayout {
        stride: std::mem::size_of::<SpriteVertex>() as u32,
        attributes: vec![
            VertexAttribute {
                location: 0,
                format: TextureFormat::R32G32B32Float,
                offset: 0,
            },
            VertexAttribute {
                location: 1,
                format: TextureFormat::Rgba32Float,
                offset: 12,
            },
            VertexAttribute {
                location: 2,
                format: TextureFormat::R32G32Float,
                offset: 28,
            },
            VertexAttribute {
                location: 3,
                format: TextureFormat::R32Float,
                offset: 36,
            },
        ],
    }
}

/// What went wrong building a 2D pipeline: the shader it compiles or the RHI it
/// asks for.
#[derive(Debug)]
pub enum PipelineError {
    Wgsl(forge::wgsl::WgslError),
    Rhi(chassis::Error),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::Wgsl(error) => write!(f, "{error}"),
            PipelineError::Rhi(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<forge::wgsl::WgslError> for PipelineError {
    fn from(error: forge::wgsl::WgslError) -> Self {
        PipelineError::Wgsl(error)
    }
}

impl From<chassis::Error> for PipelineError {
    fn from(error: chassis::Error) -> Self {
        PipelineError::Rhi(error)
    }
}

/// A 2D pipeline and everything a caller needs to bind it: the set layout the
/// bind group must be built from, so the two cannot disagree.
pub struct QuadPipeline {
    pub pipeline: Pipeline,
    pub vertex_shader: chassis::Shader,
    pub fragment_shader: chassis::Shader,
    pub set_layout: DescriptorSetLayoutDesc,
}

impl QuadPipeline {
    /// `source` is a WGSL file with `vs_main` and `fs_main`, one
    /// `create_shader` per entry point as both backends require.
    pub fn build(
        resources: &mut dyn Resources,
        label: &str,
        source: &str,
        vertex_layout: VertexLayout,
        color_format: TextureFormat,
        depth_format: Option<TextureFormat>,
    ) -> Result<Self, PipelineError> {
        let vertex_spirv =
            forge::wgsl::to_spirv_entry(source, label, "vs_main", forge::Stage::Vertex)?;
        let fragment_spirv =
            forge::wgsl::to_spirv_entry(source, label, "fs_main", forge::Stage::Fragment)?;

        let vertex_shader = resources.create_shader(&format!("{label}.vert"), &vertex_spirv)?;
        let fragment_shader = resources.create_shader(&format!("{label}.frag"), &fragment_spirv)?;

        let set_layout = quad_set_layout();
        let pipeline = resources.create_pipeline(PipelineDesc {
            label: label.to_owned(),
            shaders: vec![
                ShaderStageDesc {
                    stage: ShaderStage::Vertex,
                    shader: vertex_shader,
                },
                ShaderStageDesc {
                    stage: ShaderStage::Fragment,
                    shader: fragment_shader,
                },
            ],
            set_layouts: vec![set_layout.clone()],
            vertex: vertex_layout,
            index_format: Some(IndexFormat::U32),
            // Culling off: the C++ pipeline set neither, and a 2D quad's winding
            // is only meaningful once the view-projection is known.
            primitive: PrimitiveState {
                cull_mode: 0,
                ..PrimitiveState::default()
            },
            depth: DepthState {
                format: depth_format,
                write_enabled: depth_format.is_some(),
                compare: 0,
            },
            blend: BlendState { enabled: true },
            color_targets: vec![ColorTargetState {
                format: color_format,
                blend: true,
                write_mask: 0xF,
            }],
            samples: SampleCount::X1,
        })?;

        Ok(QuadPipeline {
            pipeline,
            vertex_shader,
            fragment_shader,
            set_layout,
        })
    }

    pub fn sprite(
        resources: &mut dyn Resources,
        color_format: TextureFormat,
        depth_format: Option<TextureFormat>,
    ) -> Result<Self, PipelineError> {
        Self::build(
            resources,
            "sprite",
            SPRITE_WGSL,
            sprite_vertex_layout(),
            color_format,
            depth_format,
        )
    }
}

/// The globals the sprite shader reads: one view-projection.
///
/// `repr(C)` and 16-byte aligned, because a uniform buffer is read as a
/// `mat4x4<f32>` by the shader and a struct with trailing padding would shift
/// every field after the first.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteGlobals {
    pub view_projection: [[f32; 4]; 4],
}

impl SpriteGlobals {
    pub fn new(view_projection: Mat4) -> Self {
        Self {
            view_projection: view_projection.to_cols_array_2d(),
        }
    }
}

/// A vertex and index buffer sized for the batch's current contents.
///
/// `chassis::Resources` has no buffer *update* — a buffer is created with its
/// contents, and changing them means a staging buffer and `CommandList::copy_buffer`,
/// which is the pass's to record. So the batch's own storage is the growable
/// one (`Vec`), and this is the size class a pass allocates from.
pub fn create_buffers(
    resources: &mut dyn Resources,
    batch: &Batch2D,
) -> chassis::Result<(Buffer, Buffer)> {
    let vertex = resources.create_buffer(
        BufferDesc::new(batch.vertex_bytes().len() as u64, BufferUsage::VERTEX),
        Some(batch.vertex_bytes()),
    )?;
    let index = resources.create_buffer(
        BufferDesc::new(batch.index_bytes().len() as u64, BufferUsage::INDEX),
        Some(batch.index_bytes()),
    )?;
    Ok((vertex, index))
}
