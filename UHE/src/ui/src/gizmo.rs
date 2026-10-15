//! Gizmo maths. Drawn by us, not by egui_gizmo.

use glam::Vec3;

/// A gizmo's operation mode.
///
/// `Q` maps to [`GizmoMode::None`] rather than to translate, so it can never
/// collide with [`GizmoMode::Translate`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GizmoMode {
    None,
    Translate,
    Rotate,
    Scale,
}

pub type GizmoType = GizmoMode;

/// A ray.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    /// Create a ray. Direction is normalized.
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction: direction.normalize_or_zero(),
        }
    }

    /// Point at parameter `t`.
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }
}

/// Intersection of a ray with a plane.
pub fn ray_plane_intersection(ray: &Ray, normal: Vec3, point: Vec3) -> Option<Vec3> {
    let denominator = normal.dot(ray.direction);
    if denominator.abs() < f32::EPSILON {
        return None;
    }
    let t = (point - ray.origin).dot(normal) / denominator;
    if t < 0.0 {
        return None;
    }
    Some(ray.at(t))
}

/// Alias accepting owned values.
pub fn ray_plane_intersection_point(
    origin: Vec3,
    direction: Vec3,
    normal: Vec3,
    point: Vec3,
) -> Option<Vec3> {
    let ray = Ray::new(origin, direction);
    ray_plane_intersection(&ray, normal, point)
}

/// Alias.
pub fn intersect_ray_plane(ray: &Ray, normal: Vec3, point: Vec3) -> Option<Vec3> {
    ray_plane_intersection(ray, normal, point)
}

/// Alias.
pub fn ray_plane(ray: &Ray, normal: Vec3, point: Vec3) -> Option<Vec3> {
    ray_plane_intersection(ray, normal, point)
}

/// Alias.
pub fn intersect(ray: &Ray, normal: Vec3, point: Vec3) -> Option<Vec3> {
    ray_plane_intersection(ray, normal, point)
}

/// Plane defined by normal and point.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Plane {
    pub normal: Vec3,
    pub point: Vec3,
}

impl Plane {
    /// Create a plane.
    pub fn new(normal: Vec3, point: Vec3) -> Self {
        Self {
            normal: normal.normalize_or_zero(),
            point,
        }
    }
}

/// Ray-plane intersection using a plane value.
pub fn ray_plane_intersection_plane(ray: &Ray, plane: Plane) -> Option<Vec3> {
    ray_plane_intersection(ray, plane.normal, plane.point)
}

/// Known-ray test helper: plane z = 0.
pub fn z_plane_hit(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, 0.0)
}
