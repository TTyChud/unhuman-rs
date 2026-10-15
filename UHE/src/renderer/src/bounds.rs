//! Mesh bounds.
//!
//! ## Why a sphere, and why it lives in its own file
//!
//! A sphere is the one geometric noun that three concerns need and none of them
//! should own: the loader fills it from a glTF accessor `min`/`max`, the draw
//! list carries it per item, and the culler tests it against six planes. Put it
//! in any one of those and the other two import it from a module whose name says
//! nothing about bounds, which is how a dependency arrow quietly points sideways.
//!
//! A box would be tighter — noticeably so for a long wall or a flat ground plane,
//! which is most of a level. It loses anyway, because a box-versus-plane test
//! needs the box's axes and its extent per axis, and that is six tests per plane
//! with a matrix multiply in front. The sphere is conservative: it over-culls
//! nothing visible, and it over-includes long flat geometry, which costs a draw
//! call that the depth test then rejects. A visible object that was culled is a
//! hole in the image; an invisible object that was drawn is free.
//!
//! ## Object space, transformed at the point of use
//!
//! Stored untransformed, so editing a model matrix does not invalidate the
//! bounds and does not require re-reading the mesh. [`Sphere::transformed`] is
//! applied once per cull test rather than once per frame, because a cull test is
//! the only thing that reads it.

use glam::{Mat4, Vec3};

/// Centre and radius.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sphere {
    pub centre: Vec3,
    /// Negative is not clamped. A loader that computes a negative radius from a
    /// corrupt accessor min/max has a bug worth surfacing, and clamping here
    /// would turn it into geometry that silently cannot be culled — a scene that
    /// is correct until it gets large.
    pub radius: f32,
}

impl Sphere {
    /// A unit sphere at the origin: what a mesh with no usable accessor bounds
    /// gets, rather than a zero-radius sphere at the origin that culls the whole
    /// model away.
    pub const UNIT: Self = Self {
        centre: Vec3::ZERO,
        radius: 1.0,
    };

    pub const fn new(centre: Vec3, radius: f32) -> Self {
        Self { centre, radius }
    }

    /// The smallest sphere containing a box given its centre and half-extent,
    /// which is what a glTF `min`/`max` pair actually is.
    ///
    /// Loose on purpose. `half_extent.length()` is the sphere of the half-extent
    /// vector, and it contains the box, so anything culled as invisible really is
    /// invisible. The alternative — per-axis scaling during the cull test — is
    /// tighter and needs the model matrix, which the bounds deliberately do not
    /// hold.
    pub fn from_aabb(centre: Vec3, half_extent: Vec3) -> Self {
        Self {
            centre,
            radius: half_extent.length(),
        }
    }

    pub fn contains_point(&self, point: Vec3) -> bool {
        self.centre.distance_squared(point) <= self.radius * self.radius
    }

    /// The same sphere under a transform: centre transformed, radius scaled by
    /// the **largest** basis axis.
    ///
    /// Largest, not per-axis, because the radius has to stay a single number to
    /// be comparable against a plane's distance in one dot product. A uniform
    /// scale and every rigid transform — which is all a static level has — are
    /// exact under this rule; a non-uniformly scaled model gets a slightly larger
    /// sphere than necessary, and a slightly larger sphere is still a sphere that
    /// contains the model.
    pub fn transformed(&self, transform: Mat4) -> Self {
        let scale = transform
            .x_axis
            .length()
            .max(transform.y_axis.length())
            .max(transform.z_axis.length());
        Self {
            centre: transform.transform_point3(self.centre),
            radius: self.radius * scale,
        }
    }
}
