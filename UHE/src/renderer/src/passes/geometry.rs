//! The geometry pass: opaque draws, then blended ones.
//!
//! Port of `Renderer3D::SubmitMesh` (`Renderer3D.cpp:346-440`): per primitive,
//! bind the pipeline its material selects, bind its uniforms, bind the vertex
//! and index buffers, and draw indexed. The C++ switches pipeline per primitive
//! for exactly one reason — `doubleSided` selects the no-cull variant — and this
//! does the same, for culling and for blending.
//!
//! ## Why the transparent draws are not yet a pass of their own
//!
//! They should be: a second pass, after this one, with its own depth-tested
//! pipeline. Without `PassBuilder::depth_load` that pass would have declared
//! `depth(texture, 1.0)` and *cleared* the depth buffer the opaque draws just
//! wrote — the transparent geometry drawn with no occlusion test at all, which
//! is worse than not splitting. `depth_load` landed, so that is no longer the
//! reason; what is missing is the split itself, and `PASS_TRANSPARENT` is the
//! slot it will record into. Until then this pass draws the opaques and then the
//! blended items back to front, which is what the C++ says it does
//! (`MaterialExtensions.h:29-36`: "drawn after opaques, blended. Needs depth
//! sorting to avoid artefacts where surfaces overlap").

use chassis::{BindGroup, CommandList, PassScope, Pipeline};

use crate::material::MaterialVariant;
use crate::mesh::GpuMesh;

/// Bind group numbers within the model pipeline's shader.
pub const FRAME_SET: u32 = 0;
pub const MATERIAL_SET: u32 = 1;
pub const DRAW_SET: u32 = 2;

/// The four pipelines one geometry pass can bind, one per [`MaterialVariant`].
///
/// The C++ builds two and keeps them in `s_Data3D.ModelPipeline` and
/// `ModelPipelineDoubleSided` (`Renderer3D.cpp:78-95`), for the same reason:
/// cull mode and blending are pipeline state, so a material cannot ask for them
/// per draw.
#[derive(Clone, Copy, Debug)]
pub struct MaterialPipelines {
    pub opaque: Pipeline,
    pub opaque_double_sided: Pipeline,
    pub transparent: Pipeline,
    pub transparent_double_sided: Pipeline,
}

impl MaterialPipelines {
    pub fn get(&self, variant: MaterialVariant) -> Pipeline {
        match variant {
            MaterialVariant::Opaque => self.opaque,
            MaterialVariant::OpaqueDoubleSided => self.opaque_double_sided,
            MaterialVariant::Transparent => self.transparent,
            MaterialVariant::TransparentDoubleSided => self.transparent_double_sided,
        }
    }
}

/// A draw whose buffers, uniforms and material are already resolved.
#[derive(Clone, Copy, Debug)]
pub struct PreparedDraw {
    pub mesh: GpuMesh,
    /// The set that carries this draw's model matrix, tint and entity id.
    pub uniforms: BindGroup,
    /// `None` draws with the default material, which is the C++'s slot 0
    /// (`Renderer3D::RestartMaterialRegion`).
    pub material: Option<BindGroup>,
    pub variant: MaterialVariant,
}

impl PreparedDraw {
    /// Blended draws are drawn after every opaque draw.
    pub fn transparent(&self) -> bool {
        self.variant.blends()
    }
}

pub struct GeometryPass {
    pipelines: MaterialPipelines,
    frame: BindGroup,
    draws: Vec<PreparedDraw>,
}

impl GeometryPass {
    pub fn new(pipelines: MaterialPipelines, frame: BindGroup, draws: Vec<PreparedDraw>) -> Self {
        Self {
            pipelines,
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

        list.bind_group(FRAME_SET, &self.frame);

        // Only rebound when the variant changes: a scene whose materials
        // alternate between culling modes would otherwise pay a pipeline bind
        // per draw. The C++ binds per primitive.
        let mut bound: Option<MaterialVariant> = None;
        for draw in &self.draws {
            if bound != Some(draw.variant) {
                list.bind_pipeline(&self.pipelines.get(draw.variant));
                bound = Some(draw.variant);
            }

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
