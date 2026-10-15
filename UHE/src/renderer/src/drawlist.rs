//! The draw list: what to submit, in what order, as few times as possible.
//!
//! ## Built once, replayed every frame
//!
//! A `DrawList` is `Vec<DrawItem>` plus `Vec<DrawCommand>`, and the two are not
//! the same thing. Items are what the scene said — one per drawable, carrying a
//! transform, bounds and a material. Commands are what the GPU will be asked for,
//! after sorting and after identical meshes have been folded into instanced draws.
//! The list is built once and replayed every frame, because the expensive half is
//! deciding the order and the cheap half is issuing it; re-deciding per frame is
//! how a renderer spends its whole budget on something it already knew.
//!
//! It is *data*. No handles, no closures, no backend.
//! [`crate::replay::DrawResolver`] is the one place ids become `chassis` handles,
//! so the whole list is testable with no device — which matters, because a draw
//! list that cannot be tested is a draw list whose ordering bugs are found on
//! screen.
//!
//! ## Sorting and batching pull in opposite directions, so both are explicit
//!
//! Depth sorting wants items in distance order. Batching wants items in
//! `(pipeline, material, mesh)` order. Those are different permutations of the same
//! set, and a list cannot have both.
//!
//! [`BatchMode::OrderPreserving`] — the default — merges only *adjacent* runs of
//! equal key, so the queue's order survives and a grid of identical meshes still
//! collapses. [`BatchMode::Grouped`] re-sorts by key and folds everything, which is
//! the fewest possible draw calls and the wrong order for early-Z; it is only safe
//! when something else has already sorted the pixels, i.e. a depth pre-pass.
//!
//! Transparency settles it anyway: reordering blended geometry is not a lost
//! optimisation, it is a different image. So [`BatchMode::Grouped`] is refused for
//! an order-dependent queue rather than trusted.

use glam::Mat4;

use crate::bounds::Sphere;
use crate::sort::{self, DepthOrder};

/// A mesh the draw list refers to by id.
///
/// An id and not a `chassis::Buffer`, because the buffer behind a mesh is the
/// backend's business — Phase 2's `ResourceCache` owns handle lifetime, and a draw
/// list holding handles would hold them across frames with no release point. The
/// C++ holds them as raw `void*` (`Renderer3D.cpp:229`) and never frees them,
/// which is the pattern to avoid rather than copy.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct MeshId(pub u32);

/// A material instance. Several meshes sharing one merge their draws.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct MaterialId(pub u32);

/// A pipeline. The primary batch key, because binding one is the expensive
/// change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct PipelineId(pub u32);

/// One thing to draw, before sorting and batching.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DrawItem {
    pub mesh: MeshId,
    pub material: MaterialId,
    pub pipeline: PipelineId,
    /// Object-space bounds, transformed to world when culled.
    pub bounds: Sphere,
    pub transform: Mat4,
    /// Distance from the camera to the world bounds centre.
    ///
    /// Stored rather than recomputed per comparison, because sorting runs per pass
    /// per frame and the camera does not move during a frame. Set it from
    /// [`crate::cull::Camera::distance_to`] at submission time.
    pub depth: f32,
}

impl DrawItem {
    pub fn new(
        mesh: MeshId,
        material: MaterialId,
        pipeline: PipelineId,
        transform: Mat4,
        bounds: Sphere,
    ) -> Self {
        Self {
            mesh,
            material,
            pipeline,
            bounds,
            transform,
            depth: 0.0,
        }
    }

    pub fn with_depth(mut self, depth: f32) -> Self {
        self.depth = depth;
        self
    }

    /// Bounds in world space.
    pub fn world_bounds(&self) -> Sphere {
        self.bounds.transformed(self.transform)
    }
}

/// How far a draw list may go to collapse draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BatchMode {
    /// Merge only items that are already adjacent after sorting.
    #[default]
    OrderPreserving,
    /// Re-sort by batch key and merge everything.
    ///
    /// Fewer draw calls, depth order gone. Safe only behind a depth pre-pass, and
    /// never for a queue whose order is a correctness requirement.
    Grouped,
}

/// One GPU draw: a run of items sharing a pipeline, a material and a mesh.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DrawCommand {
    pub pipeline: PipelineId,
    pub material: MaterialId,
    pub mesh: MeshId,
    /// Index of this run's first item in [`DrawList::items`].
    ///
    /// Where the caller writes this draw's per-instance transforms. The draw list
    /// owns the *order* of instances; the instance buffer is Phase 2's, and a
    /// command that did not say which instances it covered would force the caller
    /// to re-derive it by walking the list again.
    pub first_item: u32,
    pub instances: u32,
}

/// A queue's worth of draws, sorted and batched.
pub struct DrawList {
    order: DepthOrder,
    batch: BatchMode,
    items: Vec<DrawItem>,
    commands: Vec<DrawCommand>,
    built: bool,
}

impl DrawList {
    pub fn new(order: DepthOrder) -> Self {
        Self::with_batch_mode(order, BatchMode::default())
    }

    pub fn with_batch_mode(order: DepthOrder, batch: BatchMode) -> Self {
        Self {
            order,
            batch,
            items: Vec::new(),
            commands: Vec::new(),
            built: false,
        }
    }

    pub fn order(&self) -> DepthOrder {
        self.order
    }

    /// Queue something to draw, invalidating the built commands.
    ///
    /// They are rebuilt by [`Self::build`] rather than patched: patching a batch is
    /// how `first_item` drifts away from the item it points at, and a drifted
    /// `first_item` puts an instance's transform on a different instance — a
    /// correct-looking frame with one object somewhere else.
    pub fn push(&mut self, item: DrawItem) {
        self.items.push(item);
        self.built = false;
    }

    pub fn items(&self) -> &[DrawItem] {
        &self.items
    }

    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// True when [`Self::commands`] reflects the current items.
    ///
    /// A caller replaying stale commands after a push is drawing something that is
    /// no longer in the scene, which reads as a ghost rather than as an error, so
    /// this is a question the type is willing to answer.
    pub fn is_built(&self) -> bool {
        self.built
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.commands.clear();
        self.built = false;
    }

    pub fn total_instances(&self) -> u32 {
        self.items.len() as u32
    }

    /// Sort, then fold equal keys into instanced commands.
    ///
    /// `Vec::clear`, never a fresh `Vec`: the list is rebuilt every frame and a
    /// fresh allocation per frame is a malloc on the critical path. Sorting is the
    /// same story — the comparisons dominate, so the allocations are worth not
    /// having.
    pub fn build(&mut self) {
        sort::sort(&mut self.items, self.order);
        if self.effective_batch_mode() == BatchMode::Grouped {
            // Keep each key's items in the order the depth sort left them in,
            // and order the groups by their first appearance in that order. A
            // plain `sort_by_key(batch_key)` would instead order groups by key
            // value, which moves a group that first appeared at depth 1 behind
            // one that first appeared at depth 2 whenever its key is larger --
            // a reordering of the queue that is more than grouping, and that
            // the depth sort did not ask for.
            let mut group_order: Vec<(PipelineId, MaterialId, MeshId)> = Vec::new();
            for item in &self.items {
                let key = sort::batch_key(item);
                if !group_order.contains(&key) {
                    group_order.push(key);
                }
            }
            self.items.sort_by_key(|item| {
                group_order
                    .iter()
                    .position(|key| *key == sort::batch_key(item))
                    .unwrap_or(group_order.len())
            });
        }

        self.commands.clear();
        for index in 0..self.items.len() {
            let item = self.items[index];
            // Only the *open* command can be extended. Merging with any earlier
            // command of the same key is what `BatchMode::Grouped` achieves, by
            // having already sorted those items next to each other.
            let merges_into_open = self.commands.last().is_some_and(|open: &DrawCommand| {
                open.pipeline == item.pipeline
                    && open.material == item.material
                    && open.mesh == item.mesh
            });
            if merges_into_open {
                if let Some(open) = self.commands.last_mut() {
                    open.instances += 1;
                }
            } else {
                self.commands.push(DrawCommand {
                    pipeline: item.pipeline,
                    material: item.material,
                    mesh: item.mesh,
                    first_item: index as u32,
                    instances: 1,
                });
            }
        }
        self.built = true;
    }

    /// [`BatchMode::Grouped`] refused where order is correctness.
    ///
    /// A panic would be the honest response to "group a transparent queue", but the
    /// caller is a renderer with a frame to finish, and dropping the frame is worse
    /// than the wrong picture the request was trying to avoid. So the request is
    /// downgraded and the queue keeps an order that is merely slower.
    fn effective_batch_mode(&self) -> BatchMode {
        if self.order.is_order_dependent() {
            BatchMode::OrderPreserving
        } else {
            self.batch
        }
    }

    /// Replay the built commands.
    ///
    /// Delegates to [`crate::replay::record`]: the ordering decision is here, the
    /// state-change decisions are there.
    pub fn record(
        &self,
        list: &mut dyn chassis::CommandList,
        resolve: &dyn crate::replay::DrawResolver,
        tracker: &mut crate::stats::StateTracker,
    ) -> chassis::Result<crate::replay::DrawCount> {
        crate::replay::record(list, &self.commands, resolve, tracker)
    }
}
