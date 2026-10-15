//! Reading poses back out of the solver.
//!
//! # The sleeping-body decision, stated once
//!
//! Upstream:
//!
//! ```cpp
//! JPH::BodyID bodyID(rb3d.RuntimeBodyID);
//! if (!bodyInterface->IsActive(bodyID))
//!     continue;                                   // <- this line
//! transform.Translation = ...;
//! transform.Rotation = ...;
//! ```
//!
//! A body that goes to sleep is never read back, so the ECS transform keeps
//! whatever value it had when the body was last awake. The body is frozen at
//! some pose inside the solver; the transform is frozen at some *other* pose,
//! and the two are not required to agree. Nothing recomputes them, because the
//! only code that reads the solver is the code that is now skipping it. The
//! scene keeps simulating correctly and renders a body that is subtly, slowly,
//! permanently in the wrong place — with no error, no warning, and a symptom
//! that looks like a parenting bug rather than a physics one.
//!
//! **Decision: always read the pose back, sleeping or not.** Three reasons, in
//! order of how much they matter:
//!
//! 1. **A sleeping pose is a valid pose.** "Is the solver integrating this
//!    body" and "does this body have a pose" are different questions. rapier
//!    answers the first with `is_sleeping` and the second unconditionally, and
//!    conflating them is the bug. Conflation only looks harmless because a
//!    body that fell asleep without moving has the same pose it had awake.
//!    Set a body to sleep after teleporting it, or wake it and sleep it inside
//!    one frame, and the transform is stale by construction.
//! 2. **Reading a sleeping body costs less than simulating one.** It is a hash
//!    lookup in `RigidBodySet` plus seven float copies. The island was skipped,
//!    so the solver did no work at all for that body; the read cannot be the
//!    expensive part.
//! 3. **The expensive thing is rendering, and that is the caller's choice.**
//!    [`BodyState::sleeping`] is reported, so an engine that wants the upstream
//!    optimisation can skip *submission* for a sleeping body and get the win
//!    that upstream was probably reaching for — without giving up correctness.
//!    Upstream had one flag doing two jobs and could not separate them; two
//!    pieces of information let a caller choose.
//!
//! The cost of this decision is one line of documentation that will be read
//! once and then never again, which is the right trade for removing a class of
//! bug that cannot be detected from inside the physics code.

use crate::bodies::BodyId;
use crate::registry::{BodyRegistry, BodyRegistry2};

/// A 3D body's pose and whether the solver is currently integrating it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodyState {
    /// World position.
    pub translation: glam::Vec3,
    /// World orientation.
    pub rotation: glam::Quat,
    /// Linear velocity, world space, as the solver last computed it. Zero for
    /// a body at rest, which is what a sleeping body reports.
    pub linvel: glam::Vec3,
    /// `true` if the solver has put this body to sleep.
    ///
    /// Reported, not acted on. See the module docs.
    pub sleeping: bool,
}

/// A 2D body's pose. The 2D form of [`BodyState`]; see it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodyState2 {
    /// World position on the XY plane.
    pub translation: glam::Vec2,
    /// World rotation, **radians**, counter-clockwise about +Z.
    pub rotation: f32,
    /// Linear velocity on the XY plane.
    pub linvel: glam::Vec2,
    /// `true` if the solver has put this body to sleep.
    pub sleeping: bool,
}

/// What one read-back pass found. Counts only, so a caller can decide whether
/// anything is worth doing without allocating.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SyncStats {
    /// Bodies read.
    pub read: u32,
    /// Of those, awake.
    pub awake: u32,
    /// Of those, asleep.
    pub sleeping: u32,
    /// Of those, awake and *moving* — non-zero velocity, which is the subset a
    /// caller might use to decide what needs waking.
    pub moving: u32,
}

impl SyncStats {
    fn record3(&mut self, state: &BodyState) {
        self.read += 1;
        if state.sleeping {
            self.sleeping += 1;
        } else {
            self.awake += 1;
            if state.linvel != glam::Vec3::ZERO {
                self.moving += 1;
            }
        }
    }
}

/// Read one 3D body's state.
///
/// Returns `None` for a stale handle, which is the normal answer after a
/// despawn and is not an error.
pub fn read3(
    world: &rapier3d::pipeline::PhysicsWorld,
    handle: rapier3d::dynamics::RigidBodyHandle,
) -> Option<BodyState> {
    let body = world.bodies.get(handle)?;
    Some(BodyState {
        translation: crate::convert::from_vec3(body.translation()),
        rotation: crate::convert::from_quat(*body.rotation()),
        linvel: crate::convert::from_vec3(body.linvel()),
        sleeping: body.is_sleeping(),
    })
}

/// Read one 2D body's state.
pub fn read2(
    world: &rapier2d::pipeline::PhysicsWorld,
    handle: rapier2d::dynamics::RigidBodyHandle,
) -> Option<BodyState2> {
    let body = world.bodies.get(handle)?;
    Some(BodyState2 {
        translation: crate::convert::from_vec2(body.translation()),
        rotation: crate::convert::from_angle(*body.rotation()),
        linvel: crate::convert::from_vec2(body.linvel()),
        sleeping: body.is_sleeping(),
    })
}

/// Read every registered 3D body into `out`, which is cleared first.
///
/// `out` is a caller-owned buffer so the frame loop allocates nothing here.
/// Clearing rather than appending is deliberate: a stale entry in this buffer
/// is an entity that has been despawned and is still being drawn.
pub fn read_all3(
    world: &rapier3d::pipeline::PhysicsWorld,
    registry: &BodyRegistry,
    out: &mut Vec<(BodyId, BodyState)>,
) -> SyncStats {
    out.clear();
    let mut stats = SyncStats::default();
    for (id, slot) in registry.iter() {
        if let Some(state) = read3(world, slot.body) {
            stats.record3(&state);
            out.push((id, state));
        }
    }
    stats
}

/// Read every registered 2D body into `out`. See [`read_all3`].
pub fn read_all2(
    world: &rapier2d::pipeline::PhysicsWorld,
    registry: &BodyRegistry2,
    out: &mut Vec<(BodyId, BodyState2)>,
) -> SyncStats {
    out.clear();
    let mut stats = SyncStats::default();
    for (id, slot) in registry.iter() {
        if let Some(state) = read2(world, slot.body) {
            stats.read += 1;
            if state.sleeping {
                stats.sleeping += 1;
            } else {
                stats.awake += 1;
                if state.linvel != glam::Vec2::ZERO {
                    stats.moving += 1;
                }
            }
            out.push((id, state));
        }
    }
    stats
}
