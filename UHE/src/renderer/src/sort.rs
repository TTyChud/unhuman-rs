//! Submission order: front-to-back for opaque, back-to-front for transparent.
//!
//! ## The two orders are opposite, and both are load-bearing
//!
//! **Opaque is front-to-back** so the depth test rejects as many fragments as
//! possible before they are shaded. Every pixel drawn twice is wasted shading
//! work, and which of the two draws wins is irrelevant when both write depth and
//! only the nearer survives. On a forward renderer with expensive lighting this is
//! the difference between shading every pixel once and shading it twice.
//!
//! **Transparent is back-to-front** because there is no depth rejection to lean
//! on. Blending is `dst = src * srcAlpha + dst * (1 - srcAlpha)`, which is
//! *not* commutative: drawing the near pane first and the far pane second
//! replaces the near pane with a blend of itself and something behind it. Change
//! the order and the image changes. No amount of "it looked fine" testing finds
//! this, because the result is wrong in a way that is plausible-looking.
//!
//! So the ordering is not an optimisation with a tunable dial. For one queue it
//! is a fill-rate win and for the other it is a correctness requirement, and the
//! code that sorts them is two functions rather than one with a boolean.
//!
//! ## What the C++ did
//!
//! Nothing. `Renderer3D::SubmitMesh` (`Renderer3D.cpp:224-300`) draws every
//! primitive of every mesh in submission order: no cull test, no sort, no batch.
//! Because it writes depth for opaque geometry it is *correct* — the depth test
//! sorts the pixels for it — and it is also the slowest possible order for them,
//! because nothing gets rejected before it is shaded. Transparent geometry in
//! that loop is not correct at all; it happens to look right when a scene draws
//! its panes in a convenient order, which is a property of the scene rather than
//! of the renderer.
//!
//! ## Why the tie-break is the batching key
//!
//! Ties are common — a grid of objects at one depth, a row of crates — and the
//! tie-break is where free batching comes from. Equal-depth items are ordered by
//! `(pipeline, material, mesh)`, which is exactly the key the batcher groups by,
//! so a tie group collapses into one draw for free. Ordering ties arbitrarily
//! instead would scatter every batch across the list.

use crate::drawlist::{DrawItem, MaterialId, MeshId, PipelineId};

/// Which queue a draw list belongs to, and therefore how it is ordered.
///
/// Two values because two behaviours. A third — "does not matter" — would be a
/// flag the caller could forget to set, defaulting to one of the two answers and
/// being wrong for whichever queue it was wrong for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum DepthOrder {
    /// Depth-tested, depth-writing. Front-to-back.
    #[default]
    Opaque,
    /// Alpha-blended. Back-to-front, and never reordered across a depth.
    Transparent,
}

impl DepthOrder {
    /// True when the order is a correctness requirement rather than a saving.
    pub fn is_order_dependent(self) -> bool {
        matches!(self, DepthOrder::Transparent)
    }
}

/// Sort a draw list for its queue.
///
/// Stable, so items that compare equal keep submission order and the result
/// depends only on the input. Instanced transforms are per-instance data, so
/// "equal" is not always "indistinguishable" — a stable sort keeps a caller's
/// instance order intact instead of shuffling it.
///
/// Depth is compared with `total_cmp` rather than `partial_cmp`. `sort_by`
/// requires a *total* order; a comparator that returns `None` — which is what
/// `partial_cmp` does for a NaN — leaves the result unspecified, and one that
/// says "equal" for values that are not can make the standard library's sort
/// panic outright. A NaN depth is reachable from a degenerate projection matrix
/// or a zero-scale model transform, and it should cost one misplaced draw rather
/// than an unspecified frame.
pub fn sort(items: &mut [DrawItem], order: DepthOrder) {
    match order {
        DepthOrder::Opaque => sort_opaque(items),
        DepthOrder::Transparent => sort_transparent(items),
    }
}

/// Nearest first.
pub fn sort_opaque(items: &mut [DrawItem]) {
    items.sort_by(|a, b| {
        a.depth
            .total_cmp(&b.depth)
            .then_with(|| batch_key(a).cmp(&batch_key(b)))
    });
}

/// Farthest first.
pub fn sort_transparent(items: &mut [DrawItem]) {
    items.sort_by(|a, b| {
        b.depth
            .total_cmp(&a.depth)
            .then_with(|| batch_key(a).cmp(&batch_key(b)))
    });
}

/// The grouping key, as one comparable value.
///
/// A tuple rather than three chained comparisons because the ordering is read in
/// the source: pipeline first, then material, then mesh. That order is not
/// arbitrary — pipeline changes are the expensive rebind, material changes are a
/// descriptor set, and mesh changes are two buffer binds — so the list front
/// loads the most expensive kind of change as rarely as it can.
pub fn batch_key(item: &DrawItem) -> (PipelineId, MaterialId, MeshId) {
    (item.pipeline, item.material, item.mesh)
}
