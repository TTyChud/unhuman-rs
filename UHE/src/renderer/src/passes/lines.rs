//! The editor's grid, and gizmo lines.
//!
//! Port of `Renderer3D::DrawGrid` (`Renderer3D.cpp:243-262`) — six vertices from
//! `Grid.slang`, with the view-projection and its inverse, and the shader
//! reconstructing a world-space ray per pixel and drawing a fading grid on the
//! y = 0 plane. The C++ passed the two matrices as push constants; this RHI
//! cannot declare a push-constant range, so they are a uniform block, which is
//! the same data by another route.
//!
//! The gizmo half is new: the C++ draws nothing but the grid, and an editor with
//! nothing else to draw lines with hand-rolled its own pass. A line list with a
//! per-vertex colour is the smallest thing that draws a gizmo.

use bytemuck::{Pod, Zeroable};
use chassis::{BindGroup, Buffer, CommandList, PassScope, Pipeline};

/// Bind group number within the grid shader.
pub const GRID_SET: u32 = 0;
/// Bind group number within the line shader.
pub const LINE_DRAW_SET: u32 = 0;

/// The two matrices `Grid.slang`'s push-constant block held.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GridUniforms {
    pub view_projection: [[f32; 4]; 4],
    pub inverse_view_projection: [[f32; 4]; 4],
}

impl GridUniforms {
    pub fn new(view_projection: [[f32; 4]; 4]) -> Self {
        Self {
            view_projection,
            inverse_view_projection: glam::Mat4::from_cols_array_2d(&view_projection)
                .inverse()
                .to_cols_array_2d(),
        }
    }
}

/// One end of a gizmo line.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// The camera and the batch's own placement, which `line.wgsl` reads.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LineUniforms {
    pub view_projection: [[f32; 4]; 4],
    pub model: [[f32; 4]; 4],
}

impl LineUniforms {
    pub fn new(view_projection: [[f32; 4]; 4], model: [[f32; 4]; 4]) -> Self {
        Self {
            view_projection,
            model,
        }
    }
}

/// The C++ grid pipeline, drawn as six vertices with no vertex buffer: the
/// quad comes from `@builtin(vertex_index)`.
pub struct GridPass {
    pipeline: Pipeline,
    group: BindGroup,
}

impl GridPass {
    pub fn new(pipeline: Pipeline, group: BindGroup) -> Self {
        Self { pipeline, group }
    }

    pub fn record(
        &self,
        _scope: &PassScope<'_>,
        list: &mut dyn CommandList,
    ) -> Result<(), chassis::Error> {
        list.bind_pipeline(&self.pipeline);
        list.bind_group(GRID_SET, &self.group);
        list.draw(6, 1);
        Ok(())
    }
}

/// The line batch's buffers, uploaded once by whoever builds the gizmos.
#[derive(Clone, Copy, Debug)]
pub struct LineBatch {
    pub vertex: Buffer,
    pub index: Buffer,
    pub index_count: u32,
}

impl LineBatch {
    pub fn is_empty(&self) -> bool {
        self.index_count == 0
    }
}

/// A line-list pass over a prepared batch.
pub struct LinePass {
    pipeline: Pipeline,
    /// Set 0: the batch's model matrix, so a gizmo can be placed.
    uniforms: BindGroup,
    batch: LineBatch,
}

impl LinePass {
    pub fn new(pipeline: Pipeline, uniforms: BindGroup, batch: LineBatch) -> Self {
        Self {
            pipeline,
            uniforms,
            batch,
        }
    }

    pub fn record(
        &self,
        _scope: &PassScope<'_>,
        list: &mut dyn CommandList,
    ) -> Result<(), chassis::Error> {
        if self.batch.is_empty() {
            return Ok(());
        }

        list.bind_pipeline(&self.pipeline);
        list.bind_group(LINE_DRAW_SET, &self.uniforms);
        list.bind_vertex_buffer(0, &self.batch.vertex, 0);
        list.bind_index_buffer(&self.batch.index, 0);
        list.draw_indexed(self.batch.index_count, 1);
        Ok(())
    }
}

/// Packs vertices and indices for a line batch. Pure, so the interleave is
/// testable without a device.
pub fn pack(vertices: &[LineVertex], indices: &[u32]) -> (Vec<u8>, Vec<u8>) {
    (
        bytemuck::cast_slice(vertices).to_vec(),
        bytemuck::cast_slice(indices).to_vec(),
    )
}
