//! Composition: the scene colour and the occlusion into the swapchain image.
//!
//! The C++ draws its geometry straight into the swapchain image, so it has no
//! pass corresponding to this one. Two things in the Rust port need it:
//!
//! * the AO texture has to be *applied* somewhere, and the graph refuses a pass
//!   that reads and writes one resource, so it cannot be applied to the colour
//!   target it read (`compiler.rs`, `ErrorKind::PassStateConflict`);
//! * the 2D and text slice draws into the scene colour rather than into the
//!   swapchain, so something has to put the result there.
//!
//! One full-screen triangle, no vertex buffer.

use chassis::{BindGroup, CommandList, PassScope, Pipeline};

/// Bind group number within the composite shader.
pub const COMPOSITE_SET: u32 = 0;

pub struct CompositePass {
    pipeline: Pipeline,
    group: BindGroup,
}

impl CompositePass {
    pub fn new(pipeline: Pipeline, group: BindGroup) -> Self {
        Self { pipeline, group }
    }

    pub fn record(&self, _scope: &PassScope<'_>, list: &mut dyn CommandList) -> Result<(), chassis::Error> {
        list.bind_pipeline(&self.pipeline);
        list.bind_group(COMPOSITE_SET, &self.group);
        list.draw(3, 1);
        Ok(())
    }
}
