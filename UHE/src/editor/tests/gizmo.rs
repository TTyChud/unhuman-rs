//! Gizmo math against a camera both ends of the workspace can agree on: an
//! orthographic view of the origin, so every screen position is arithmetic.

use glam::{Quat, Vec2, Vec3};
use mirror::gizmo::{
    snap_angle, snap_scale, snap_translate, Axis, Camera, GizmoMode, GizmoTransform, Snap,
    TransformGizmo,
};

fn camera() -> Camera {
    Camera {
        view: glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y),
        projection: glam::camera::rh::proj::directx::orthographic(-1.0, 1.0, -1.0, 1.0, 0.1, 100.0),
        viewport_size: Vec2::new(800.0, 600.0),
    }
}

fn set_rotation(scene: &mut meats::Scene, entity: meats::Entity, degrees: Vec3) {
    if let Some(mut entry) = scene.get_mut(entity) {
        if let Some(transform) = entry.get_mut::<meats::Transform>() {
            transform.rotation = degrees;
        }
    }
}

#[test]
fn the_orthographic_view_puts_world_units_on_known_pixels() {
    let cam = camera();
    let at = |world| cam.project(world).unwrap();
    // 2 world units across 800 px: 400 px per unit, y flipped.
    assert!((at(Vec3::new(1.0, 0.0, 0.0)).x - 800.0).abs() < 1e-3);
    assert!((at(Vec3::new(0.0, 1.0, 0.0)).y - 0.0).abs() < 1e-3);
    assert!((at(Vec3::ZERO).x - 400.0).abs() < 1e-3);
    assert!((at(Vec3::ZERO).y - 300.0).abs() < 1e-3);
}

#[test]
fn an_axis_is_picked_at_the_pixels_it_is_drawn_over() {
    let cam = camera();
    let target = GizmoTransform::IDENTITY;
    assert_eq!(
        TransformGizmo::pick_axis(&cam, &target, Vec2::new(600.0, 300.0)),
        Some(Axis::X)
    );
    assert_eq!(
        TransformGizmo::pick_axis(&cam, &target, Vec2::new(400.0, 200.0)),
        Some(Axis::Y)
    );
    // Off the arrows entirely.
    assert_eq!(
        TransformGizmo::pick_axis(&cam, &target, Vec2::new(700.0, 500.0)),
        None
    );
}

#[test]
fn dragging_translate_moves_the_expected_distance() {
    let cam = camera();
    let mut gizmo = TransformGizmo::new();
    assert_eq!(gizmo.mode, GizmoMode::Translate);
    let target = GizmoTransform::IDENTITY;

    assert!(gizmo.begin(&cam, &target, Vec2::new(400.0, 300.0), Axis::X));
    // 400 px per world unit, so 160 px is 0.4 units along local X.
    let dragged = gizmo.update(Vec2::new(560.0, 300.0)).unwrap();
    assert!((dragged.translation.x - 0.4).abs() < 1e-4);
    assert!(dragged.translation.y.abs() < 1e-6);
    assert!(dragged.translation.z.abs() < 1e-6);
    gizmo.end();
}

#[test]
fn snapping_rounds_the_drag_to_a_grid() {
    let cam = camera();
    let mut gizmo = TransformGizmo::new();
    gizmo.snap = Snap {
        enabled: true,
        translate: 0.5,
        rotate_degrees: 45.0,
        scale: 0.5,
    };
    assert!(gizmo.begin(
        &cam,
        &GizmoTransform::IDENTITY,
        Vec2::new(400.0, 300.0),
        Axis::Y
    ));
    // 80 px up the screen is 0.2 world units, which the grid lifts to 0.5.
    let dragged = gizmo.update(Vec2::new(400.0, 220.0)).unwrap();
    assert!((dragged.translation.y - 0.5).abs() < 1e-4);

    assert_eq!(snap_translate(0.2, 0.5), 0.0);
    assert_eq!(snap_translate(0.3, 0.5), 0.5);
    let quarter = std::f32::consts::FRAC_PI_4;
    assert!((snap_angle(quarter + 0.05, 45.0) - quarter).abs() < 1e-5);
    assert_eq!(snap_scale(1.6, 0.5), 1.5);
}

#[test]
fn scale_and_rotate_drag_along_their_own_axis() {
    let cam = camera();

    let mut scaler = TransformGizmo::new();
    scaler.mode = GizmoMode::Scale;
    assert!(scaler.begin(
        &cam,
        &GizmoTransform::IDENTITY,
        Vec2::new(400.0, 300.0),
        Axis::Y
    ));
    let scaled = scaler.update(Vec2::new(400.0, 200.0)).unwrap();
    assert!((scaled.scale.y - 2.0).abs() < 1e-4);
    assert!((scaled.scale.x - 1.0).abs() < 1e-6);

    let mut rotator = TransformGizmo::new();
    rotator.mode = GizmoMode::Rotate;
    rotator.snap.enabled = true;
    assert!(rotator.begin(
        &cam,
        &GizmoTransform::IDENTITY,
        Vec2::new(500.0, 300.0),
        Axis::X
    ));
    // A quarter turn of the pointer about the gizmo origin, snapped to 45.
    let rotated = rotator.update(Vec2::new(500.0, 400.0)).unwrap();
    assert!(
        (rotated.rotation.angle_between(Quat::IDENTITY) - std::f32::consts::FRAC_PI_4).abs() < 1e-4
    );
}

#[test]
fn a_gizmo_whose_axis_faces_the_camera_cannot_be_grabbed() {
    let cam = camera();
    let mut gizmo = TransformGizmo::new();
    assert!(!gizmo.begin(
        &cam,
        &GizmoTransform::IDENTITY,
        Vec2::new(400.0, 300.0),
        Axis::Z
    ));
    assert!(!gizmo.is_dragging());
    assert!(gizmo.update(Vec2::new(500.0, 300.0)).is_none());
}

#[test]
fn the_component_angles_round_trip_through_its_own_quaternion() {
    use meats::{Scene, Transform};

    // About all three axes, so a conversion that only works on a single axis
    // cannot pass.
    let authored = Vec3::new(30.0, 20.0, 10.0);

    let mut scene = Scene::new();
    let entity = scene.spawn("Rotated");
    set_rotation(&mut scene, entity, authored);
    let quaternion = scene
        .get(entity)
        .and_then(|entry| entry.get::<Transform>().map(|t| t.rotation_quat()))
        .expect("a Transform");

    let back = meats::euler_degrees(quaternion);
    for (got, want) in [
        (back.x, authored.x),
        (back.y, authored.y),
        (back.z, authored.z),
    ] {
        assert!((got - want).abs() < 1e-4, "{back:?} is not {authored:?}");
    }
}

#[test]
fn a_compound_gizmo_rotation_lands_where_the_gizmo_put_it() {
    use meats::{Scene, Transform};
    use mirror::Editor;

    let authored = Vec3::new(30.0, 20.0, 10.0);
    let mut scene = Scene::new();
    let entity = scene.spawn("Rotated");

    // The gizmo drags quaternions: take the orientation these angles mean, then
    // blank the component, so a write-back that does nothing cannot pass.
    set_rotation(&mut scene, entity, authored);
    let dragged = scene
        .get(entity)
        .and_then(|entry| entry.get::<Transform>().map(|t| t.rotation_quat()))
        .expect("a Transform");
    set_rotation(&mut scene, entity, Vec3::ZERO);

    let mut editor = Editor::new();
    editor.select(Some(entity));
    let target = GizmoTransform {
        translation: Vec3::ZERO,
        rotation: dragged,
        scale: Vec3::ONE,
    };
    assert!(editor.apply_gizmo(&mut scene, &target));

    let stored = scene
        .get(entity)
        .and_then(|entry| entry.get::<Transform>().map(|t| t.rotation))
        .expect("a Transform");
    for (got, want) in [
        (stored.x, authored.x),
        (stored.y, authored.y),
        (stored.z, authored.z),
    ] {
        assert!((got - want).abs() < 0.1, "{stored:?} is not {authored:?}");
    }
}

#[test]
fn a_dragged_child_is_written_back_in_its_parents_space() {
    use meats::{Scene, Transform};
    use mirror::Editor;

    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.set_parent(child, Some(parent));
    {
        let mut entry = scene.get_mut(parent).unwrap();
        entry.get_mut::<Transform>().unwrap().translation = Vec3::new(2.0, 0.0, 0.0);
    }
    scene.update_transforms();

    let mut editor = Editor::new();
    editor.select(Some(child));

    let target = mirror::gizmo_target(&scene, child);
    assert!((target.translation.x - 2.0).abs() < 1e-4, "{target:?}");

    // The gizmo hands back a world transform: five metres along X.
    let dragged = GizmoTransform {
        translation: Vec3::new(5.0, 0.0, 0.0),
        ..target
    };
    assert!(editor.apply_gizmo(&mut scene, &dragged));

    let local = scene
        .get(child)
        .and_then(|entry| entry.get::<Transform>().map(|t| t.translation))
        .unwrap();
    assert!((local.x - 3.0).abs() < 1e-4, "{local:?}");
    assert!(local.y.abs() < 1e-4);

    while editor.undo(&mut scene) {}
    let restored = scene
        .get(child)
        .and_then(|entry| entry.get::<Transform>().map(|t| t.translation))
        .unwrap();
    assert!(restored.x.abs() < 1e-4, "{restored:?}");
}
