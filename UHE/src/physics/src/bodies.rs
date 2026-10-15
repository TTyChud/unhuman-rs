//! Rigid bodies: what one is, and how the crate keeps track of them.
//!
//! [`BodyId`] is the key a caller names a body by — in practice an entity id
//! from `meats`, so femur never depends on the ECS and the ECS never has to
//! know what a solver handle is. It is a newtype over `u64` because
//! `despawn(id)` and `despawn(collider_count)` are both `fn(u64)` at the type
//! level otherwise, and they mean different things.
//!
//! # Mass
//!
//! The C++ asks Jolt for `CalculateInertia` with an explicit mass override: give
//! me this shape, this total mass, and work out the inertia tensor from the
//! shape. rapier's equivalent is per-collider mass, not per-body — it has no
//! body-level mass override, and `additional_mass` on the body is an *addition*
//! to what the colliders already give, which is not the same number.
//!
//! So the total is divided across the body's colliders by volume, and each is
//! given that share. [`mass_shares`] is the whole algorithm, as a pure function,
//! because it is the one piece of this file that can be wrong in a way no
//! integration test will notice: a wrong share still produces a body that
//! falls at the right rate, just with the wrong rotational inertia.

use crate::convert::{self, Pose, Pose2};
use crate::solver::Dofs;

/// The key a caller uses to name a body. See the module docs for why this is a
/// newtype and why it is not the ECS's entity type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BodyId(u64);

impl BodyId {
    /// Wraps a raw key.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw key, for handing back to whatever produced it.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl From<u64> for BodyId {
    fn from(raw: u64) -> Self {
        Self(raw)
    }
}

impl From<BodyId> for u64 {
    fn from(id: BodyId) -> Self {
        id.0
    }
}

/// Whether a body is moved by the solver, moved by code, or not moved at all.
///
/// Naming follows `vault::components::BodyType`, which is the C++'s
/// `RigidBody3DComponent::BodyType` with the same three cases. The mapping to
/// rapier is in [`crate::solver::body_type3`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BodyKind {
    /// Never moves. Level geometry, walls, the floor.
    #[default]
    Static,
    /// Moved only by [`crate::solver::drive_kinematic3`]. Pushes dynamic bodies,
    /// is not pushed back.
    Kinematic,
    /// Moved by gravity, contacts and forces.
    Dynamic,
}

/// A rigid body, as `femur` describes one.
///
/// The shape of this is `vault::components::RigidBody` — the fields line up one
/// for one, so loading a scene is a field copy and not a translation layer.
/// The two additions are [`BodyDesc::dofs`] and [`BodyDesc::sensor`], both of
/// which the C++ has on the component (`AllowedDOFs`, `IsSensor`) and neither of
/// which `vault`'s component currently carries.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodyDesc {
    /// Whether forces move it, or only code does.
    pub kind: BodyKind,
    /// Kilograms. Only meaningful for [`BodyKind::Dynamic`]; a static body has
    /// infinite mass by definition and this is ignored.
    pub mass: f32,
    /// Coulomb friction, applied to every collider this body owns unless that
    /// collider overrides it. 0 is ice, 1 is rubber.
    pub friction: f32,
    /// Bounce, `0..=1`. 0 is a dead stop, 1 is a perfect spring.
    pub restitution: f32,
    /// Fraction of linear velocity shed per second.
    pub linear_damping: f32,
    /// Fraction of angular velocity shed per second.
    pub angular_damping: f32,
    /// `0.0` floats, `1.0` normal, `2.0` double.
    pub gravity_scale: f32,
    /// A sensor reports overlaps and is not pushed. See the note in
    /// [`colliders`](crate::colliders) about where this is applied.
    pub sensor: bool,
    /// Start asleep, so a distant pile of rubble costs nothing until something
    /// touches it.
    pub start_asleep: bool,
    /// Which axes this body may move along. See [`crate::solver::Dofs`].
    pub dofs: Dofs,
}

impl Default for BodyDesc {
    /// A 1 kg dynamic box-ish body with ordinary friction.
    ///
    /// Note this is *not* `vault::components::RigidBody::default()`, whose
    /// `body_type` is also `Dynamic` but whose friction default is 0.5 rather
    /// than the C++'s 0.2. The two defaults differ because the two files were
    /// written by different hands; whichever a scene author means, they can now
    /// see both.
    fn default() -> Self {
        Self {
            kind: BodyKind::Dynamic,
            mass: 1.0,
            friction: 0.2,
            restitution: 0.0,
            linear_damping: 0.05,
            angular_damping: 0.05,
            gravity_scale: 1.0,
            sensor: false,
            start_asleep: false,
            dofs: Dofs::FREE,
        }
    }
}

/// A 2D rigid body.
///
/// Friction and restitution live on the *body* here, as they do in
/// [`BodyDesc`], but note that the C++ put them on the 2D *collider*
/// (`BoxColliderComponent::Friction`) and on the 3D body. One rule for both
/// dimensions is worth more than matching one of them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodyDesc2 {
    /// Whether forces move it, or only code does.
    pub kind: BodyKind,
    /// Kilograms. Ignored unless [`BodyKind::Dynamic`].
    pub mass: f32,
    /// Coulomb friction.
    pub friction: f32,
    /// Bounce, `0..=1`.
    pub restitution: f32,
    /// Fraction of linear velocity shed per second.
    pub linear_damping: f32,
    /// `0.0` floats, `1.0` normal, `2.0` double.
    pub gravity_scale: f32,
    /// Whether the body starts asleep.
    pub start_asleep: bool,
    /// Which axes this body may move along.
    pub dofs: crate::solver::Dofs2,
}

impl Default for BodyDesc2 {
    fn default() -> Self {
        Self {
            kind: BodyKind::Dynamic,
            mass: 1.0,
            friction: 0.2,
            restitution: 0.0,
            linear_damping: 0.0,
            gravity_scale: 1.0,
            start_asleep: false,
            dofs: crate::solver::Dofs2::FREE,
        }
    }
}

/// Why a spawn was refused.
///
/// Two cases and no more, because a spawn that fails for a reason the caller
/// cannot act on is not an error the caller can handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpawnError {
    /// A body is already registered under this key. Despawn it first.
    AlreadyExists(BodyId),
}

/// Split a total mass across colliders in proportion to their volumes.
///
/// Proportional, not equal, and the difference is not cosmetic: a compound
/// body of one big box and one small box given equal mass has the wrong inertia
/// tensor about both axes, and the error shows up as a body that tips at the
/// wrong rate. The C++ never had to make this choice because it only ever
/// created one shape per body — which is also why it never exercised it.
///
/// A zero or negative total volume (a degenerate collider, or none at all)
/// falls back to an even split. Even is wrong, but the alternative is a
/// division by zero, and a body with no volume has no mass distribution to be
/// right about anyway.
pub fn mass_shares(volumes: &[f32], total_mass: f32) -> Vec<f32> {
    if volumes.is_empty() {
        return Vec::new();
    }
    let total_volume: f32 = volumes.iter().sum();
    if total_volume <= 0.0 || !total_volume.is_finite() {
        return vec![total_mass / volumes.len() as f32; volumes.len()];
    }
    let mut shares: Vec<f32> = volumes
        .iter()
        .map(|v| total_mass * (v / total_volume))
        .collect();
    // Push the rounding error into the largest share, so the shares sum to
    // exactly `total_mass` and a body never gains or loses a milligram to
    // float error.
    let assigned: f32 = shares.iter().sum();
    if let Some(largest) = shares
        .iter_mut()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
    {
        shares[largest] += total_mass - assigned;
    }
    shares
}

/// Build a 3D rigid body from a [`BodyDesc`], tagged with `id`.
///
/// The translation layer from femur's own vocabulary to rapier's, and nothing
/// else: every field on the descriptor appears here exactly once, which is what
/// makes "did the port drop a field?" a question this function answers by
/// being readable rather than by being tested.
///
/// Three choices worth stating:
///
/// - **`user_data` carries the body key.** rapier stores it with the body, so a
///   contact or query result can be attributed without a map lookup. It is a
///   copy of [`BodyId`], never a substitute for one — the registry stays the
///   only authority on which keys exist.
/// - **Per-axis locks are applied here, not after insertion.** rapier's builder
///   sets the flags before the body is registered, so a body that is never
///   unlocked never has an unlocked frame — whereas applying them after
///   insertion means the body exists for one instant in the solver with all six
///   axes free, which is exactly the instant an overlapping spawn can push it.
/// - **Damping, restitution and friction are not body properties in rapier.**
///   Only the two damping values and the gravity scale are; the material
///   coefficients live on colliders and are written by
///   [`crate::colliders::build3`].
pub fn build_body3(desc: &BodyDesc, pose: Pose, id: BodyId) -> rapier3d::dynamics::RigidBody {
    use rapier3d::dynamics::RigidBodyBuilder as Builder;

    let builder: Builder = match desc.kind {
        BodyKind::Static => Builder::fixed(),
        BodyKind::Kinematic => Builder::kinematic_position_based(),
        BodyKind::Dynamic => Builder::dynamic(),
    };
    builder
        .pose(rapier3d::math::Pose::from_parts(
            convert::to_vec3(pose.translation),
            convert::to_quat(pose.rotation),
        ))
        .linear_damping(desc.linear_damping)
        .angular_damping(desc.angular_damping)
        .gravity_scale(desc.gravity_scale)
        .sleeping(desc.start_asleep)
        .user_data(u128::from(id.raw()))
        .enabled_translations(
            desc.dofs.translate[0],
            desc.dofs.translate[1],
            desc.dofs.translate[2],
        )
        .enabled_rotations(
            desc.dofs.rotate[0],
            desc.dofs.rotate[1],
            desc.dofs.rotate[2],
        )
        .build()
}

/// Build a 2D rigid body from a [`BodyDesc2`]. See [`build_body3`].
///
/// The rotation is an `f32` in radians here because 2D rotation is one scalar,
/// which is also the fix for the C++'s degree/radian confusion: upstream writes
/// `b2MakeRot(transform.Rotation.z)` where `TransformComponent::Rotation` is
/// documented "assumed in degrees", so every 2D body's angle in the C++ is
/// wrong by a factor of 57.3 unless it happens to be zero. There is no degrees
/// anywhere in femur's 2D API, so the question cannot be asked.
pub fn build_body2(desc: &BodyDesc2, pose: Pose2, id: BodyId) -> rapier2d::dynamics::RigidBody {
    use rapier2d::dynamics::RigidBodyBuilder as Builder;

    let builder = match desc.kind {
        BodyKind::Static => Builder::fixed(),
        BodyKind::Kinematic => Builder::kinematic_position_based(),
        BodyKind::Dynamic => Builder::dynamic(),
    };
    let builder = builder
        .translation(convert::to_vec2(pose.translation))
        .rotation(pose.rotation)
        .linear_damping(desc.linear_damping)
        .gravity_scale(desc.gravity_scale)
        .sleeping(desc.start_asleep)
        .user_data(u128::from(id.raw()))
        .enabled_translations(desc.dofs.translate[0], desc.dofs.translate[1]);
    // The 2D builder's only rotation setter is `lock_rotations`, with no
    // argument, so honouring a single rotation DOF is a conditional rather
    // than a call. Box2D spells the same thing `motionLocks.angularZ`.
    let builder = if desc.dofs.rotate {
        builder
    } else {
        builder.lock_rotations()
    };
    builder.build()
}
