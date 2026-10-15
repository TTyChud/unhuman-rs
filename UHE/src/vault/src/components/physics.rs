//! Physics bodies and colliders.

use serde::{Deserialize, Serialize};

/// How a rigid body moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum BodyType {
    /// Never moves.
    #[default]
    Static,
    /// Moves only when something moves it.
    Kinematic,
    /// Moved by forces and gravity.
    Dynamic,
}

/// A 3D rigid body.
///
/// `linear_damping` and `angular_damping` are spelled out rather than
/// abbreviated because the C++ calls one `m_LinearDamp` and the other
/// `m_AngularDamp`, and the abbreviation hides that they are not the same kind
/// of quantity.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct RigidBody {
    /// Whether forces move it, or only code does.
    pub body_type: BodyType,
    /// Kilograms. Ignored for a static body.
    pub mass: f32,
    /// 0 = no bounce, 1 = perfectly elastic.
    pub restitution: f32,
    /// Coulomb friction coefficient.
    pub friction: f32,
    /// Velocity lost per second, as a fraction.
    pub linear_damping: f32,
    /// Spin lost per second, as a fraction.
    pub angular_damping: f32,
    /// 0 = weightless, 1 = normal, 2 = double.
    pub gravity_scale: f32,
    /// Whether the body starts asleep.
    ///
    /// Upstream skips syncing sleeping bodies back (`Scene.cpp` guards on
    /// `IsActive`), so a body that falls asleep visually freezes. Parity here
    /// means fixing that, not reproducing it.
    /// Whether the body starts asleep and is skipped until woken.
    pub start_asleep: bool,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            body_type: BodyType::Dynamic,
            mass: 1.0,
            restitution: 0.0,
            friction: 0.5,
            linear_damping: 0.0,
            angular_damping: 0.0,
            gravity_scale: 1.0,
            start_asleep: false,
        }
    }
}

/// A 3D box collider in local space. Half-extents, not full, so that a scale
/// of 1 gives a unit cube rather than a 2×2×2 one.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct BoxCollider3d {
    /// Half the size on each local axis.
    pub half_extents: [f32; 3],
    /// Offset from the entity's origin.
    /// Offset from the entity's origin.
    pub offset: [f32; 3],
}

impl Default for BoxCollider3d {
    fn default() -> Self {
        Self {
            half_extents: [0.5; 3],
            offset: [0.0; 3],
        }
    }
}

/// A sphere collider in local space.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct SphereCollider3d {
    /// Sphere radius.
    pub radius: f32,
    /// Offset from the entity's origin.
    /// Offset from the entity's origin.
    pub offset: [f32; 3],
}

impl Default for SphereCollider3d {
    fn default() -> Self {
        Self {
            radius: 0.5,
            offset: [0.0; 3],
        }
    }
}

/// A capsule collider in local space, aligned to a local axis.
///
/// Axis-aligned rather than freely oriented: a tilted capsule needs a rotation
/// and a half-remembered one is worse than a stated constraint.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct CapsuleCollider3d {
    /// Capsule radius.
    pub radius: f32,
    /// Length of the cylindrical section, excluding the caps.
    /// Half the cylindrical length, excluding the caps.
    pub half_height: f32,
    /// Offset from the entity's origin.
    pub offset: [f32; 3],
}

impl Default for CapsuleCollider3d {
    fn default() -> Self {
        Self {
            radius: 0.5,
            half_height: 0.5,
            offset: [0.0; 3],
        }
    }
}
/// A 2D box collider in local space.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct BoxCollider2d {
    /// Half the size on each local axis.
    pub half_extents: [f32; 2],
    /// Offset from the entity's origin.
    pub offset: [f32; 2],
}

/// A 2D rigid body.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct RigidBody2d {
    /// Whether forces move it, or only code does.
    pub body_type: BodyType,
    /// Kilograms. Ignored for a static body.
    pub mass: f32,
    /// 0 = no bounce, 1 = perfectly elastic.
    pub restitution: f32,
    /// Coulomb friction coefficient.
    pub friction: f32,
    /// Velocity lost per second, as a fraction.
    pub linear_damping: f32,
    /// 0 = weightless, 1 = normal, 2 = double.
    pub gravity_scale: f32,
}
