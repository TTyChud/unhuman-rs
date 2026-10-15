//! The depth prepass, and the shadow map it produces.
//!
//! The C++ has no prepass and no shadow map: `Renderer3D` draws straight into
//! the editor's colour and depth targets, and `SSAO` is an empty placeholder
//! (`SSAO.cpp:7` is `void SSAO::init() {}`). This is the pass the renderer needs
//! for both: a depth-only draw of every shadow-casting mesh, from whichever
//! matrix the caller gives it. The camera's own depth prepass is the same pass
//! with the camera's matrix, so there is one implementation, not two.
//!
//! A depth-only pipeline has one stage: `ShaderSet::fragment_for` returns `None`
//! for it, which is what Vulkan wants and what the C++ would have had to spell
//! out with two shader modules.

use chassis::{BindGroup, CommandList, PassScope, Pipeline};

use super::geometry::PreparedDraw;

/// Bind group numbers within the depth pipeline's shader.
pub const SHADOW_SET: u32 = 0;
pub const DRAW_SET: u32 = 1;

pub struct ShadowPass {
    pipeline: Pipeline,
    /// Set 0: the light-space view-projection the depth is measured against.
    frame: BindGroup,
    draws: Vec<PreparedDraw>,
}

impl ShadowPass {
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
        list.bind_group(SHADOW_SET, &self.frame);

        for draw in &self.draws {
            list.bind_group(DRAW_SET, &draw.uniforms);
            list.bind_vertex_buffer(0, &draw.mesh.vertex, 0);
            list.bind_index_buffer(&draw.mesh.index, 0);
            list.draw_indexed(draw.mesh.index_count, 1);
        }

        Ok(())
    }
}
