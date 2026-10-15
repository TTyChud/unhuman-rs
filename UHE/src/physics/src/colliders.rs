//! Collision shapes: what geometry a body has, and how it is built.
//!
//! # The shape set, and why it is exactly this set
//!
//! Box, sphere and capsule in 3D; box in 2D. That is the C++'s set, and it is
//! the set Jolt has cheapest broad-phase proxies for: a sphere is one AABB, a
//! box is one AABB, a capsule is a segment swept into an AABB. Each has a
//! closed-form volume, which matters because [`crate::bodies::mass_shares`]
//! divides a body's declared mass across its colliders by volume.
//!
//! 2D is box-only and that is **not** a simplification — it is what the C++
//! does. `OnRuntimeStart` iterates
//! `view<TransformComponent, RigidBody2DComponent, BoxColliderComponent>`: a 2D
//! entity with no box collider gets no 2D body at all, where the 3D loop has
//! three `else if` branches and picks whichever collider component it finds.
//! The asymmetry is upstream's, and TRANSFER.md names it ("box-only, no shape
//! priority chain unlike 3D").
//!
//! # `RuntimeShape`, the field upstream declares and never writes
//!
//! `BoxColliderComponent::RuntimeShape` is a `b2ShapeId` initialised to
//! `b2_nullShapeId` and never assigned. It is dead in the C++.
//!
//! It has **no counterpart here at all**, which is the fix rather than a
//! reproduction. The runtime handle for a shape lives in
//! [`crate::registry::BodyRegistry`], keyed by entity, so:
//!
//! - the scene document contains nothing solver-specific, so a saved scene
//!   cannot carry a stale `b2ShapeId` from a previous run into a new one;
//! - there is no second field that can disagree with the registry, which is the
//!   only way the C++ version could ever have been noticed;
//! - "is this collider still valid" is `registry.contains(id)`, one lookup,
//!   rather than a sentinel comparison against a field nobody maintains.
//!
//! A field that exists, is named like a feature, is copied in `Scene::Copy`, and
//! is never written is not dead weight — it is a lie the type system tells the
//! next reader. Leaving it out is the honest version.
//!
//! # Scale, and where it is applied
//!
//! The C++ multiplies shape dimensions by the entity's `Transform::Scale`
//! before building the shape: `bc.HalfExtent * transform.Scale` for a box,
//! `sc.Radius * max(Scale)` for a sphere, `HalfHeight * Scale.y` with
//! `Radius * max(Scale.x, Scale.z)` for a capsule. That is reproduced here, in
//! [`build3`]'s `scale` argument, because getting it from the scene is the
//! ECS's job and femur is handed the result.
//!
//! Note what it does *not* do: non-uniform scale on a sphere takes the maximum
//! axis, so a sphere on a body scaled `(1, 4, 1)` becomes a sphere of radius
//! `r`, not an ellipsoid. Correct enough for level geometry, and a faithful
//! copy of an upstream decision rather than an oversight of ours.
//!
//! # Where friction and restitution live
//!
//! Upstream puts them on the **collider** in 3D (`BoxCollider3DComponent::Friction`)
//! and on the **collider** in 2D as well, while the body carries only mass,
//! damping and `GravityScale`. `vault`'s components put friction and
//! restitution on the **body**, on both sides.
//!
//! Rather than pick one and lose information, [`ColliderDesc`] carries
//! `Option<f32>` overrides: `None` means "inherit the body's", `Some` means
//! "this one shape is different". A single-collider body reads exactly as it
//! would have in the C++, and a compound body can still have a rubbery bumper
//! on one end.

use glam::Vec2;

use rapier2d::geometry::{ColliderBuilder as ColliderBuilder2, SharedShape as Shape2};
use rapier3d::geometry::{ColliderBuilder as ColliderBuilder3, SharedShape as Shape3};

/// The 3D shape set. See the module docs for why it is these three.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    /// An axis-aligned box in local space, given as **half**-extents.
    ///
    /// Half-extents, not full, so a scale of 1 is a unit cube rather than a
    /// 2x2x2 one. This matches `vault::components::BoxCollider3d` and
    /// `glm`-free arithmetic; it does not match Box2D v3's `b2MakeOffsetBox`,
    /// which also takes half-extents but multiplies by scale before halving.
    /// Same answer, one fewer factor of two to get wrong.
    Box {
        /// Half the size on each local axis.
        half_extents: glam::Vec3,
    },
    /// A sphere centred on the collider's local origin.
    Sphere {
        /// The radius.
        radius: f32,
    },
    /// A capsule along the local **Y** axis.
    ///
    /// Y, because that is Jolt's `CapsuleShape` convention and therefore the
    /// C++'s. It is a real limitation, stated rather than hidden: a Z-aligned
    /// character capsule cannot be expressed, and adding an axis enum would
    /// mean a shape that can be constructed three ways and tested one. When
    /// that matters, the honest change is an `Axis` field plus a test per axis,
    /// not a silent default.
    Capsule {
        /// Half the length of the cylindrical section, excluding the caps.
        half_height: f32,
        /// The radius of the caps.
        radius: f32,
    },
}

/// One collider on a 3D body.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColliderDesc {
    /// The shape, in the body's local space before scaling.
    pub shape: Shape,
    /// Offset from the body's origin. This is what lets a body's origin sit at
    /// its feet, which is how characters are built.
    pub offset: glam::Vec3,
    /// Override the body's friction for this shape only.
    pub friction: Option<f32>,
    /// Override the body's restitution for this shape only.
    pub restitution: Option<f32>,
}

impl ColliderDesc {
    /// A collider with no material overrides, so it inherits its body's.
    pub fn new(shape: Shape, offset: glam::Vec3) -> Self {
        Self {
            shape,
            offset,
            friction: None,
            restitution: None,
        }
    }

    /// Overrides this collider's friction against its body's.
    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = Some(friction);
        self
    }

    /// Overrides this collider's restitution against its body's.
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = Some(restitution);
        self
    }
}

/// One collider on a 2D body.
///
/// A distinct type from [`ColliderDesc`] rather than the same type with a
/// shape enum, because the 2D shape set has one member and a `Shape` with one
/// variant is a box wearing a disguise.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColliderDesc2 {
    /// Half the size on each local axis.
    pub half_extents: Vec2,
    /// Offset from the body's origin.
    pub offset: Vec2,
    /// Override the body's friction for this shape only.
    pub friction: Option<f32>,
    /// Override the body's restitution for this shape only.
    pub restitution: Option<f32>,
}

impl ColliderDesc2 {
    /// A unit box at the origin, inheriting its body's material.
    pub fn new(half_extents: Vec2, offset: Vec2) -> Self {
        Self {
            half_extents,
            offset,
            friction: None,
            restitution: None,
        }
    }

    /// Overrides this collider's friction against its body's.
    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = Some(friction);
        self
    }

    /// Overrides this collider's restitution against its body's.
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = Some(restitution);
        self
    }
}

/// The largest component of a scale, which is how a sphere's radius is scaled.
fn max_scale(scale: glam::Vec3) -> f32 {
    scale.x.max(scale.y).max(scale.z)
}

/// Build a 3D collider, applying the body's scale.
///
/// The material arguments are the body's values; a per-collider override is
/// applied by the caller that owns the descriptor, so this function stays a
/// pure shape-and-scale translation with no policy in it.
///
/// Dimensions are clamped to a small positive minimum. A zero half-extent makes
/// a degenerate shape with zero volume, which is legal in rapier and means the
/// collider contributes no mass and no contact — a silent no-op that looks like
/// a physics bug. Clamping keeps the body collidable.
pub fn build3(
    desc: &ColliderDesc,
    scale: glam::Vec3,
    friction: f32,
    restitution: f32,
) -> ColliderBuilder3 {
    let offset = desc.offset * scale;
    let mut builder = match desc.shape {
        Shape::Box { half_extents } => ColliderBuilder3::cuboid(
            positive(half_extents.x * scale.x),
            positive(half_extents.y * scale.y),
            positive(half_extents.z * scale.z),
        ),
        Shape::Sphere { radius } => ColliderBuilder3::ball(positive(radius * max_scale(scale))),
        Shape::Capsule {
            half_height,
            radius,
        } => ColliderBuilder3::capsule_y(
            positive(half_height * scale.y),
            positive(radius * max_scale(scale)),
        ),
    };
    builder = builder
        .translation(crate::convert::to_vec3(offset))
        .friction(desc.friction.unwrap_or(friction))
        .restitution(desc.restitution.unwrap_or(restitution));
    builder
}

/// Build a 2D collider, applying the body's scale. See [`build3`].
pub fn build2(
    desc: &ColliderDesc2,
    scale: Vec2,
    friction: f32,
    restitution: f32,
) -> ColliderBuilder2 {
    ColliderBuilder2::cuboid(
        positive(desc.half_extents.x * scale.x),
        positive(desc.half_extents.y * scale.y),
    )
    .translation(crate::convert::to_vec2(desc.offset * scale))
    .friction(desc.friction.unwrap_or(friction))
    .restitution(desc.restitution.unwrap_or(restitution))
}

/// The smallest half-extent or radius femur will build, in metres.
///
/// 1 mm. Small enough to be invisible in a game world, large enough that the
/// shape has a non-zero volume and therefore contributes to a body's mass and
/// its inertia — which is what stops a typo'd `0.0` from turning a crate into
/// a ghost.
const MIN_DIMENSION: f32 = 1.0e-3;

fn positive(value: f32) -> f32 {
    if value.is_finite() && value.abs() > MIN_DIMENSION {
        value.abs()
    } else {
        MIN_DIMENSION
    }
}

/// Re-exported so a caller can talk about shapes without naming parry.
///
/// `SharedShape` is a parry type, not an nalgebra one, so this is not a leak of
/// the second math type — but the *builder* it returns is femu's own API
/// surface in practice, and having the shape type reachable from `femur` is
/// what keeps callers from reaching into `parry3d` themselves.
pub type SharedShape3 = Shape3;

/// The 2D form of [`SharedShape3`].
pub type SharedShape2 = Shape2;
