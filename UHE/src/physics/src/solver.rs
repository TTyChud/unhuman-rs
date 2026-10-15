//! Constraints: what a body is allowed to do, and how that is applied.
//!
//! # `AllowedDOFs`, applied
//!
//! `RigidBody3DComponent::AllowedDOFs[6]` is serialized, deserialized, and
//! copied in `Scene::Copy` — and never reaches the solver. It is the clearest
//! example in the C++ of a field that looks like a feature because it has a
//! name, a serialiser entry and a copy loop, and which a test cannot catch
//! because no test asserts on motion.
//!
//! It is applied here, per axis, through rapier's own per-axis locks. Two
//! details that are easy to get wrong and are therefore explicit:
//!
//! - **Locking an axis scrubs its velocity component.** rapier locks an axis by
//!   making its effective inverse mass zero, which stops the *solver* from
//!   accelerating it — but any velocity already in the body stays there. For a
//!   body that code teleports every frame, that residual is a force along an
//!   axis the author explicitly locked. Both [`apply_dofs3`] and [`apply_dofs2`]
//!   therefore zero the locked components at the moment they lock.
//! - **A kinematic body honours the locks too.** The C++ built an
//!   `EMotionType::Kinematic` body and then had nothing to point it at, so
//!   every kinematic entity in the C++ sat still. [`drive_kinematic3`] supplies
//!   the missing target, and the velocity rapier derives from it is what the
//!   locks constrain.
//!
//! The one thing deliberately *not* done here is continuous collision detection
//! tuning. rapier's `max_ccd_substeps` defaults to 1 — the same value the C++
//! hardcodes — and there is no setting on a body that expresses "this is a
//! projectile", so leaving it at the default is parity rather than an
//! omission. See [`crate::timestep`] for what the fixed step does about the
//! rest of it.

use glam::{Vec2, Vec3};

use rapier2d::dynamics::RigidBodyHandle as Handle2;
use rapier2d::pipeline::PhysicsWorld as World2;
use rapier3d::dynamics::RigidBodyHandle as Handle3;
use rapier3d::pipeline::PhysicsWorld as World3;

use crate::convert::{self, Pose, Pose2};
/// Apply per-axis locks to a 3D body, and scrub the velocity it already had
/// along every newly locked axis.
///
/// Returns `false` if the handle is stale, which is the normal answer after a
/// despawn and is not an error.
pub fn apply_dofs3(world: &mut World3, id: Handle3, dofs: Dofs, wake: bool) -> bool {
    let Some(body) = world.bodies.get_mut(id) else {
        return false;
    };
    body.set_enabled_translations(
        dofs.translate[0],
        dofs.translate[1],
        dofs.translate[2],
        wake,
    );
    body.set_enabled_rotations(dofs.rotate[0], dofs.rotate[1], dofs.rotate[2], wake);
    scrub3(body, dofs);
    true
}

/// Apply per-axis locks to a 2D body. See [`apply_dofs3`].
pub fn apply_dofs2(world: &mut World2, id: Handle2, dofs: Dofs2, wake: bool) -> bool {
    let Some(body) = world.bodies.get_mut(id) else {
        return false;
    };
    body.set_enabled_translations(dofs.translate[0], dofs.translate[1], wake);
    // In 2D a rotation is one degree of freedom, so the per-axis API is the
    // wrong shape; `lock_rotations` sets the flag the 2D solver reads
    // (`ROTATION_LOCKED_Z`, which zeroes the scalar inverse inertia).
    body.lock_rotations(!dofs.rotate, wake);

    let linvel = convert::from_vec2(body.linvel());
    let scrubbed = Vec2::new(
        if dofs.translate[0] { linvel.x } else { 0.0 },
        if dofs.translate[1] { linvel.y } else { 0.0 },
    );
    if scrubbed != linvel {
        body.set_linvel(convert::to_vec2(scrubbed), false);
    }
    if !dofs.rotate && body.angvel() != 0.0 {
        body.set_angvel(0.0, false);
    }
    true
}

fn scrub3(body: &mut rapier3d::dynamics::RigidBody, dofs: Dofs) {
    let linvel = convert::from_vec3(body.linvel());
    let scrubbed = Vec3::new(
        if dofs.translate[0] { linvel.x } else { 0.0 },
        if dofs.translate[1] { linvel.y } else { 0.0 },
        if dofs.translate[2] { linvel.z } else { 0.0 },
    );
    if scrubbed != linvel {
        body.set_linvel(convert::to_vec3(scrubbed), false);
    }

    // The angular velocity is a world-space axis * angle vector in 3D. Zeroing
    // an axis means removing that axis' contribution — the angular velocity
    // does not decompose into independent components, so what gets scrubbed is
    // the component of the spin vector along the locked direction. A body
    // locked about X and Z therefore keeps only whatever Y-spin it has, which
    // is the honest reading of "rotation is free about Y only".
    if !dofs.rotate.iter().all(|free| *free) {
        let angvel = convert::from_vec3(body.angvel());
        let scrubbed = Vec3::new(
            if dofs.rotate[0] { angvel.x } else { 0.0 },
            if dofs.rotate[1] { angvel.y } else { 0.0 },
            if dofs.rotate[2] { angvel.z } else { 0.0 },
        );
        if scrubbed != angvel {
            body.set_angvel(convert::to_vec3(scrubbed), false);
        }
    }
}

/// Point a 3D kinematic body at `target`; rapier derives the velocity that
/// gets it there from the world's own fixed step.
///
/// A kinematic body is inert on its own: the C++ created them with
/// `EMotionType::Kinematic` and then had nothing to tell them where to go, so
/// every "kinematic" entity in the C++ sat still. This is the missing half.
pub fn drive_kinematic3(world: &mut World3, id: Handle3, target: Pose) -> bool {
    let Some(body) = world.bodies.get_mut(id) else {
        return false;
    };
    if !body.is_kinematic() {
        return false;
    }
    body.set_next_kinematic_translation(convert::to_vec3(target.translation));
    body.set_next_kinematic_rotation(convert::to_quat(target.rotation));
    true
}

/// The 2D form of [`drive_kinematic3`]. Rotation is an angle in radians.
pub fn drive_kinematic2(world: &mut World2, id: Handle2, target: Pose2) -> bool {
    let Some(body) = world.bodies.get_mut(id) else {
        return false;
    };
    if !body.is_kinematic() {
        return false;
    }
    body.set_next_kinematic_translation(convert::to_vec2(target.translation));
    body.set_next_kinematic_rotation(convert::to_angle(target.rotation));
    true
}

/// Scale gravity for one body.
///
/// `0.0` floats, `1.0` normal, `2.0` double. Per body rather than per world
/// because the C++ exposes `GravityScale` on the component and the world has a
/// single global gravity; the component-level value is the one an author
/// actually sets.
pub fn set_gravity_scale3(world: &mut World3, id: Handle3, scale: f32, wake: bool) -> bool {
    let Some(body) = world.bodies.get_mut(id) else {
        return false;
    };
    body.set_gravity_scale(scale, wake);
    true
}

/// The 2D form of [`set_gravity_scale3`].
pub fn set_gravity_scale2(world: &mut World2, id: Handle2, scale: f32, wake: bool) -> bool {
    let Some(body) = world.bodies.get_mut(id) else {
        return false;
    };
    body.set_gravity_scale(scale, wake);
    true
}
/// Allowed degrees of freedom for a 3D body. `true` means free.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Dofs {
    /// Translation along X, Y, Z.
    pub translate: [bool; 3],
    /// Rotation about X, Y, Z.
    pub rotate: [bool; 3],
}

impl Dofs {
    /// All six axes free.
    pub const FREE: Self = Self {
        translate: [true, true, true],
        rotate: [true, true, true],
    };
}

/// Allowed degrees of freedom for a 2D body. `true` means free.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dofs2 {
    /// Translation along X, Y.
    pub translate: [bool; 2],
    /// Rotation about Z.
    pub rotate: bool,
}

impl Dofs2 {
    /// All three axes free.
    pub const FREE: Self = Self {
        translate: [true, true],
        rotate: true,
    };
}
