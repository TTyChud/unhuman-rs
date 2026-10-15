//! The façade: a world you can spawn into, step, and read back.
//!
//! # What this owns, and what it refuses to own
//!
//! It owns a rapier world, the [`BodyRegistry`] that maps keys to handles, and
//! the [`FixedStep`] clock. It does not own scene hierarchy, transforms, or
//! entity lifetimes — a caller tells it "this key is now a dynamic box here" and
//! later "read me every pose", and everything in between is physics.
//!
//! That boundary is the reason there is no `PhysicsWorld::set_transform(entity)`
//! taking a hierarchy-aware pose. A physics pose is **world space**, so a
//! body-parented-to-a-moving-platform question is the caller's to answer, and
//! answering it here would need femur to know what a scene graph is.
//!
//! # Gravity
//!
//! Upstream sets 2D gravity to `{0, -9.8}` in `b2WorldDef` and leaves 3D at
//! Jolt's default of `{0, -9.81, 0}`. The 0.01 difference is not a decision, it
//! is one number being written down in one place and defaulted in the other, so
//! both dimensions get the same default here and a caller can set either. World
//! gravity is a [`PhysicsWorld::new`] argument rather than a
//! `set_gravity` method because changing it after bodies exist is a tuning
//! mistake more often than it is a feature.
//!
//! # Collision filtering
//!
//! Not ported, and deliberately: the C++ puts bodies on two Jolt object layers
//! (`Layers::NON_MOVING`, `Layers::MOVING`) and defines no filter matrix
//! between them, so every layer collides with every other. rapier's default is
//! the same "everything collides with everything". Exposing
//! [`rapier3d::geometry::InteractionGroups`] before there is a layer scheme to
//! map onto would be inventing an API upstream does not have.

use glam::Vec3;

use rapier3d::pipeline::PhysicsWorld as Rapier3;

use crate::bodies::{build_body3, mass_shares, BodyDesc, BodyId, BodyKind, SpawnError};
use crate::colliders::{build3, ColliderDesc};
use crate::convert::Pose;
use crate::registry::BodyRegistry;
use crate::solver::{self, Dofs};
use crate::sync::{read_all3, BodyState, SyncStats};
use crate::timestep::{FixedStep, StepStats};

/// Why a collider could not be moved to a different body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReparentError {
    /// The source key is not registered.
    UnknownSource(BodyId),
    /// The destination key is not registered.
    UnknownTarget(BodyId),
    /// The source entity has no collider to move.
    NoCollider(BodyId),
    /// The destination already owns a collider. One entity owns at most one
    /// collider in this API, so accepting the move would orphan the old one
    /// while leaving it in the solver — a leak with no error.
    TargetOccupied(BodyId),
}

/// A 3D physics world: rapier, a key registry, and a fixed-step clock.
pub struct PhysicsWorld {
    rapier: Rapier3,
    registry: BodyRegistry,
    clock: FixedStep,
}

impl PhysicsWorld {
    /// A world with `gravity` and the default 60 Hz clock.
    pub fn new(gravity: Vec3) -> Self {
        Self::with_clock(gravity, FixedStep::default_rate())
    }

    /// A world with `gravity` and a chosen clock.
    pub fn with_clock(gravity: Vec3, clock: FixedStep) -> Self {
        let mut rapier = Rapier3::new();
        rapier.gravity = crate::convert::to_vec3(gravity);
        rapier.integration_parameters.dt = clock.fixed_dt();
        Self {
            rapier,
            registry: BodyRegistry::new(),
            clock,
        }
    }

    /// The clock, for [`PhysicsWorld::step_stats`]-style reporting.
    pub fn clock(&self) -> &FixedStep {
        &self.clock
    }

    /// Read-only access to the solver, for ray casts and the rest.
    pub fn rapier(&self) -> &Rapier3 {
        &self.rapier
    }

    /// How many bodies this world owns.
    pub fn body_count(&self) -> usize {
        self.registry.len()
    }

    /// How many colliders the solver holds. Equal to the number of entities
    /// spawned with at least one shape — asserted by the leak tests, because a
    /// drifting count here is the signature of every registry bug.
    pub fn collider_count(&self) -> usize {
        self.rapier.colliders.len()
    }

    /// Whether a key is registered.
    pub fn contains(&self, id: BodyId) -> bool {
        self.registry.contains(id)
    }

    /// Spawn a body, optionally with colliders.
    ///
    /// `colliders` is a slice rather than an `Option` because a compound body
    /// is one call: `&[]` for a body with no shape, `&[one]` for exactly what
    /// the C++ builds, `&[a, b, c]` for something it cannot build at all. The
    /// declared `mass` is divided across them by volume — see
    /// [`crate::bodies::mass_shares`].
    ///
    /// A body with no collider is created. The C++ skips such an entity
    /// entirely (`if (shape) { ... }` guards the whole 3D creation), which
    /// means a scene that forgot a collider gets a body-less entity whose
    /// transform is never read back and therefore never moves — a bug with no
    /// message. Creating the body makes the omission visible as a body falling
    /// forever, which is a much easier bug to see.
    pub fn spawn(
        &mut self,
        id: BodyId,
        desc: &BodyDesc,
        colliders: &[ColliderDesc],
        pose: Pose,
        scale: Vec3,
    ) -> Result<(), SpawnError> {
        if self.registry.contains(id) {
            return Err(SpawnError::AlreadyExists(id));
        }
        let body = self.rapier.insert_body(build_body3(desc, pose, id));

        let mut handles = Vec::with_capacity(colliders.len());
        for collider in colliders {
            let builder =
                build3(collider, scale, desc.friction, desc.restitution).sensor(desc.sensor);
            handles.push(self.rapier.insert_collider(builder.build(), Some(body)));
        }

        if desc.kind == BodyKind::Dynamic && !handles.is_empty() {
            self.apply_declared_mass(&handles, desc.mass);
        }

        self.registry.insert(id, body, handles.into_iter().next())
    }

    /// Remove a body and everything attached to it.
    ///
    /// Idempotent: a second call for the same key is `false`, not a panic and
    /// not a second free. Callers despawn on entity destruction, and entity
    /// destruction can happen twice — on a scene teardown that races an
    /// explicit removal, or on two systems disagreeing that they own an entity.
    pub fn despawn(&mut self, id: BodyId) -> bool {
        let Some(slot) = self.registry.remove(id) else {
            return false;
        };
        // rapier removes a body's attached colliders with it, so removing the
        // collider handle separately here would be a double free waiting for a
        // slot reuse.
        self.rapier.remove_body(slot.body).is_some()
    }

    /// Give one entity's collider to another entity's body.
    ///
    /// The shape is unchanged; what changes is which body it pushes. Note the
    /// collider offset is now interpreted in the *new* parent's frame, which is
    /// the honest semantic — a collider offset is a local quantity and the
    /// entity that owns it has just changed.
    pub fn reparent_collider(&mut self, from: BodyId, to: BodyId) -> Result<(), ReparentError> {
        let source = self
            .registry
            .get(from)
            .ok_or(ReparentError::UnknownSource(from))?;
        let Some(collider) = source.collider else {
            return Err(ReparentError::NoCollider(from));
        };
        let target = self
            .registry
            .get(to)
            .ok_or(ReparentError::UnknownTarget(to))?;
        if target.collider.is_some() {
            return Err(ReparentError::TargetOccupied(to));
        }
        self.rapier
            .colliders
            .set_parent(collider, Some(target.body), &mut self.rapier.bodies);
        if !self.registry.reparent_collider(from, to) {
            return Err(ReparentError::UnknownTarget(to));
        }
        Ok(())
    }

    /// Apply per-axis locks to one body. See [`Dofs`] and [`solver::apply_dofs3`].
    pub fn set_dofs(&mut self, id: BodyId, dofs: Dofs, wake: bool) -> bool {
        match self.registry.get(id) {
            Some(slot) => solver::apply_dofs3(&mut self.rapier, slot.body, dofs, wake),
            None => false,
        }
    }

    /// Scale gravity for one body. See [`solver::set_gravity_scale3`].
    pub fn set_gravity_scale(&mut self, id: BodyId, scale: f32, wake: bool) -> bool {
        match self.registry.get(id) {
            Some(slot) => solver::set_gravity_scale3(&mut self.rapier, slot.body, scale, wake),
            None => false,
        }
    }

    /// Point a kinematic body at a world-space target.
    pub fn drive_kinematic(&mut self, id: BodyId, target: Pose) -> bool {
        match self.registry.get(id) {
            Some(slot) => solver::drive_kinematic3(&mut self.rapier, slot.body, target),
            None => false,
        }
    }

    /// Teleport a body, waking it. For a static body this is the "move the
    /// wall" call.
    pub fn set_pose(&mut self, id: BodyId, pose: Pose, wake: bool) -> bool {
        let Some(slot) = self.registry.get(id) else {
            return false;
        };
        let Some(body) = self.rapier.bodies.get_mut(slot.body) else {
            return false;
        };
        body.set_translation(crate::convert::to_vec3(pose.translation), wake);
        body.set_rotation(crate::convert::to_quat(pose.rotation), wake);
        true
    }

    /// Advance by one frame's worth of wall-clock time.
    ///
    /// Runs [`FixedStep::plan`], then that many solver steps of the fixed
    /// timestep. See [`crate::timestep`] for why this is an accumulator and a
    /// clamp rather than the C++'s variable step.
    pub fn step(&mut self, frame_dt: f32) -> StepStats {
        let plan = self.clock.plan(frame_dt);
        if plan.steps > 0 {
            // Re-asserted every frame: a caller can rebuild the clock's
            // settings through `with_clock`, and a world whose `dt` disagrees
            // with its own clock simulates at the wrong rate with no symptom
            // other than being subtly wrong.
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

    /// Read every body's pose into `out`, which is cleared first.
    ///
    /// Unconditional, sleeping bodies included. See [`crate::sync`] for the
    /// full argument; the short version is that a sleeping pose is a valid
    /// pose and skipping it is upstream's bug, not a feature.
    pub fn sync(&self, out: &mut Vec<(BodyId, BodyState)>) -> SyncStats {
        read_all3(&self.rapier, &self.registry, out)
    }

    /// Read one body's pose without a pass over the registry.
    pub fn pose(&self, id: BodyId) -> Option<BodyState> {
        let slot = self.registry.get(id)?;
        crate::sync::read3(&self.rapier, slot.body)
    }

    /// Divide `mass` across the body's colliders by volume.
    ///
    /// Reads the volume back from rapier rather than computing it here, because
    /// parry's `volume()` is the authority and a second implementation of
    /// "what is a capsule's volume" is a second thing to be wrong about.
    fn apply_declared_mass(&mut self, handles: &[rapier3d::geometry::ColliderHandle], mass: f32) {
        let volumes: Vec<f32> = handles
            .iter()
            .map(|h| self.rapier.colliders.get(*h).map_or(0.0, |c| c.volume()))
            .collect();
        for (handle, share) in handles.iter().zip(mass_shares(&volumes, mass)) {
            if let Some(collider) = self.rapier.colliders.get_mut(*handle) {
                collider.set_mass(share);
            }
        }
    }
}
