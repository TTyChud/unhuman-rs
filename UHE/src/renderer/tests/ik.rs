//! Inverse kinematics: reach, clamping and degenerate chains. Pure math only.

use bifrost::ik::{solve_fabrik, solve_two_bone, DEFAULT_MAX_ITERATIONS, DEFAULT_TOLERANCE};
use glam::Vec3;

fn three_joint_chain() -> [Vec3; 3] {
    [
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 2.0, 0.0),
    ]
}

#[test]
fn a_reachable_target_is_reached_within_the_iteration_cap() {
    let mut joints = three_joint_chain();
    let target = Vec3::new(1.0, 1.0, 0.0);

    let result = solve_fabrik(
        &mut joints,
        target,
        None,
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_TOLERANCE,
    );

    assert!(result.solved);
    assert!(result.iterations <= DEFAULT_MAX_ITERATIONS);
    assert!((joints[2] - target).length() <= DEFAULT_TOLERANCE);
    for joint in joints {
        assert!(!joint.is_nan());
    }
}

#[test]
fn the_root_stays_put_and_the_bone_lengths_survive() {
    let mut joints = three_joint_chain();
    let root = joints[0];
    let target = Vec3::new(1.0, 1.0, 0.0);

    solve_fabrik(
        &mut joints,
        target,
        None,
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_TOLERANCE,
    );

    assert_eq!(joints[0], root);
    assert!(((joints[1] - joints[0]).length() - 1.0).abs() < 1e-3);
    assert!(((joints[2] - joints[1]).length() - 1.0).abs() < 1e-3);
}

#[test]
fn a_pole_vector_moves_the_hinge_into_its_plane() {
    let mut joints = three_joint_chain();
    let target = Vec3::new(1.0, 1.0, 0.0);

    let result = solve_fabrik(
        &mut joints,
        target,
        Some(Vec3::new(0.0, 0.0, 5.0)),
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_TOLERANCE,
    );

    assert!(result.solved);
    // The hinge plane now contains +Z, and the effector is still on the target.
    assert!(joints[1].z > 0.0);
    assert!((joints[2] - target).length() <= DEFAULT_TOLERANCE);
}

#[test]
fn an_unreachable_target_ends_pointing_at_it_without_nan() {
    let mut joints = [
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(2.0, 0.0, 0.0),
    ];
    let target = Vec3::new(10.0, 0.0, 0.0);

    let result = solve_fabrik(
        &mut joints,
        target,
        None,
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_TOLERANCE,
    );

    assert!(!result.solved);
    let direction = (joints[2] - joints[0]).normalize();
    assert!(direction.dot(Vec3::X) > 0.999);
    assert!((joints[2] - joints[0]).length() <= 2.0 + 1e-4);
    for joint in joints {
        assert!(!joint.is_nan());
    }
}

#[test]
fn a_degenerate_bone_chain_does_not_produce_nan() {
    // Every joint on the same point: zero total length.
    let mut joints = [Vec3::ZERO, Vec3::ZERO, Vec3::ZERO];
    let result = solve_fabrik(
        &mut joints,
        Vec3::new(1.0, 0.0, 0.0),
        Some(Vec3::Z),
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_TOLERANCE,
    );
    assert!(!result.solved);
    for joint in joints {
        assert!(!joint.is_nan());
    }

    // A single zero-length bone inside an otherwise sane chain.
    let mut joints = [Vec3::ZERO, Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)];
    solve_fabrik(
        &mut joints,
        Vec3::new(0.5, 0.5, 0.0),
        None,
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_TOLERANCE,
    );
    for joint in joints {
        assert!(!joint.is_nan());
    }
}

#[test]
fn the_two_bone_solve_places_the_effector_on_a_reachable_target() {
    let root = Vec3::ZERO;
    let mid = Vec3::new(1.0, 0.0, 0.0);
    let end = Vec3::new(2.0, 0.0, 0.0);
    let target = Vec3::new(1.0, 1.0, 0.0);

    let result = solve_two_bone(root, mid, end, target, Vec3::Z);
    assert!(result.solved);

    // Apply the two joint corrections the way a skeleton would: the root rotation
    // steers the upper bone, the mid rotation is relative to it.
    let upper = result.root_rotation * (mid - root).normalize();
    let new_mid = root + upper * 1.0;
    let lower = (result.root_rotation * result.mid_rotation) * (end - mid).normalize();
    let new_end = new_mid + lower * 1.0;

    assert!(!new_end.is_nan());
    assert!((new_end - target).length() < 1e-3, "end {new_end:?}");
}

#[test]
fn the_two_bone_solve_clamps_a_target_out_of_reach() {
    let root = Vec3::ZERO;
    let mid = Vec3::new(1.0, 0.0, 0.0);
    let end = Vec3::new(2.0, 0.0, 0.0);
    let target = Vec3::new(10.0, 0.0, 0.0);

    let result = solve_two_bone(root, mid, end, target, Vec3::Z);
    assert!(result.solved);

    let upper = result.root_rotation * (mid - root).normalize();
    let new_mid = root + upper * 1.0;
    let lower = (result.root_rotation * result.mid_rotation) * (end - mid).normalize();
    let new_end = new_mid + lower * 1.0;

    // Stretched towards the target, stopping just short of full extension.
    assert!(new_end.dot(Vec3::X) > 1.99);
    assert!((new_end - root).length() <= 2.0);
    assert!(!new_end.is_nan());
}

#[test]
fn a_degenerate_two_bone_solve_reports_unsolved_without_nan() {
    let result = solve_two_bone(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::X, Vec3::Y);
    assert!(!result.solved);
    assert!(!result.root_rotation.is_nan());
    assert!(!result.mid_rotation.is_nan());

    // A target sitting on the root joint has no direction to solve towards.
    let result = solve_two_bone(
        Vec3::ZERO,
        Vec3::X,
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::ZERO,
        Vec3::Y,
    );
    assert!(!result.solved);
}

#[test]
fn a_chain_shorter_than_one_bone_is_left_alone() {
    let mut joints = [Vec3::ZERO];
    let result = solve_fabrik(
        &mut joints,
        Vec3::X,
        None,
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_TOLERANCE,
    );
    assert!(!result.solved);
    assert_eq!(result.iterations, 0);
    assert_eq!(joints[0], Vec3::ZERO);
}
