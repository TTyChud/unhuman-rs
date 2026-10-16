//! CPU picking: a cursor into a ray, and a ray into the scene.
//!
//! The C++ editor rendered an `EntityID` target and read the pixel back. The
//! port has no integer colour target in `chassis::TextureFormat` and no
//! map/read-back path in the RHI, and a readback was only ever an approximation
//! of the ray test — so picking is arithmetic here and needs no surface, no
//! GPU, and no frame in flight.
//!
//! Bounds are `vault::MeshData`'s own (`bounds_min`/`bounds_max`, folded from
//! the positions by the loader), and each is tested in the mesh's *local*
//! space, which is both tighter and cheaper than a world-space box.

use glam::{Mat4, Vec3, Vec4};
use meats::{Entity, Scene};
use vault::Assets;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ray {
    pub origin: Vec3,
    /// Deliberately not normalised: a hit's parameter is then the same number
    /// in world space and inside every mesh's local space, so "nearest" is
    /// comparable across entities with different scales.
    pub direction: Vec3,
}

impl Ray {
    /// The ray through a cursor in a viewport, unprojected through the inverse
    /// view-projection. `None` for a viewport with no area, a cursor outside
    /// it, a matrix that does not invert, or a projection the cursor maps to
    /// nothing.
    ///
    /// The origin sits on the near plane, which is the closest a ray can be and
    /// still be inside what the view draws. That is all a pick needs; the eye
    /// itself is not recoverable from a view-projection.
    pub fn through_cursor(
        cursor: (f32, f32),
        viewport: (u32, u32),
        view_projection: Mat4,
    ) -> Option<Ray> {
        let width = viewport.0 as f32;
        let height = viewport.1 as f32;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        if cursor.0 < 0.0 || cursor.1 < 0.0 || cursor.0 > width || cursor.1 > height {
            return None;
        }

        let inverse = view_projection.try_inverse()?;
        // Window y points down; clip y points up.
        let ndc_x = cursor.0 / width * 2.0 - 1.0;
        let ndc_y = 1.0 - cursor.1 / height * 2.0;
        // Both depths lie on the same ray, so this does not care whether the
        // projection maps to a [0,1] or a [-1,1] depth range.
        let near = unproject(&inverse, ndc_x, ndc_y, 0.0)?;
        let far = unproject(&inverse, ndc_x, ndc_y, 1.0)?;

        let direction = far - near;
        if !direction.is_finite() || direction.length_squared() <= 1e-12 {
            return None;
        }
        Some(Ray {
            origin: near,
            direction,
        })
    }

    pub fn point_at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }

    /// The parameter where the ray enters an axis-aligned box, or `None`. A ray
    /// that starts inside the box enters at `0`.
    pub fn hit_bounds(&self, min: Vec3, max: Vec3) -> Option<f32> {
        let mut entry = f32::NEG_INFINITY;
        let mut exit = f32::INFINITY;
        for axis in 0..3 {
            let origin = self.origin[axis];
            let direction = self.direction[axis];
            let low = min[axis];
            let high = max[axis];

            if direction.abs() <= f32::EPSILON {
                // Parallel to this slab: inside it for the whole ray, or never.
                if origin < low || origin > high {
                    return None;
                }
                continue;
            }

            let inverse = 1.0 / direction;
            let mut near = (low - origin) * inverse;
            let mut far = (high - origin) * inverse;
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }
            entry = entry.max(near);
            exit = exit.min(far);
            if entry > exit {
                return None;
            }
        }
        if exit < 0.0 {
            return None;
        }
        Some(entry.max(0.0))
    }
}

fn unproject(inverse: &Mat4, x: f32, y: f32, depth: f32) -> Option<Vec3> {
    let clip = *inverse * Vec4::new(x, y, depth, 1.0);
    if clip.w.abs() <= 1e-9 {
        return None;
    }
    let world = clip.truncate() / clip.w;
    if !world.is_finite() {
        return None;
    }
    Some(world)
}

/// The nearest entity whose mesh bounds the ray hits.
///
/// A mesh with no handle, one the registry cannot resolve, one with no
/// geometry, or one whose transform does not invert is skipped: a scene with a
/// stale reference still picks everything else.
pub fn pick(scene: &Scene, assets: &Assets, ray: Ray) -> Option<Entity> {
    let mut nearest: Option<(f32, Entity)> = None;
    for sample in scene.meshes() {
        let Some(primitives) = assets.meshes(sample.mesh) else {
            continue;
        };
        let Some(inverse) = sample.transform.try_inverse() else {
            continue;
        };
        let local = Ray {
            origin: inverse.transform_point3(ray.origin),
            direction: inverse.transform_vector3(ray.direction),
        };
        for mesh in primitives {
            if mesh.positions.is_empty() {
                continue;
            }
            let min = Vec3::from_array(mesh.bounds_min);
            let max = Vec3::from_array(mesh.bounds_max);
            let Some(t) = local.hit_bounds(min, max) else {
                continue;
            };
            if nearest.is_none_or(|(best, _)| t < best) {
                nearest = Some((t, sample.entity));
            }
        }
    }
    nearest.map(|(_, entity)| entity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_centre_ray_follows_the_camera_axis() {
        let view =
            glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let perspective =
            glam::camera::rh::proj::directx::perspective(60f32.to_radians(), 4.0 / 3.0, 0.1, 100.0)
                * view;
        let ray = Ray::through_cursor((400.0, 300.0), (800, 600), perspective).unwrap();
        assert!((ray.direction.normalize() - Vec3::NEG_Z).length() < 1e-4);

        let ortho =
            glam::camera::rh::proj::directx::orthographic(-2.0, 2.0, -1.5, 1.5, 0.1, 100.0) * view;
        let ray = Ray::through_cursor((400.0, 300.0), (800, 600), ortho).unwrap();
        assert!((ray.direction.normalize() - Vec3::NEG_Z).length() < 1e-4);
        // Depth 0 is the near plane, a tenth of a unit in front of the eye.
        assert!((ray.origin - Vec3::new(0.0, 0.0, 4.9)).length() < 1e-3);
    }

    #[test]
    fn a_degenerate_matrix_gives_no_ray() {
        assert!(Ray::through_cursor((400.0, 300.0), (800, 600), Mat4::ZERO).is_none());
        let mut collapsed = Mat4::IDENTITY;
        collapsed.x_axis.x = 0.0;
        assert!(Ray::through_cursor((400.0, 300.0), (800, 600), collapsed).is_none());
    }

    #[test]
    fn a_cursor_outside_the_viewport_gives_no_ray() {
        let matrix =
            glam::camera::rh::proj::directx::orthographic(-1.0, 1.0, -1.0, 1.0, 0.1, 100.0);
        assert!(Ray::through_cursor((-1.0, 300.0), (800, 600), matrix).is_none());
        assert!(Ray::through_cursor((400.0, 601.0), (800, 600), matrix).is_none());
        assert!(Ray::through_cursor((400.0, 300.0), (0, 600), matrix).is_none());
    }

    #[test]
    fn a_box_is_hit_where_the_ray_crosses_it() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.0, 5.0),
            direction: Vec3::NEG_Z,
        };
        let hit = ray
            .hit_bounds(Vec3::splat(-1.0), Vec3::new(1.0, 1.0, 0.0))
            .unwrap();
        assert!((hit - 5.0).abs() < 1e-6);
        assert!((ray.point_at(hit) - Vec3::new(0.0, 0.0, 0.0)).length() < 1e-6);

        assert!(ray
            .hit_bounds(Vec3::new(2.0, 2.0, 2.0), Vec3::new(3.0, 3.0, 3.0))
            .is_none());
        // Behind the origin.
        assert!(ray
            .hit_bounds(Vec3::new(-1.0, -1.0, 6.0), Vec3::new(1.0, 1.0, 7.0))
            .is_none());
        // Starting inside enters at zero.
        let inside = Ray {
            origin: Vec3::ZERO,
            direction: Vec3::NEG_Z,
        };
        assert_eq!(
            inside.hit_bounds(Vec3::splat(-1.0), Vec3::splat(1.0)),
            Some(0.0)
        );
        // The parameter survives unnormalised directions, which is what makes
        // two entities comparable.
        let long = Ray {
            direction: Vec3::NEG_Z * 2.0,
            ..ray
        };
        assert!(
            (long
                .hit_bounds(Vec3::splat(-1.0), Vec3::splat(1.0))
                .unwrap()
                - 2.0)
                .abs()
                < 1e-6
        );
    }
}
