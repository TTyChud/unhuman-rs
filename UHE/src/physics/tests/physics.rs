//! The port's own gate: what physics must do to a scene, with no GPU, no window and no
//! audio — a `Scene` and a `Physics3D`, stepped by hand.
//!
//! The C++ had no equivalent: `tests/scene` recompiled engine `.cpp` files with a
//! stubbed renderer, and physics only ran inside play mode (`Scene::OnRuntimeStart`), so
//! none of this was testable.

use adam::PhysicsBackend;
use femur::{ColliderShape, Physics3D, GRAVITY};
use glam::Vec3;
use meats::{Collider, Entity, RigidBody, RigidBodyKind, Scene, Transform};

/// The engine's fixed step (60 Hz, `adam::Time::new`).
const DT: f32 = 1.0 / 60.0;

fn box_shape(half_extent: Vec3) -> ColliderShape {
    ColliderShape::Box { half_extent }
}

/// Damping is zeroed on every test body so the numbers below are the integrator's and not
/// the component default's (0.05, `Components.h:241`).
fn add_body(
    scene: &mut Scene,
    name: &str,
    translation: Vec3,
    kind: RigidBodyKind,
    shape: ColliderShape,
) -> Entity {
    let entity = scene.spawn(name);
    let mut built = scene.get_mut(entity).expect("spawned");
    built
        .get_mut::<Transform>()
        .expect("every entity has a transform")
        .translation = translation;
    built.insert(RigidBody {
        kind,
        mass: 1.0,
        velocity: Vec3::ZERO,
        damping: 0.0,
        gravity_scale: 1.0,
    });
    built.insert(Collider {
        shape,
        offset: Vec3::ZERO,
        friction: 0.5,
        restitution: 0.0,
    });
    entity
}

fn translation(scene: &Scene, entity: Entity) -> Vec3 {
    scene
        .get(entity)
        .expect("entity")
        .get::<Transform>()
        .expect("transform")
        .translation
}

fn rotation(scene: &Scene, entity: Entity) -> Vec3 {
    scene
        .get(entity)
        .expect("entity")
        .get::<Transform>()
        .expect("transform")
        .rotation
}

/// Every transform in the scene, as bit patterns: `assert_eq!` on floats would pass for
/// two runs that differ in the last bit, which is the whole property being claimed.
fn snapshot(scene: &Scene) -> Vec<[u32; 6]> {
    let mut out = Vec::new();
    for entity in scene.entities() {
        let transform = scene
            .get(entity)
            .expect("entity")
            .get::<Transform>()
            .expect("transform");
        out.push([
            transform.translation.x.to_bits(),
            transform.translation.y.to_bits(),
            transform.translation.z.to_bits(),
            transform.rotation.x.to_bits(),
            transform.rotation.y.to_bits(),
            transform.rotation.z.to_bits(),
        ]);
    }
    out
}

/// A box dropped onto a static plane, built fresh each call so two runs cannot share so
/// much as an allocator's state through the scene.
fn drop_scene() -> (Scene, Entity) {
    let mut scene = Scene::new();
    add_body(
        &mut scene,
        "ground",
        Vec3::new(0.0, -1.0, 0.0),
        RigidBodyKind::Static,
        box_shape(Vec3::new(50.0, 1.0, 50.0)),
    );
    let box_entity = add_body(
        &mut scene,
        "box",
        Vec3::new(0.0, 2.0, 0.0),
        RigidBodyKind::Dynamic,
        box_shape(Vec3::splat(0.5)),
    );
    (scene, box_entity)
}

#[test]
fn a_falling_box_reaches_the_analytic_position() {
    let mut scene = Scene::new();
    let entity = add_body(
        &mut scene,
        "box",
        Vec3::new(0.0, 10.0, 0.0),
        RigidBodyKind::Dynamic,
        box_shape(Vec3::splat(0.5)),
    );

    let mut physics = Physics3D::new();
    assert_eq!(physics.body_count(), 0);

    let steps = 60;
    for _ in 0..steps {
        physics.step(&mut scene, DT);
    }
    assert_eq!(physics.body_count(), 1);

    // y = y0 - g t^2 / 2, over one second. The gap to Rapier's semi-implicit Euler (which
    // accumulates g dt^2 n(n+1)/2 rather than the continuous integral) is ~0.08 m here, so
    // the tolerance separates the two rather than papering over a mistake.
    let seconds = DT * steps as f32;
    let analytic = 10.0 - 0.5 * GRAVITY.y.abs() * seconds * seconds;
    let y = translation(&scene, entity).y;
    assert!(
        (y - analytic).abs() < 0.2,
        "fell to y = {y}, analytic {analytic}"
    );
}

#[test]
fn a_box_on_the_ground_comes_to_rest_and_stays() {
    let (mut scene, entity) = drop_scene();
    let mut physics = Physics3D::new();

    // Five seconds is well past the drop, the restitution-free impact, and Rapier's half
    // second of stillness before a body sleeps.
    for _ in 0..300 {
        physics.step(&mut scene, DT);
    }
    let rest = translation(&scene, entity);
    // The plane's top is y = 0 and the box's half-extent is 0.5.
    assert!((rest.y - 0.5).abs() < 0.05, "resting at {rest:?}");

    for _ in 0..100 {
        physics.step(&mut scene, DT);
    }
    let after = translation(&scene, entity);
    assert!(
        (after - rest).length() < 1.0e-3,
        "rested at {rest:?}, drifted to {after:?}"
    );
}

#[test]
fn a_sphere_rests_at_its_radius() {
    let mut scene = Scene::new();
    add_body(
        &mut scene,
        "ground",
        Vec3::new(0.0, -1.0, 0.0),
        RigidBodyKind::Static,
        box_shape(Vec3::new(50.0, 1.0, 50.0)),
    );
    let ball = add_body(
        &mut scene,
        "ball",
        Vec3::new(0.0, 2.0, 0.0),
        RigidBodyKind::Dynamic,
        ColliderShape::Sphere { radius: 0.25 },
    );

    let mut physics = Physics3D::new();
    for _ in 0..300 {
        physics.step(&mut scene, DT);
    }

    let rest = translation(&scene, ball);
    assert!((rest.y - 0.25).abs() < 0.05, "resting at {rest:?}");
}

#[test]
fn two_identical_runs_agree_bit_for_bit() {
    let (mut left, _) = drop_scene();
    let (mut right, _) = drop_scene();
    let mut left_physics = Physics3D::new();
    let mut right_physics = Physics3D::new();

    for _ in 0..240 {
        left_physics.step(&mut left, DT);
        right_physics.step(&mut right, DT);
    }

    assert_eq!(snapshot(&left), snapshot(&right));
}

#[test]
fn a_static_body_does_not_move() {
    let mut scene = Scene::new();
    add_body(
        &mut scene,
        "ground",
        Vec3::new(1.0, -1.0, 2.0),
        RigidBodyKind::Static,
        box_shape(Vec3::new(50.0, 1.0, 50.0)),
    );
    let before = snapshot(&scene);

    let mut physics = Physics3D::new();
    for _ in 0..100 {
        physics.step(&mut scene, DT);
    }

    assert_eq!(before, snapshot(&scene));
    assert_eq!(physics.body_count(), 1);
}

#[test]
fn an_entity_without_a_rigid_body_is_untouched() {
    let mut scene = Scene::new();
    let entity = scene.spawn("props");
    scene
        .get_mut(entity)
        .expect("spawned")
        .get_mut::<Transform>()
        .expect("transform")
        .translation = Vec3::new(3.0, 4.0, 5.0);
    let before = snapshot(&scene);

    let mut physics = Physics3D::new();
    for _ in 0..10 {
        physics.step(&mut scene, DT);
    }

    assert_eq!(before, snapshot(&scene));
    assert_eq!(physics.body_count(), 0);
    assert!(physics.handle(entity).is_none());
}

#[test]
fn a_rigid_body_without_a_collider_creates_no_body() {
    let mut scene = Scene::new();
    let entity = scene.spawn("crate");
    scene.get_mut(entity).expect("spawned").insert(RigidBody {
        kind: RigidBodyKind::Dynamic,
        mass: 1.0,
        velocity: Vec3::ZERO,
        damping: 0.05,
        gravity_scale: 1.0,
    });
    let before = snapshot(&scene);
    let mut physics = Physics3D::new();
    for _ in 0..10 {
        physics.step(&mut scene, DT);
    }

    // The C++ created a body only inside `if (shape)` (`Scene.cpp:1412`), so a rigid body
    // with no collider component fell nowhere.
    assert_eq!(physics.body_count(), 0);
    assert_eq!(before, snapshot(&scene));
}

#[test]
fn an_unchanged_body_keeps_its_handle() {
    let (mut scene, entity) = drop_scene();
    let mut physics = Physics3D::new();

    physics.step(&mut scene, DT);
    let first = physics.handle(entity).expect("the box has a body");
    for _ in 0..60 {
        physics.step(&mut scene, DT);
    }

    assert_eq!(physics.handle(entity), Some(first));
    assert_eq!(physics.body_count(), 2);
}

/// The rotation the scene authored has to survive the trip into Rapier and back.
///
/// `Transform::rotation_quat` is the C++ `glm::quat(vec3)` — `Rz*Ry*Rx`, glam's `XYZEx`
/// and not the `Rx*Ry*Rz` of its `XYZ` — so `to_euler(EulerRot::XYZ)`, the inverse anyone
/// would reach for, belongs to a different order and would hand the scene angles that
/// mean another orientation.
#[test]
fn an_authored_rotation_survives_the_round_trip() {
    let mut scene = Scene::new();
    let entity = add_body(
        &mut scene,
        "box",
        Vec3::new(0.0, 5.0, 0.0),
        RigidBodyKind::Dynamic,
        box_shape(Vec3::splat(0.5)),
    );
    let authored = Vec3::new(30.0, 20.0, 10.0);
    scene
        .get_mut(entity)
        .expect("entity")
        .get_mut::<Transform>()
        .expect("transform")
        .rotation = authored;

    let mut physics = Physics3D::new();
    // One step of free fall: nothing moves the rotation, so whatever comes back is the
    // conversion's error and nothing else.
    physics.step(&mut scene, DT);

    let after = rotation(&scene, entity);
    assert!(
        (after - authored).length() < 0.1,
        "rotation {authored:?} came back as {after:?}"
    );
}

#[test]
fn a_body_the_user_moved_is_pushed_into_rapier() {
    let (mut scene, entity) = drop_scene();
    let mut physics = Physics3D::new();

    for _ in 0..300 {
        physics.step(&mut scene, DT);
    }
    let rested = translation(&scene, entity);

    // The editor drags the body up two metres; the next step must start from there instead
    // of overwriting the edit with where the simulation had left it.
    let lifted = Vec3::new(rested.x, rested.y + 2.0, rested.z);
    scene
        .get_mut(entity)
        .expect("entity")
        .get_mut::<Transform>()
        .expect("transform")
        .translation = lifted;

    physics.step(&mut scene, DT);

    let after = translation(&scene, entity);
    assert!(
        after.y > lifted.y - 0.05,
        "moved to {after:?}, the edit was at {lifted:?}"
    );
}

#[test]
fn a_moving_body_is_written_back_to_the_scene() {
    let (mut scene, entity) = drop_scene();
    let start = translation(&scene, entity);
    let mut physics = Physics3D::new();

    physics.step(&mut scene, DT);

    let moved = translation(&scene, entity);
    assert!(
        moved.y < start.y,
        "the box should have fallen from {start:?}, scene says {moved:?}"
    );
}
