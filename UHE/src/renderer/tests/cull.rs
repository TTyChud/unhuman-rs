//! Frustum culling, and the matrices it depends on.
//!
//! The tests that matter here are the ones that make the culler *drop* something.
//! A cull test that only proves items are kept passes just as happily when the
//! frustum is empty, the planes are all zero, or the projection is transposed —
//! every one of which renders a scene of invisible objects.

use bifrost::bounds::Sphere;
use bifrost::cull::{cull, Camera, Frustum, Projection};
use bifrost::drawlist::{DrawItem, MaterialId, MeshId, PipelineId};
use bifrost::shadow::{cascade_corners, directional_shadow, Cascade};
use glam::{Mat4, Vec3, Vec4};

fn camera() -> Camera {
    // At the origin, looking down -Z, 90 degrees vertical fov, 1:1.
    Camera::perspective(
        Vec3::ZERO,
        Vec3::NEG_Z,
        std::f32::consts::FRAC_PI_2,
        1.0,
        0.1,
        100.0,
    )
}

fn item_at(position: Vec3, radius: f32) -> DrawItem {
    DrawItem::new(
        MeshId(0),
        MaterialId(0),
        PipelineId(0),
        Mat4::from_translation(position),
        Sphere::new(Vec3::ZERO, radius),
    )
}

#[test]
fn the_near_plane_maps_to_zero_and_the_far_plane_to_one() {
    // Proves the projection is depth-zero-to-one. With GL's [-1, 1] range these
    // come out as -1 and 1, and the depth test silently clamps everything past the
    // near plane onto z = 0.
    let projection = camera().projection_matrix();
    let near = projection * Vec4::new(0.0, 0.0, -0.1, 1.0);
    let far = projection * Vec4::new(0.0, 0.0, -100.0, 1.0);
    assert!((near.z / near.w).abs() < 1e-5, "near plane is not z = 0");
    assert!((far.z / far.w - 1.0).abs() < 1e-5, "far plane is not z = 1");
}

#[test]
fn an_object_in_front_of_the_camera_survives() {
    let frustum = camera().frustum();
    let mut items = vec![item_at(Vec3::new(0.0, 0.0, -5.0), 0.5)];
    let stats = cull(&mut items, &frustum);
    assert_eq!(items.len(), 1);
    assert_eq!(stats.culled, 0);
    assert_eq!(stats.tested, 1);
}

#[test]
fn an_object_behind_the_camera_is_culled() {
    let frustum = camera().frustum();
    let mut items = vec![item_at(Vec3::new(0.0, 0.0, 5.0), 0.5)];
    let stats = cull(&mut items, &frustum);
    assert!(
        items.is_empty(),
        "an object behind the camera was submitted"
    );
    assert_eq!(stats.culled, 1);
}

#[test]
fn an_object_beside_the_camera_is_culled() {
    let frustum = camera().frustum();
    // 90 degrees vertical and 1:1 aspect means the horizontal half-angle is also
    // 45 degrees, so x = 50 at z = -5 is far outside.
    let mut items = vec![item_at(Vec3::new(50.0, 0.0, -5.0), 0.5)];
    assert_eq!(cull(&mut items, &frustum).culled, 1);
    assert!(items.is_empty());
}

#[test]
fn a_sphere_reaching_into_the_frustum_is_kept() {
    // The cull is conservative: a sphere whose centre is outside but whose body
    // crosses the plane is still submitted. Getting this wrong is a hole in the
    // image at the screen edge, which is where nobody looks while testing.
    let frustum = camera().frustum();
    let mut items = vec![item_at(Vec3::new(5.2, 0.0, -5.0), 1.0)];
    assert_eq!(cull(&mut items, &frustum).culled, 0);
    assert_eq!(items.len(), 1);
}

#[test]
fn the_culler_reports_what_it_dropped() {
    let frustum = camera().frustum();
    let mut items = vec![
        item_at(Vec3::new(0.0, 0.0, -5.0), 0.5),
        item_at(Vec3::new(0.0, 0.0, 5.0), 0.5),
        item_at(Vec3::new(0.0, 0.0, 50.0), 0.5), // beyond the far plane
        item_at(Vec3::new(-2.0, 0.0, -8.0), 0.5),
    ];
    let stats = cull(&mut items, &frustum);
    assert_eq!(stats.tested, 4);
    assert_eq!(stats.culled, 2, "behind and beyond-far should go");
    assert_eq!(items.len(), 2);
    assert!((stats.fraction_culled() - 0.5).abs() < 1e-6);
}

#[test]
fn a_degenerate_matrix_culls_nothing() {
    // NaN comparisons are false, so a NaN plane would fail "is this outside" for
    // every item and delete the entire scene. The zero matrix is the cheap way to
    // prove the guard exists.
    let frustum = Frustum::from_view_projection(Mat4::ZERO);
    let mut items = vec![
        item_at(Vec3::new(0.0, 0.0, -5.0), 0.5),
        item_at(Vec3::new(1000.0, 0.0, 1000.0), 0.5),
    ];
    assert_eq!(cull(&mut items, &frustum).culled, 0);
    assert_eq!(items.len(), 2);
}

#[test]
fn an_orthographic_camera_culls_outside_its_box() {
    let camera = Camera::orthographic(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, 5.0, 5.0, 0.1, 20.0);
    let frustum = camera.frustum();
    let mut items = vec![
        item_at(Vec3::ZERO, 0.5),
        item_at(Vec3::new(20.0, 0.0, 0.0), 0.5),
        item_at(Vec3::new(0.0, 0.0, 100.0), 0.5),
    ];
    assert_eq!(cull(&mut items, &frustum).culled, 2);
    assert_eq!(items.len(), 1);
}

#[test]
fn distance_to_the_camera_drives_the_sort_key() {
    let camera = camera();
    let near = camera.distance_to(&Sphere::new(Vec3::new(0.0, 0.0, -5.0), 0.5));
    let far = camera.distance_to(&Sphere::new(Vec3::new(0.0, 0.0, -50.0), 0.5));
    assert!((near - 5.0).abs() < 1e-5);
    assert!((far - 50.0).abs() < 1e-5);
    assert!(near < far);
}

#[test]
fn a_sphere_from_an_aabb_contains_the_box() {
    // A glTF accessor min/max is a box, and the sphere must contain all of it —
    // otherwise the loader produces bounds that cull visible geometry.
    let centre = Vec3::new(1.0, 2.0, 3.0);
    let half = Vec3::new(2.0, 0.5, 1.0);
    let sphere = Sphere::from_aabb(centre, half);
    for x in [-2.0, 2.0] {
        for y in [-0.5, 0.5] {
            for z in [-1.0, 1.0] {
                let corner = centre + Vec3::new(x, y, z);
                assert!(
                    sphere.contains_point(corner),
                    "corner {corner:?} escaped the bounds sphere"
                );
            }
        }
    }
    assert!(sphere.contains_point(centre));
}

#[test]
fn a_transformed_bounds_sphere_follows_the_model_matrix() {
    let bounds = Sphere::new(Vec3::ZERO, 1.0);
    let scaled = bounds.transformed(Mat4::from_scale(Vec3::splat(3.0)));
    assert!((scaled.radius - 3.0).abs() < 1e-5);
    let moved = bounds.transformed(Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)));
    assert_eq!(moved.centre, Vec3::new(5.0, 0.0, 0.0));
    assert_eq!(moved.radius, bounds.radius, "a rigid move does not scale");
}

#[test]
fn cascade_corners_land_on_the_slice() {
    // The four near corners must be at view depth `near` and the far four at
    // `far`. Interpolating between two unprojected quads — the usual shortcut —
    // does not do this, and the error shows up as a shadow map that stops short of
    // the geometry it is meant to cover.
    let camera = camera();
    let cascade = Cascade {
        near: 2.0,
        far: 20.0,
    };
    let corners = cascade_corners(&camera, cascade);
    let view = camera.view();

    for corner in &corners[..4] {
        let view_space = view * Vec4::from((*corner, 1.0));
        assert!(
            (view_space.z + 2.0).abs() < 1e-3,
            "near corner at z {}",
            view_space.z
        );
    }
    for corner in &corners[4..] {
        let view_space = view * Vec4::from((*corner, 1.0));
        assert!(
            (view_space.z + 20.0).abs() < 1e-3,
            "far corner at z {}",
            view_space.z
        );
    }
    // With a 90 degree vertical fov the half-width at the far plane is 20.
    let far_x: f32 = corners[4..]
        .iter()
        .map(|c| view * Vec4::from((*c, 1.0)))
        .map(|v| v.x.abs())
        .fold(0.0f32, f32::max);
    assert!((far_x - 20.0).abs() < 1e-3, "far half-width {far_x}");
}

#[test]
fn the_shadow_camera_covers_the_whole_slice() {
    // The property that matters: every corner of the cascade projects inside the
    // shadow map. A fit that is tight but clips a corner is missing that corner's
    // shadows, and it is a seam rather than an obvious failure.
    let camera = camera();
    let cascade = Cascade {
        near: 1.0,
        far: 50.0,
    };
    let shadow = directional_shadow(&camera, cascade, Vec3::new(-1.0, -2.0, -0.5))
        .expect("a valid cascade and light direction fit");
    let matrix = shadow.view_projection();

    for corner in cascade_corners(&camera, cascade) {
        let clip = matrix * Vec4::from((corner, 1.0));
        let ndc = clip.truncate() / clip.w;
        assert!(clip.w > 0.0, "corner behind the shadow camera: {corner:?}");
        assert!(
            ndc.x.abs() <= 1.0 + 1e-4,
            "x {ndc:?} escaped the shadow map"
        );
        assert!(
            ndc.y.abs() <= 1.0 + 1e-4,
            "y {ndc:?} escaped the shadow map"
        );
        assert!((0.0..=1.0).contains(&ndc.z), "z {ndc:?} outside [0, 1]");
    }
}

#[test]
fn a_noon_sun_still_produces_a_usable_matrix() {
    // A light straight down is parallel to +Y, which is the up vector everyone
    // hardcodes. The degenerate look_at is a NaN matrix, and a NaN matrix quietly
    // removes every shadow in the scene.
    let camera = camera();
    let cascade = Cascade {
        near: 1.0,
        far: 20.0,
    };
    let shadow = directional_shadow(&camera, cascade, Vec3::new(0.0, -1.0, 0.0))
        .expect("a vertical sun is not a degenerate one");
    let matrix = shadow.view_projection();
    assert!(
        matrix.to_cols_array().iter().all(|v| v.is_finite()),
        "the shadow view-projection has a non-finite element"
    );
}

#[test]
fn an_inverted_or_empty_cascade_is_refused() {
    let camera = camera();
    assert!(directional_shadow(
        &camera,
        Cascade {
            near: 5.0,
            far: 1.0
        },
        Vec3::NEG_Y
    )
    .is_none());
    assert!(directional_shadow(
        &camera,
        Cascade {
            near: 0.0,
            far: 0.0
        },
        Vec3::NEG_Y
    )
    .is_none());
    // A zero light direction has no direction, so there is no matrix to fit.
    assert!(directional_shadow(
        &camera,
        Cascade {
            near: 1.0,
            far: 10.0
        },
        Vec3::ZERO
    )
    .is_none());
}

#[test]
fn cascades_split_a_range_into_slices() {
    let cascades = Cascade::splits(100.0, 4);
    assert_eq!(cascades.len(), 4);
    assert_eq!(cascades[0].near, 0.0);
    assert_eq!(cascades[3].far, 100.0);
    for pair in cascades.windows(2) {
        assert_eq!(
            pair[0].far, pair[1].near,
            "the slices must not overlap or gap"
        );
    }
    // A count of zero would otherwise produce an empty list and a frame with no
    // shadows, which is not a failure anybody would look for.
    assert_eq!(Cascade::splits(10.0, 0).len(), 1);
}

#[test]
fn a_projection_is_one_of_two_shapes() {
    let perspective = camera();
    assert!(matches!(
        perspective.projection,
        Projection::Perspective { .. }
    ));
    let ortho = Camera::orthographic(Vec3::ZERO, Vec3::NEG_Z, 1.0, 1.0, 0.1, 10.0);
    assert!(matches!(ortho.projection, Projection::Orthographic { .. }));
    assert!(ortho
        .view_projection()
        .to_cols_array()
        .iter()
        .all(|v| v.is_finite()));
}
