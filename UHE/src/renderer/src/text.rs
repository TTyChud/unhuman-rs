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

use glam::{Mat4, Vec4};

use chassis::{Texture, TextureFormat, VertexAttribute, VertexLayout};

use crate::batch2d::{PipelineError, QuadPipeline};
use crate::font::{layout, Atlas, Layout, LayoutOptions};

pub const TEXT_WGSL: &str = include_str!("../../../assets/shaders/text.wgsl");

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
    open: u32,
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
        self.open = 0;
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
        if self.texture != Some(texture) {
            self.close();
            self.texture = Some(texture);
        }

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
        self.open += quads * 6;
    }

    pub fn flush(&mut self) {
        self.close();
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

    fn close(&mut self) {
        if self.open == 0 {
            return;
        }
        let index_offset = self.indices.len() as u32 - self.open;
        self.runs.push(TextRun {
            texture: self.texture.expect("an open run always has a texture"),
            index_offset,
            index_count: self.open,
        });
        self.open = 0;
    }
}

pub fn text_vertex_layout() -> VertexLayout {
    VertexLayout {
        stride: std::mem::size_of::<TextVertex>() as u32,
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
