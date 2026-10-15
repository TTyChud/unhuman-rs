//! The 2D façade.
//!
//! Split out of [`crate::world`] rather than sharing it, for the reason the
//! rest of this crate is split: 3D and 2D are two solvers with two shape sets
//! and two handle types, and merging them into one generic `PhysicsWorld<D>`
//! would need a trait parameterised over rapier types that share no API. The
//! duplication that remains is the spawn/teardown/step boilerplate, and that
//! duplication is *checkable* — the leak tests run the same assertions against
//! both worlds, which is the thing a shared generic would have made awkward to
//! write.
//!
//! Everything about *policy* is shared, not duplicated: the clock is
//! [`crate::timestep::FixedStep`], the locks are [`crate::solver`], and the
//! read-back rule is [`crate::sync`]. What is here is the plumbing that
//! connects a 2D solver to the same policy.
//!
//! # Rotation is radians, always
//!
//! The C++ writes `b2MakeRot(transform.Rotation.z)` where
//! `TransformComponent::Rotation` is documented "assumed in degrees". Box2D v3
//! takes radians. So every 2D body in the C++ is rotated by `degrees *
//! 0.01745`, and it is correct only at `z = 0` — which is why the bug survives:
//! an unrotated 2D scene is indistinguishable from a correct one. There is no
//! degrees anywhere in femur's 2D API, so the mistake has nowhere to be made.

/// A 2D physics world.
///
/// The same façade over rapier2d, box-only. See the module docs for why 2D is
/// box-only and [`crate::sync`] for why `read_all2` is unconditional.
use glam::Vec2;

use rapier2d::pipeline::PhysicsWorld as Rapier2;

use crate::bodies::{build_body2, mass_shares, BodyDesc2, BodyId, BodyKind, SpawnError};
use crate::colliders::{build2, ColliderDesc2};
use crate::convert::Pose2;
use crate::registry::BodyRegistry2;
use crate::solver::{self, Dofs2};
use crate::sync::{read_all2, BodyState2, SyncStats};
use crate::timestep::{FixedStep, StepStats};

/// A 2D physics world: rapier2d, a key registry, and a fixed-step clock.
pub struct PhysicsWorld2 {
    rapier: Rapier2,
    registry: BodyRegistry2,
    clock: FixedStep,
}

impl PhysicsWorld2 {
    /// A 2D world with `gravity` and the default 60 Hz clock.
    pub fn new(gravity: Vec2) -> Self {
        Self::with_clock(gravity, FixedStep::default_rate())
    }

    /// A 2D world with `gravity` and a chosen clock.
    pub fn with_clock(gravity: Vec2, clock: FixedStep) -> Self {
        let mut rapier = Rapier2::new();
        rapier.gravity = crate::convert::to_vec2(gravity);
        rapier.integration_parameters.dt = clock.fixed_dt();
        Self {
            rapier,
            registry: BodyRegistry2::new(),
            clock,
        }
    }

    /// The clock.
    pub fn clock(&self) -> &FixedStep {
        &self.clock
    }

    /// Read-only access to the solver.
    pub fn rapier(&self) -> &Rapier2 {
        &self.rapier
    }

    /// How many bodies this world owns.
    pub fn body_count(&self) -> usize {
        self.registry.len()
    }

    /// How many colliders the solver holds.
    pub fn collider_count(&self) -> usize {
        self.rapier.colliders.len()
    }

    /// Whether a key is registered.
    pub fn contains(&self, id: BodyId) -> bool {
        self.registry.contains(id)
    }

    /// Spawn a 2D body with at most one box collider.
    ///
    /// `colliders` is a slice for the same reason as in [`PhysicsWorld::spawn`]:
    /// `&[]` for a shape-less body, `&[one]` for the C++'s box. rapier2d
    /// supports compound polygons; that is not reachable from here because no
    /// scene component describes one.
    pub fn spawn(
        &mut self,
        id: BodyId,
        desc: &BodyDesc2,
        colliders: &[ColliderDesc2],
        pose: Pose2,
        scale: Vec2,
    ) -> Result<(), SpawnError> {
        if self.registry.contains(id) {
            return Err(SpawnError::AlreadyExists(id));
        }
        let body = self.rapier.insert_body(build_body2(desc, pose, id));

        let mut handles = Vec::with_capacity(colliders.len());
        for collider in colliders {
            let builder = build2(collider, scale, desc.friction, desc.restitution);
            handles.push(self.rapier.insert_collider(builder.build(), Some(body)));
        }

        if desc.kind == BodyKind::Dynamic && !handles.is_empty() {
            let volumes: Vec<f32> = handles
                .iter()
                .map(|h| self.rapier.colliders.get(*h).map_or(0.0, |c| c.volume()))
                .collect();
            for (handle, share) in handles.iter().zip(mass_shares(&volumes, desc.mass)) {
                if let Some(collider) = self.rapier.colliders.get_mut(*handle) {
                    collider.set_mass(share);
                }
            }
        }

        self.registry.insert(id, body, handles.into_iter().next())
    }

    /// Remove a body and everything attached to it. Idempotent; see
    /// [`PhysicsWorld::despawn`].
    pub fn despawn(&mut self, id: BodyId) -> bool {
        let Some(slot) = self.registry.remove(id) else {
            return false;
        };
        self.rapier.remove_body(slot.body).is_some()
    }

    /// Apply per-axis locks to one 2D body.
    pub fn set_dofs(&mut self, id: BodyId, dofs: Dofs2, wake: bool) -> bool {
        match self.registry.get(id) {
            Some(slot) => solver::apply_dofs2(&mut self.rapier, slot.body, dofs, wake),
            None => false,
        }
    }

    /// Scale gravity for one 2D body.
    pub fn set_gravity_scale(&mut self, id: BodyId, scale: f32, wake: bool) -> bool {
        match self.registry.get(id) {
            Some(slot) => solver::set_gravity_scale2(&mut self.rapier, slot.body, scale, wake),
            None => false,
        }
    }

    /// Point a 2D kinematic body at a world-space target.
    pub fn drive_kinematic(&mut self, id: BodyId, target: Pose2) -> bool {
        match self.registry.get(id) {
            Some(slot) => solver::drive_kinematic2(&mut self.rapier, slot.body, target),
            None => false,
        }
    }

    /// Teleport a 2D body, waking it. Rotation is in radians.
    pub fn set_pose(&mut self, id: BodyId, pose: Pose2, wake: bool) -> bool {
        let Some(slot) = self.registry.get(id) else {
            return false;
        };
        let Some(body) = self.rapier.bodies.get_mut(slot.body) else {
            return false;
        };
        body.set_translation(crate::convert::to_vec2(pose.translation), wake);
        body.set_rotation(crate::convert::to_angle(pose.rotation), wake);
        true
    }

    /// Advance by one frame. See [`PhysicsWorld::step`].
    pub fn step(&mut self, frame_dt: f32) -> StepStats {
        let plan = self.clock.plan(frame_dt);
        if plan.steps > 0 {
            self.rapier.integration_parameters.dt = self.clock.fixed_dt();
            for _ in 0..plan.steps {
                self.rapier.step();
            }
        }
        StepStats {
            steps: plan.steps,
            simulated: plan.simulated,
            dropped: plan.dropped,
            clamped: plan.clamped,
        }
    }

    /// Read every 2D body's pose into `out`, which is cleared first.
    pub fn sync(&self, out: &mut Vec<(BodyId, BodyState2)>) -> SyncStats {
        read_all2(&self.rapier, &self.registry, out)
    }

    /// Read one 2D body's pose.
    pub fn pose(&self, id: BodyId) -> Option<BodyState2> {
        let slot = self.registry.get(id)?;
        crate::sync::read2(&self.rapier, slot.body)
    }
}
