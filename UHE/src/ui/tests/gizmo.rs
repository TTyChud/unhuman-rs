use glam::Vec3;
use mirror::gizmo::{ray_plane_intersection, Ray};

#[test]
fn ray_plane_known_rays() {
    let ray = Ray::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -1.0));
    let hit = ray_plane_intersection(&ray, Vec3::Z, Vec3::ZERO).unwrap();
    assert_eq!(hit, Vec3::new(1.0, 2.0, 0.0));

    let parallel = Ray::new(Vec3::ZERO, Vec3::Y);
    assert!(ray_plane_intersection(&parallel, Vec3::X, Vec3::ZERO).is_none());
}
