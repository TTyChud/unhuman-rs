use femur::bodies::{BodyDesc, BodyId, BodyKind};
use femur::colliders::ColliderDesc;
use femur::colliders::Shape;
use femur::convert::{self, Pose};
use femur::timestep::FixedStep;
use femur::PhysicsWorld;
use glam::{Quat, Vec2, Vec3};

#[test]
fn fixed_step_huge_dt_no_spiral() {
    let mut clock = FixedStep::default_rate();
    let first = clock.plan(10.0);
    assert!(first.clamped);
    assert_eq!(first.steps, femur::timestep::DEFAULT_MAX_SUBSTEPS);
    // accumulator cleared: next normal frame does not owe a spiral
    let second = clock.plan(1.0 / 60.0);
    assert!(!second.clamped);
    assert!(second.steps <= 1);
    assert_eq!(clock.leftover(), 0.0);
}

#[test]
fn substep_clamp_stats() {
    let mut clock = FixedStep::default_rate();
    let plan = clock.plan(1.0);
    assert!(plan.clamped);
    assert_eq!(plan.steps, 8);
    assert!(plan.dropped > 0.0);
}

#[test]
fn reuse_no_leak() {
    let mut world = PhysicsWorld::new(Vec3::ZERO);
    for i in 0..10u64 {
        let id = BodyId::new(i);
        let coll = ColliderDesc::new(
            Shape::Box {
                half_extents: Vec3::splat(0.5),
            },
            Vec3::ZERO,
        );
        world
            .spawn(id, &BodyDesc::default(), &[coll], Pose::IDENTITY, Vec3::ONE)
            .unwrap();
        assert!(world.despawn(id));
    }
    assert_eq!(world.body_count(), 0);
    assert_eq!(world.collider_count(), 0);
}

#[test]
fn conversion_roundtrip_exact() {
    let v = Vec3::new(1.25, -2.5, 3.75);
    let r = convert::to_vec3(v);
    assert_eq!(convert::from_vec3(r), v);

    let v2 = Vec2::new(0.5, -1.25);
    let r2 = convert::to_vec2(v2);
    assert_eq!(convert::from_vec2(r2), v2);

    let q = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9).normalize();
    let rq = convert::to_quat(q);
    assert_eq!(convert::from_quat(rq), q);

    // 2D angle is intentionally not bit exact (atan2) but within tolerance
    let a = 1.234_f32;
    let r = convert::to_angle(a);
    let back = convert::from_angle(r);
    assert!((a - back).abs() < 1e-6);
}

#[test]
fn two_box_collision_resolves() {
    let mut world = PhysicsWorld::new(Vec3::new(0.0, -9.81, 0.0));
    let ground = BodyDesc {
        kind: BodyKind::Static,
        ..BodyDesc::default()
    };
    let coll = ColliderDesc::new(
        Shape::Box {
            half_extents: Vec3::new(5.0, 0.5, 5.0),
        },
        Vec3::ZERO,
    );
    world
        .spawn(
            BodyId::new(1),
            &ground,
            &[coll],
            Pose::new(Vec3::new(0.0, -1.0, 0.0), Quat::IDENTITY),
            Vec3::ONE,
        )
        .unwrap();

    let ball_coll = ColliderDesc::new(
        Shape::Box {
            half_extents: Vec3::splat(0.5),
        },
        Vec3::ZERO,
    );
    world
        .spawn(
            BodyId::new(2),
            &BodyDesc::default(),
            &[ball_coll],
            Pose::new(Vec3::new(0.0, 5.0, 0.0), Quat::IDENTITY),
            Vec3::ONE,
        )
        .unwrap();

    for _ in 0..120 {
        world.step(1.0 / 60.0);
    }
    let state = world.pose(BodyId::new(2)).unwrap();
    assert!(state.translation.y >= -1.0e-3);
}
