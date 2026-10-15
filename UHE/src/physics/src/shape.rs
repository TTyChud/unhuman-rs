//! Collider shapes: the three the C++ built, and the refusal for anything else.
//!
//! The C++ had one component per shape (`BoxCollider3DComponent`,
//! `SphereCollider3DComponent`, `CapsuleCollider3DComponent`, `Components.h:255-289`)
//! and a chain of `if`/`else if` in `Scene.cpp:1376-1407` that turned whichever was
//! present into a Jolt shape — creating nothing at all when none was. The port has one
//! `meats::Collider` carrying the shape, so the chain lives here.
//!
//! `ColliderShape` is `meats`'. It is re-exported because a caller of this crate names
//! it when it builds a collider; what this module owns is the part the backend adds —
//! the transform's scale, and the Rapier shape.

pub use meats::ColliderShape;

use glam::Vec3;
use meats::Collider;
use rapier3d::prelude::SharedShape;

/// Why a rigid body did not become a Rapier body.
#[derive(PartialEq)]
pub(crate) enum Refusal {
    /// A collider shape this backend does not build, named.
    Unsupported(String),
    /// A `RigidBody` with no `Collider` component, which the C++ also created nothing
    /// for (`if (shape)`, `Scene.cpp:1412`).
    Missing,
}

/// What an entity's collider gave the backend.
#[derive(PartialEq)]
pub(crate) enum ShapeSlot {
    Supported(ScaledShape),
    Refused(Refusal),
}

impl ShapeSlot {
    pub(crate) fn from_collider(collider: Option<&Collider>, scale: Vec3) -> ShapeSlot {
        let Some(collider) = collider else {
            return ShapeSlot::Refused(Refusal::Missing);
        };
        match scaled_shape(&collider.shape, scale) {
            Some(shape) => ShapeSlot::Supported(shape),
            None => ShapeSlot::Refused(Refusal::Unsupported(format!("{:?}", collider.shape))),
        }
    }

    pub(crate) fn shared(&self) -> Result<SharedShape, Refusal> {
        match self {
            ShapeSlot::Supported(shape) => Ok((*shape).shared()),
            ShapeSlot::Refused(Refusal::Unsupported(name)) => {
                Err(Refusal::Unsupported(name.clone()))
            }
            ShapeSlot::Refused(Refusal::Missing) => Err(Refusal::Missing),
        }
    }
}

/// A scene shape under the transform's scale, in this crate's own terms.
///
/// Owned here rather than mirroring `meats::ColliderShape`, so change detection compares
/// something with a `PartialEq` this crate controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScaledShape {
    Box(Vec3),
    Sphere(f32),
    Capsule { radius: f32, half_height: f32 },
}

impl ScaledShape {
    fn shared(self) -> SharedShape {
        match self {
            ScaledShape::Box(half_extent) => {
                SharedShape::cuboid(half_extent.x, half_extent.y, half_extent.z)
            }
            ScaledShape::Sphere(radius) => SharedShape::ball(radius),
            ScaledShape::Capsule {
                radius,
                half_height,
            } => SharedShape::capsule_y(half_height, radius),
        }
    }
}

/// The scaled shape, or `None` for one this backend does not build.
///
/// Scaling is the C++'s: a box per axis, a sphere by its largest axis, a capsule's radius
/// by the larger of x and z and its half-height by y (`Scene.cpp:1377-1407`).
fn scaled_shape(shape: &ColliderShape, scale: Vec3) -> Option<ScaledShape> {
    match shape {
        ColliderShape::Box { half_extent } => Some(ScaledShape::Box(*half_extent * scale)),
        ColliderShape::Sphere { radius } => Some(ScaledShape::Sphere(
            *radius * scale.x.max(scale.y).max(scale.z),
        )),
        ColliderShape::Capsule {
            radius,
            half_height,
        } => Some(ScaledShape::Capsule {
            radius: *radius * scale.x.max(scale.z),
            half_height: *half_height * scale.y,
        }),
        // Unreachable while every `meats::ColliderShape` variant is one of the three. A
        // shape `meats` grows is refused by name rather than dropped, which is what the
        // C++'s `if (shape)` guard would have done.
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_follows_the_cpp() {
        let scale = Vec3::new(2.0, 3.0, 4.0);

        let box_shape = ColliderShape::Box {
            half_extent: Vec3::splat(0.5),
        };
        assert_eq!(
            scaled_shape(&box_shape, scale),
            Some(ScaledShape::Box(Vec3::new(1.0, 1.5, 2.0)))
        );

        // A sphere scales by its largest axis (`Scene.cpp:1394`).
        let sphere_shape = ColliderShape::Sphere { radius: 0.5 };
        assert_eq!(
            scaled_shape(&sphere_shape, scale),
            Some(ScaledShape::Sphere(2.0))
        );

        // A capsule's radius scales by the larger of x and z, its half-height by y
        // (`Scene.cpp:1402-1403`).
        let capsule_shape = ColliderShape::Capsule {
            radius: 0.5,
            half_height: 1.0,
        };
        assert_eq!(
            scaled_shape(&capsule_shape, scale),
            Some(ScaledShape::Capsule {
                radius: 2.0,
                half_height: 3.0
            })
        );
    }

    #[test]
    fn a_rigid_body_without_a_collider_is_refused_as_missing() {
        let slot = ShapeSlot::from_collider(None, Vec3::ONE);
        assert!(matches!(slot.shared(), Err(Refusal::Missing)));
    }

    #[test]
    fn every_scene_shape_becomes_a_rapier_shape() {
        let shapes = [
            ColliderShape::Box {
                half_extent: Vec3::splat(0.5),
            },
            ColliderShape::Sphere { radius: 0.5 },
            ColliderShape::Capsule {
                radius: 0.5,
                half_height: 1.0,
            },
        ];
        for shape in shapes {
            let collider = Collider {
                shape,
                offset: Vec3::ZERO,
                friction: 0.2,
                restitution: 0.0,
            };
            let slot = ShapeSlot::from_collider(Some(&collider), Vec3::splat(2.0));
            assert!(slot.shared().is_ok(), "{shape:?}");
        }
    }
}
