//! The text component's draw data.
//!
//! A `meats::Text` becomes one text run: every glyph of one atlas in one vertex
//! buffer, flushed into a single `draw_indexed`. The C++ issued one quad per
//! glyph through the same batch but re-resolved the texture slot inside the
//! character loop (`UHE/src/UHE/Renderer/Renderer2D.cpp:489-517`), which is a
//! per-glyph dependency on batch state that only ends up correct by accident.
//! Here the atlas is a parameter, so a run cannot sample the wrong one.
//!
//! Coordinates are `font::layout`'s, unmodified: this applies the entity's
//! `Mat4` and nothing else.
//!
//! `runs` is complete after every `push`, for the reason `crate::batch2d`
//! documents: a pass that has to be flushed before it can see what to draw is a
//! pass that draws nothing.

use glam::{Mat4, Vec4};

use chassis::{
    BindGroup, Buffer, BufferDesc, BufferUsage, CommandList, Pipeline, Resources, Texture,
    TextureFormat, VertexAttribute, VertexLayout,
};

use crate::batch2d::{GLOBALS_SET, INDEX_SIZE, PipelineError, QuadPipeline, TEXTURE_SET};
use crate::font::layout::{layout, LayoutOptions};
use crate::font::{Atlas, Layout};

pub const TEXT_WGSL: &str = include_str!("../assets/shaders/text.wgsl");

/// `TextVertex` (`Renderer2D.cpp:26-33`) without `TexIndex` and `EntityID`, for
/// the reasons on [`crate::batch2d::SpriteVertex`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
}

/// One indexed draw for one atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRun {
    pub texture: Texture,
    pub index_offset: u32,
    pub index_count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct TextRenderer {
    vertices: Vec<TextVertex>,
    indices: Vec<u32>,
    texture: Option<Texture>,
    /// Whether the last entry of `runs` is still taking glyphs.
    open: bool,
    runs: Vec<TextRun>,
}

impl TextRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.runs.clear();
        self.texture = None;
        self.open = false;
    }

    /// Lay out `component` and append it. `atlas` and `texture` are the font the
    /// caller resolved from `component.font` — the component names an asset, not
    /// a GPU object, and resolving that is the font cache's job.
    pub fn push(
        &mut self,
        component: &meats::Text,
        atlas: &Atlas,
        texture: Texture,
        transform: Mat4,
    ) {
        let options = LayoutOptions {
            kerning: component.kerning,
            line_spacing: component.line_spacing,
            ..LayoutOptions::default()
        };
        let laid_out = layout(&component.text, atlas, &options);
        self.push_layout(&laid_out, component.color, texture, transform);
    }

    pub fn push_layout(
        &mut self,
        laid_out: &Layout,
        color: Vec4,
        texture: Texture,
        transform: Mat4,
    ) {
        if laid_out.placements.is_empty() {
            return;
        }
        self.open_run(texture);

        for placement in &laid_out.placements {
            let base = self.vertices.len() as u32;
            for corner in 0..4 {
                let local = placement.quad.positions[corner];
                let position = transform * Vec4::new(local.x, local.y, 0.0, 1.0);
                self.vertices.push(TextVertex {
                    position: [position.x, position.y, position.z],
                    color: color.to_array(),
                    uv: placement.quad.uv[corner].to_array(),
                });
            }
            for offset in [0u32, 1, 2, 2, 3, 0] {
                self.indices.push(base + offset);
            }
        }

        let quads = laid_out.placements.len() as u32;
        if let Some(run) = self.runs.last_mut() {
            run.index_count += quads * 6;
        }
    }

    /// Ends the run in progress, so the next `push` starts a new one even with
    /// the same atlas. Never needed for `runs` to be complete.
    pub fn flush(&mut self) {
        self.open = false;
    }

    /// Bind the pipeline, the buffers and the globals, then draw every run.
    ///
    /// `texture_group` resolves a run's atlas to the bind group for set 0; the
    /// pass owns the sampler and that cache for the same reason the sprite batch
    /// documents.
    pub fn record(
        &self,
        list: &mut dyn CommandList,
        pipeline: &Pipeline,
        vertex_buffer: &Buffer,
        index_buffer: &Buffer,
        globals: &BindGroup,
        texture_group: impl Fn(Texture) -> BindGroup,
    ) {
        if self.runs.is_empty() {
            return;
        }

        list.bind_pipeline(pipeline);
        list.bind_vertex_buffer(0, vertex_buffer, 0);
        list.bind_group(GLOBALS_SET, globals);
        for run in &self.runs {
            let group = texture_group(run.texture);
            list.bind_group(TEXTURE_SET, &group);
            list.bind_index_buffer(index_buffer, run.index_offset as u64 * INDEX_SIZE);
            list.draw_indexed(run.index_count, 1);
        }
    }

    pub fn runs(&self) -> &[TextRun] {
        &self.runs
    }

    pub fn vertices(&self) -> &[TextVertex] {
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

    /// The run a string belongs to. A different atlas starts the next one, which
    /// is what keeps `runs` complete after every `push` — see
    /// [`crate::batch2d::Batch2D`] for why that matters to the pass.
    fn open_run(&mut self, texture: Texture) {
        if self.open && self.texture == Some(texture) {
            return;
        }
        self.texture = Some(texture);
        self.runs.push(TextRun {
            texture,
            index_offset: self.indices.len() as u32,
            index_count: 0,
        });
        self.open = true;
    }
}

pub fn text_vertex_layout() -> VertexLayout {
    VertexLayout {
        stride: std::mem::size_of::<TextVertex>() as u32,
        attributes: vec![
            VertexAttribute {
                location: 0,
                format: TextureFormat::R32G32B32Float,
                offset: std::mem::offset_of!(TextVertex, position) as u32,
            },
            VertexAttribute {
                location: 1,
                format: TextureFormat::Rgba32Float,
                offset: std::mem::offset_of!(TextVertex, color) as u32,
            },
            VertexAttribute {
                location: 2,
                format: TextureFormat::R32G32Float,
                offset: std::mem::offset_of!(TextVertex, uv) as u32,
            },
        ],
    }
}

pub fn create_text_pipeline(
    resources: &mut dyn chassis::Resources,
    color_format: TextureFormat,
    depth_format: Option<TextureFormat>,
) -> Result<QuadPipeline, PipelineError> {
    QuadPipeline::build(
        resources,
        "text",
        TEXT_WGSL,
        text_vertex_layout(),
        color_format,
        depth_format,
    )
}

/// A vertex and index buffer sized for the runs' current contents. See
/// [`crate::batch2d::create_buffers`] for why they are created rather than
/// updated.
pub fn create_buffers(
    resources: &mut dyn Resources,
    text: &TextRenderer,
) -> chassis::Result<(Buffer, Buffer)> {
    let vertex = resources.create_buffer(
        BufferDesc::new(text.vertex_bytes().len() as u64, BufferUsage::VERTEX),
        Some(text.vertex_bytes()),
    )?;
    let index = resources.create_buffer(
        BufferDesc::new(text.index_bytes().len() as u64, BufferUsage::INDEX),
        Some(text.index_bytes()),
    )?;
    Ok((vertex, index))
}

/// What the text shader's uniform buffer holds. `atlas` is
/// `(width, height, distance range, mode)`: the first three are what the C++
/// hardcoded (`assets/shaders/Text.slang`), the fourth tells the shader whether
/// to reconstruct a distance field or read plain coverage.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextGlobals {
    pub view_projection: [[f32; 4]; 4],
    pub atlas: [f32; 4],
}

impl TextGlobals {
    pub fn new(view_projection: Mat4, atlas: &Atlas) -> Self {
        Self {
            view_projection: view_projection.to_cols_array_2d(),
            atlas: [
                atlas.width as f32,
                atlas.height as f32,
                atlas.metrics.distance_range,
                if atlas.is_msdf() { 1.0 } else { 0.0 },
            ],
        }
    }
}
