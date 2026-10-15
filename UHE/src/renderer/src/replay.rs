//! Replay: turning commands back into command-list calls, without the redundant ones.
//!
//! ## The split, and why it is a split
//!
//! [`crate::drawlist::DrawList`] decides *what* to submit and in what order. This
//! file decides how many of those submissions the driver has to act on. Both
//! halves are needed and they are not the same number: a correctly sorted list
//! that re-binds the same pipeline on every command still costs the driver a
//! pipeline bind per command, which is the single most expensive thing in the
//! frame and the thing the C++ does unconditionally
//! (`Renderer3D.cpp:240`).
//!
//! Keeping them apart is what makes the counters mean something.
//! [`crate::stats::StateChanges::pipelines`] counts what this file issued;
//! `DrawList::commands().len()` counts what the sort and the batcher produced.
//! When they disagree, the difference is exactly the redundancy being removed.
//!
//! ## Nothing here opens a render scope
//!
//! The executor does that, from the pass's compiled attachments
//! (`VulkanRenderGraphExecutor.rs:166-171`). A pass that opened its own could open
//! it on the wrong attachments, and the failure reports itself as
//! `vkCmdDraw must be issued inside an active render pass` with no mention of
//! which pass. Both backends read their attachments from the same compiled
//! `RenderPassDesc`, which is the other half of why the two produce the same
//! image.

use crate::drawlist::{DrawCommand, MaterialId, MeshId, PipelineId};
use crate::stats::StateTracker;

/// Draw calls and instances one replay issued.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DrawCount {
    pub draws: u32,
    pub instances: u32,
}

/// Ids to the handles the RHI wants.
///
/// The one place replay touches `chassis` types, and a trait rather than a struct
/// so a test resolver is a dozen lines and a real one can sit behind a cache, a
/// lock or a content hash. [`chassis::CommandList`] still does the binding; this
/// only translates, and translating is where the missing-handle error belongs.
///
/// Phase 2's `ResourceCache` is the production implementation. Keeping it a trait
/// rather than a field is what lets the draw list be tested with no device — and
/// ordering and batching bugs are invisible until they are tested, because they
/// show up as a frame rate rather than as a crash.
pub trait DrawResolver {
    fn pipeline(&self, id: PipelineId) -> Option<chassis::Pipeline>;
    /// The descriptor set for a material, bound at set 0.
    fn bind_group(&self, id: MaterialId) -> Option<chassis::BindGroup>;
    /// Vertex buffer handle and its offset, in bytes.
    fn vertex_buffer(&self, mesh: MeshId) -> Option<(chassis::Buffer, u64)>;
    /// Index buffer handle and its offset, in bytes.
    fn index_buffer(&self, mesh: MeshId) -> Option<(chassis::Buffer, u64)>;
    /// Indices in the mesh — what `draw_indexed` counts, not vertices and not
    /// triangles.
    fn index_count(&self, mesh: MeshId) -> u32;
}

/// Issue the commands, binding only what changed.
///
/// A missing handle is an error rather than a skipped draw. Skipping would turn
/// "the resource cache missed an entry" into "an object is mysteriously
/// invisible", which is the same class of bug as the C++'s mesh handles, which
/// are created once and never destroyed (`Renderer3D.cpp:229`): a leak that shows
/// up as pixels, never as a message.
///
/// The state cache is the caller's, and it is reset per pass by
/// [`StateTracker::begin_pass`], because each pass records into its own command
/// list and nothing a previous pass bound survives into this one.
pub fn record(
    list: &mut dyn chassis::CommandList,
    commands: &[DrawCommand],
    resolve: &dyn DrawResolver,
    tracker: &mut StateTracker,
) -> chassis::Result<DrawCount> {
    let mut count = DrawCount::default();
    for command in commands {
        let pipeline = resolve.pipeline(command.pipeline).ok_or_else(|| {
            chassis::Error::InvalidDescriptor(format!("no pipeline for {:?}", command.pipeline))
        })?;
        let group = resolve.bind_group(command.material).ok_or_else(|| {
            chassis::Error::InvalidDescriptor(format!(
                "no bind group for material {:?}",
                command.material
            ))
        })?;
        let (vertex, vertex_offset) = resolve.vertex_buffer(command.mesh).ok_or_else(|| {
            chassis::Error::InvalidDescriptor(format!("no vertex buffer for {:?}", command.mesh))
        })?;
        let (index, index_offset) = resolve.index_buffer(command.mesh).ok_or_else(|| {
            chassis::Error::InvalidDescriptor(format!("no index buffer for {:?}", command.mesh))
        })?;
        let indices = resolve.index_count(command.mesh);

        // Check the tracker first and issue only on a change. Inverted — issue
        // then count — this would work and would report every draw as a state
        // change, which is the counter going back to reporting nothing.
        if tracker.bind_pipeline(&pipeline) {
            list.bind_pipeline(&pipeline);
        }
        if tracker.bind_group(0, &group) {
            list.bind_group(0, &group);
        }
        if tracker.bind_vertex_buffer(&vertex, vertex_offset) {
            list.bind_vertex_buffer(0, &vertex, vertex_offset);
        }
        if tracker.bind_index_buffer(&index, index_offset) {
            list.bind_index_buffer(&index, index_offset);
        }
        list.draw_indexed(indices, command.instances);

        count.draws += 1;
        count.instances += command.instances;
    }
    Ok(count)
}
