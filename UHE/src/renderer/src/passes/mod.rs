//! The frame's passes, and the seam a backend records them through.
//!
//! Mirrors `UHE/Renderer3D/Renderer3D.cpp` plus the passes the graph makes
//! possible: a depth prepass (which is also the shadow map), the sky clear, the
//! opaque and transparent geometry, ambient occlusion, the editor's grid and
//! gizmo lines, and the composition into the swapchain image.
//!
//! ## One recorder, dispatched by pass name
//!
//! `PassRecorder` is implemented once, here, and both backends call it
//! (`chassis::graph::pass`). A backend knows nothing about geometry; the
//! renderer knows nothing about command pools. The dispatch is by the pass's
//! name because a graph pass carries no closure — that is deliberate, so a
//! compiled frame is pure data a worker thread can own.

pub mod composite;
pub mod editor;
pub mod geometry;
pub mod lines;
pub mod overlay;
pub mod pipeline;
pub mod shadow;
pub mod ssao;
pub mod uniforms;

use std::sync::Arc;

use chassis::graph::{PassRecorder, PassScope, TextureId};
use chassis::{CommandList, Error};

pub use composite::CompositePass;
pub use editor::EditorPass;
pub use geometry::{GeometryPass, MaterialPipelines, PreparedDraw};
pub use lines::{GridPass, GridUniforms, LineBatch, LinePass, LineUniforms, LineVertex};
pub use overlay::{SpriteTextOverlay, PASS_2D, PASS_TEXT};
pub use pipeline::{PipelineCache, PipelineKey, ShaderSet, ShaderVariant};
pub use shadow::ShadowPass;
pub use ssao::{SsaoPass, SsaoUniforms};
pub use uniforms::{BindSet, DrawArena, DrawUniforms, FrameUniforms, ImageSet, ShadowUniforms};

/// Pass names, which are also the keys the dispatch is done on.
pub const PASS_SHADOW: &str = "shadow";
pub const PASS_OPAQUE: &str = "opaque";
/// The blended draws, after the opaques, loading the colour and depth they
/// wrote rather than clearing them.
pub const PASS_TRANSPARENT: &str = "transparent";
pub const PASS_SSAO: &str = "ssao";
/// The grid and the gizmos, in one pass.
pub const PASS_EDITOR: &str = "editor";
pub const PASS_COMPOSITE: &str = "composite";

/// The graph slots this frame's passes are attached to.
///
/// `backbuffer` names the swapchain image, which the *executor* resolves: a
/// swapchain image has no RHI `Texture` handle, so the only object that can turn
/// an acquired image into a barrier target is the backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTargets {
    pub scene_color: TextureId,
    pub scene_depth: TextureId,
    pub shadow: TextureId,
    pub ssao: TextureId,
    pub backbuffer: TextureId,
}

/// Passes this crate does not own: the 2D batch and the text slice.
///
/// `batch2d` and `text` are that slice's; this is the seam, so a frame can
/// include them without this crate knowing how they draw. `declare` is called
/// while the frame's graph is open, `record` for each pass name it declared.
pub trait OverlayPasses: Send + Sync {
    fn declare(&self, graph: &mut chassis::graph::RenderGraph, targets: &FrameTargets);
    fn record(
        &self,
        pass: &str,
        scope: &PassScope<'_>,
        list: &mut dyn CommandList,
    ) -> Result<(), Error>;
}

/// Every recorder of one frame, already prepared.
///
/// Built after the frame's textures and uniform buffers exist, then shared with
/// the backend as one `Arc<dyn PassRecorder>`. Nothing is mutated while
/// recording, which is what lets a backend record passes on worker threads.
pub struct FrameRecorder {
    pub shadow: Option<ShadowPass>,
    pub geometry: Option<GeometryPass>,
    pub transparent: Option<GeometryPass>,
    pub ssao: Option<SsaoPass>,
    pub editor: Option<EditorPass>,
    pub composite: Option<CompositePass>,
    pub overlay: Option<Arc<dyn OverlayPasses>>,
}

impl FrameRecorder {
    pub fn new() -> Self {
        Self {
            shadow: None,
            geometry: None,
            transparent: None,
            ssao: None,
            editor: None,
            composite: None,
            overlay: None,
        }
    }
}

impl Default for FrameRecorder {
    fn default() -> Self {
        Self::new()
    }
}

fn missing(pass: &str) -> Error {
    Error::InvalidDescriptor(format!(
        "pass '{pass}' is in the frame's graph with no recorder behind it"
    ))
}

impl PassRecorder for FrameRecorder {
    fn record(&self, scope: &PassScope<'_>, list: &mut dyn CommandList) -> Result<(), Error> {
        match scope.name {
            PASS_SHADOW => match &self.shadow {
                Some(pass) => pass.record(scope, list),
                None => Err(missing(PASS_SHADOW)),
            },
            PASS_OPAQUE => match &self.geometry {
                Some(pass) => pass.record(scope, list),
                None => Err(missing(PASS_OPAQUE)),
            },
            PASS_TRANSPARENT => match &self.transparent {
                Some(pass) => pass.record(scope, list),
                None => Err(missing(PASS_TRANSPARENT)),
            },
            PASS_SSAO => match &self.ssao {
                Some(pass) => pass.record(scope, list),
                None => Err(missing(PASS_SSAO)),
            },
            PASS_EDITOR => match &self.editor {
                Some(pass) => pass.record(scope, list),
                None => Err(missing(PASS_EDITOR)),
            },
            PASS_COMPOSITE => match &self.composite {
                Some(pass) => pass.record(scope, list),
                None => Err(missing(PASS_COMPOSITE)),
            },
            other => match &self.overlay {
                Some(overlay) => overlay.record(other, scope, list),
                None => Err(Error::InvalidDescriptor(format!(
                    "no recorder for pass '{other}': it is neither a bifrost pass nor one \
                     declared by an installed overlay"
                ))),
            },
        }
    }
}
