//! The single place where `glam` and `nalgebra` meet.
//!
//! # Why this file exists at all
//!
//! rapier is built on `nalgebra`. We are built on `glam`. Those are different
//! vector types with different constructors, different component order, and —
//! the part that actually bites — no conversion between them, so a physics
//! call like `body.set_translation(v, false)` is a type error unless `v` was
//! built as an `nalgebra::Vector3`.
//!
//! rapier does offer an opt-in `glamx` integration, and **enabling it would be
//! a mistake here**: `glamx` 0.3 resolves glam **0.33**, while the engine is on
//! **0.34**. Turning it on does not make glam and rapier agree — it makes the
//! build graph carry a second, older glam next to ours, so a `glam::Vec3`
//! passed from `bifrost` still would not typecheck as a rapier vector while two
//! glam versions now sit in `Cargo.lock`. That is the dependency duplication
//! `docs/TRANSFER.md` refuses for `egui_gizmo` (which pins glam 0.25) and
//! `egui_gizmo`-style traps generally. So: no `glamx`.
//!
//! The price is one conversion at every physics boundary. The way to pay that
//! price once instead of forty times is to have exactly one module that knows
//! both types — this one — and to make it a *rule* rather than a convention.
//! `tests/boundary.rs` greps the crate's own sources and fails if `nalgebra`,
//! `rapier3d::math`, `rapier2d::math` or `rapier*::prelude` appears anywhere
//! else. A convention that nothing checks is a convention that decays.
//!
//! # The rule, stated so it can be enforced
//!
//! - [`femur::sync`]-style values crossing **out** of this crate are `glam`.
//! - Values crossing **in** are constructed **here**, by these functions.
//! - No public item anywhere in `femur` has `nalgebra` in its signature. A
//!   `pub fn` that took one would leak a second math type into every crate
//!   above us and undo the reason for choosing `glam` at all.
//!
//! # One construction site, not four
//!
//! The obvious shape of this file is four near-identical constructors, and the
//! obvious failure mode is the fifth one, added later for a 2D-only call site,
//! which gets the axis order wrong. So there is exactly **one** place that
//! builds an nalgebra vector ([`to_vec3`]) and exactly one that reads one back
//! ([`from_vec3`]); [`to_vec2`] and [`from_vec2`] delegate to them rather than
//! repeating the unpacking. That delegation is not free of cost — it round
//! trips through a z component — it is free of cost, and more importantly it
//! cannot drift.
//!
//! # Exactness
//!
//! Vectors round-trip bit-for-bit: nothing is computed, only copied.
//!
//! Rotations round-trip bit-for-bit **when the input is already unit**.
//! nalgebra's `UnitQuaternion` is a `new_unchecked` wrapper around a raw
//! quaternion, and the obvious-looking constructor, `from_quaternion`,
//! normalizes — which for an already-unit quaternion divides by a `sqrt` that
//! is `1.0 ± 1 ulp`, so "identity comes back as identity" quietly becomes
//! "identity comes back as something one ulp away from identity" *every time*,
//! and a test that only checks `abs_diff < 1e-6` will never see it. So
//! [`to_quat`] normalizes **only when the input is not already unit** and
//! otherwise stores the components verbatim.
//!
//! The same asymmetry applies in reverse for 2D, where a rotation is a scalar
//! angle stored as a `(cos, sin)` pair: going out is exact, coming back is an
//! `atan2`, and `atan2(sin, cos)` is correctly rounded for `cos, sin` but the
//! `sin` and `cos` were themselves rounded on the way in. Angles are therefore
//! compared with an epsilon, and the test says so rather than pretending
//! otherwise.

use glam::{Quat, Vec2, Vec3};
use rapier2d::math::{Rotation as Rotation2, Vector as Vector2N};
use rapier3d::math::{Rotation as Rotation3, Vector as Vector3N};

/// A `glam` vector as the `nalgebra` one rapier takes.
///
/// **The one construction site.** Everything else in this file that produces an
/// `nalgebra` vector routes through here.
#[inline]
pub fn to_vec3(v: Vec3) -> Vector3N {
    Vector3N::new(v.x, v.y, v.z)
}

/// A `glam` vector from the `nalgebra` one rapier hands back.
///
/// **The one read-back site**, for the same reason as [`to_vec3`].
#[inline]
pub fn from_vec3(v: Vector3N) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// The 2D form of [`to_vec3`], delegated rather than repeated.
///
/// The z round trip is two field copies and buys a single place to get the
/// axis order right.
#[inline]
pub fn to_vec2(v: Vec2) -> Vector2N {
    to_vec3(Vec3::new(v.x, v.y, 0.0)).truncate()
}

/// The 2D form of [`from_vec3`], delegated rather than repeated.
#[inline]
pub fn from_vec2(v: Vector2N) -> Vec2 {
    from_vec3(Vector3N::new(v.x, v.y, 0.0)).truncate()
}

/// A `glam` rotation as the unit quaternion rapier stores.
///
/// Normalizes only if it has to. See the module docs: rapier's ergonomic
/// constructor normalizes unconditionally, which costs an ulp on every
/// already-unit input and is invisible to any tolerance-based test.
#[inline]
pub fn to_quat(q: Quat) -> Rotation3 {
    let q = if q.is_normalized() { q } else { q.normalize() };
    // rapier's quaternion is a 0.33 glam Quat, ours is a 0.34 glam Quat: two
    // distinct types, so construct instead of copying.
    Rotation3::from_xyzw(q.x, q.y, q.z, q.w)
}

/// A `glam` rotation from rapier's, exact for unit input.
#[inline]
pub fn from_quat(q: Rotation3) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

/// A 2D rotation in **radians**, as a unit complex.
///
/// `Rotation2::new` takes the angle directly, so nothing here names a complex
/// number and nothing has to be unpacked by hand.
#[inline]
pub fn to_angle(radians: f32) -> Rotation2 {
    Rotation2::new(radians)
}

/// A 2D rotation back to **radians**.
///
/// `atan2(sin, cos)`, which is the only way back and is accurate to well under
/// a milliradian for any angle. See the module docs on why this one is not
/// bit-exact in the way vectors are.
#[inline]
pub fn from_angle(rotation: Rotation2) -> f32 {
    rotation.angle()
}

/// A position plus an orientation, in `glam`.
///
/// rapier's own pose type is a third pair of types again; naming it in a public
/// signature would put `nalgebra` in `femur`'s API, so femur has its own and it
/// is a plain data pair. Cheap to copy, impossible to get wrong.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pose {
    /// World position.
    pub translation: Vec3,
    /// World orientation.
    pub rotation: Quat,
}

impl Pose {
    /// Identity: no translation, no rotation.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };

    /// Builds a pose from a position and an orientation.
    pub fn new(translation: Vec3, rotation: Quat) -> Self {
        Self {
            translation,
            rotation,
        }
    }
}

/// The 2D form of [`Pose`]: a position and a rotation in radians.
///
/// Not `Pose` with a `z` bolted on. A 2D scene file has no third axis, and
/// modelling it as a 3D pose with an unused component is how a 2D body ends up
/// with a rotation about an axis nobody authored.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pose2 {
    /// World position on the XY plane.
    pub translation: Vec2,
    /// World rotation, **radians**, counter-clockwise, about +Z.
    pub rotation: f32,
}

impl Pose2 {
    /// Identity: no translation, no rotation.
    pub const IDENTITY: Self = Self {
        translation: Vec2::ZERO,
        rotation: 0.0,
    };

    /// Builds a 2D pose from a position and a rotation in radians.
    pub fn new(translation: Vec2, rotation: f32) -> Self {
        Self {
            translation,
            rotation,
        }
    }
}
