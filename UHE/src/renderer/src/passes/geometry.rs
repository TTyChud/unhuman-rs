//! The opaque and transparent geometry passes.
//!
//! Port of `Renderer3D::SubmitMesh` (`Renderer3D.cpp:346-440`): for every
//! primitive, bind the pipeline its material selects, bind its uniforms, bind
//! the vertex and index buffers, and draw indexed. One difference the RHI
//! forces: the C++ picked a *pipeline* per primitive and pushed constants; here
//! the per-draw values are a descriptor set, so each draw carries the group that
//! holds them.
//!
//! The C++ also had no transparent pass — its `MaterialExtensions.h:29-36`
//! records that transmission "needs a sorted pass to look right; until one
//! exists, the renderer draws these in submission order". This one is sorted
//! back to front, which is what that comment asks for.

use chassis::{BindGroup, CommandList, PassScope, Pipeline};

use crate::mesh::GpuMesh;

/// Bind group numbers within the model pipeline's shader.
pub const FRAME_SET: u32 = 0;
pub const MATERIAL_SET: u32 = 1;
pub const DRAW_SET: u32 = 2;

/// A draw whose buffers, uniforms and material are already resolved.
#[derive(Clone, Copy, Debug)]
pub struct PreparedDraw {
    pub mesh: GpuMesh,
    /// The set that carries this draw's model matrix, tint and entity id.
    pub uniforms: BindGroup,
    /// `None` draws with the default material, which is the C++'s slot 0
    /// (`Renderer3D::RestartMaterialRegion`).
    pub material: Option<BindGroup>,
}

/// Records one variant of the geometry pass.
pub struct GeometryPass {
    pipeline: Pipeline,
    frame: BindGroup,
    draws: Vec<PreparedDraw>,
}

impl GeometryPass {
    pub fn new(pipeline: Pipeline, frame: BindGroup, draws: Vec<PreparedDraw>) -> Self {
        Self {
            pipeline,
            frame,
            draws,
        }
    }

    pub fn draws(&self) -> usize {
        self.draws.len()
    }

    pub fn record(&self, _scope: &PassScope<'_>, list: &mut dyn CommandList) -> Result<(), chassis::Error> {
        if self.draws.is_empty() {
            return Ok(());
        }

        list.bind_pipeline(&self.pipeline);
        list.bind_group(FRAME_SET, &self.frame);

        for draw in &self.draws {
            if let Some(material) = draw.material {
                list.bind_group(MATERIAL_SET, &material);
            }
            list.bind_group(DRAW_SET, &draw.uniforms);
            list.bind_vertex_buffer(0, &draw.mesh.vertex, 0);
            list.bind_index_buffer(&draw.mesh.index, 0);
            list.draw_indexed(draw.mesh.index_count, 1);
        }

        Ok(())
    }
}
