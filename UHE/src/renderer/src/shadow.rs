//! Directional shadow fitting: one orthographic camera per cascade.
//!
//! ## Why this is separate from the culler
//!
//! A shadow camera is a projection like any other, but it is fitted to a *slice*
//! of another camera's frustum rather than to the viewport, and the fitting has
//! its own failure mode: a shadow map that does not cover the cascade stops
//! casting shadows exactly where the seam is. That is a different bug from an
//! object failing to be culled, so it gets its own file.
//!
//! ## Sphere fit, not box fit, and this is the whole point
//!
//! The orthographic box is fitted to the sphere around the cascade slice's eight
//! corners. Fitting to the box instead would give a smaller shadow map for the
//! same geometry — cheaper in one way, and wrong in the way that matters:
//!
//! A box fit gives the same world-units-per-texel along its longest axis and a
//! *different* texel density along the other two, which varies with the cascade's
//! distance and aspect. Cascades therefore do not agree on texel size, a surface
//! crossing a cascade boundary is sampled at two different densities, and the
//! shadow edge crawls as the camera moves. That is cascade seam shimmer, and
//! session.md Phase 6 calls it out as the test for the phase. A sphere fit makes
//! every cascade's texel size a function of its distance alone, so the densities
//! match at the boundary and the seam disappears.
//!
//! The cost is real and worth naming: the shadow map covers the sphere's
//! silhouette, so up to 42% of its texels (circle/square) are wasted on geometry
//! outside the box. It is texels in a map nobody sees, traded for a seam that
//! nobody has to debug.
//!
//! ## The up-vector trap
//!
//! `look_at_mat4` is degenerate when the eye direction is parallel to `up`, and a
//! sun straight overhead — noon, or a scene with no horizon — is exactly that. The
//! result is a NaN view matrix, which then silently removes every shadow. The
//! fallback axis is chosen from the light direction rather than hardcoded, so a
//! light along *any* axis is covered.

use glam::camera::rh::proj::directx::orthographic;
use glam::camera::rh::view::look_at_mat4;
use glam::{Mat4, Vec3, Vec4};

use crate::cull::Camera;

/// A distance slice of the camera's view volume, in world units along the view
/// axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cascade {
    /// Near edge of the slice, measured from the camera position.
    pub near: f32,
    pub far: f32,
}

impl Cascade {
    /// Split `[0, far]` into `count` slices of equal length.
    ///
    /// Equal-length rather than logarithmic: a practical split puts most of the
    /// depth range in the first cascade, which is a tuning decision about how
    /// much texel density to spend near the camera. Equal slices are the neutral
    /// choice and the split is one number to change.
    pub fn splits(far: f32, count: u32) -> Vec<Self> {
        let count = count.max(1);
        let step = far / count as f32;
        (0..count)
            .map(|i| Self {
                near: step * i as f32,
                far: step * (i + 1) as f32,
            })
            .collect()
    }

    pub fn is_valid(&self) -> bool {
        self.far > self.near && self.near >= 0.0
    }
}

/// An orthographic camera looking down a light's direction.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShadowCamera {
    pub view: Mat4,
    pub projection: Mat4,
}

impl ShadowCamera {
    pub fn view_projection(&self) -> Mat4 {
        self.projection * self.view
    }
}

/// The eight world-space corners of a cascade slice: four at `near`, four at
/// `far`.
///
/// Exact rather than interpolated between the near and far quad. Interpolating
/// between two unprojected quads is the common shortcut and it is wrong at the
/// edges: the segment from a near-plane corner to its far-plane corner is a ray
/// from the eye, and a point partway along it is *not* at the view depth the
/// cascade asked for. The error grows with field of view and with how far along
/// the slice it is, which puts the shadow map's edge short of the geometry it is
/// meant to cover — a shadow that stops working as you look to the side.
///
/// The corners are computed from the four corner *rays* scaled by the view depth,
/// so each is exactly at `z = -d` in view space.
pub fn cascade_corners(camera: &Camera, cascade: Cascade) -> [Vec3; 8] {
    let inverse = camera.projection_matrix().inverse();
    let view = camera.view();
    let mut corners = [Vec3::ZERO; 8];
    for (index, (ndc_x, ndc_y)) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        let near_point = inverse * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let near_point = near_point.truncate() / near_point.w;
        // A view-space direction with z = -1, so scaling by `d` lands the point
        // at view depth `d`. `look_at_mat4`'s view space looks down -Z.
        let direction = Vec3::new(
            near_point.x / -near_point.z,
            near_point.y / -near_point.z,
            -1.0,
        );
        corners[index] = view.transform_point3(direction * cascade.near);
        corners[index + 4] = view.transform_point3(direction * cascade.far);
    }
    corners
}

/// Fit an orthographic camera to a cascade slice.
///
/// `light_direction` is the direction the light **travels**, matching the glTF
/// and the convention every directional light in the scene data uses. The eye is
/// placed against it: at `centre - light_direction * radius`, so the camera is on
/// the side the light comes from. Reversing the sign produces a correct-looking
/// matrix that renders the shadow map from the far side of the geometry, and the
/// result is a shadow that is not there.
pub fn directional_shadow(
    camera: &Camera,
    cascade: Cascade,
    light_direction: Vec3,
) -> Option<ShadowCamera> {
    if !cascade.is_valid() {
        return None;
    }
    let corners = cascade_corners(camera, cascade);
    let centre = corners.iter().fold(Vec3::ZERO, |sum, c| sum + *c) / 8.0;
    // The largest distance from the centre, not the average: an average radius
    // clips the corners of the slice, and clipped corners mean missing shadows.
    let radius = corners
        .iter()
        .map(|c| c.distance(centre))
        .fold(0.0f32, f32::max);

    let direction = light_direction.normalize_or_zero();
    if radius <= 0.0 || direction == Vec3::ZERO {
        return None;
    }

    let eye = centre - direction * radius;
    let view = look_at_mat4(eye, centre, shadow_up(direction));
    // `near` at zero: the eye is already one radius outside the slice, so the
    // depth range only has to reach the far side of it. Clamping to something
    // positive would either clip the near geometry or waste precision on an
    // empty metre of shadow map.
    let projection = orthographic(-radius, radius, -radius, radius, 0.0, radius * 2.0);
    Some(ShadowCamera { view, projection })
}

/// An axis that is not parallel to `direction`.
///
/// The world axis least aligned with the light, which makes degeneracy
/// impossible by construction rather than by a check: the chosen axis has a
/// component of at most 1/sqrt(3) of the direction's length, so `look_at_mat4`
/// always has a cross product to normalise. `+Y` alone — the usual choice — is
/// parallel to a noon sun, and the resulting NaN view matrix silently removes
/// every shadow in the scene.
fn shadow_up(direction: Vec3) -> Vec3 {
    let axis = direction.abs();
    if axis.y <= axis.x && axis.y <= axis.z {
        Vec3::Y
    } else if axis.x <= axis.z {
        Vec3::X
    } else {
        Vec3::Z
    }
}
