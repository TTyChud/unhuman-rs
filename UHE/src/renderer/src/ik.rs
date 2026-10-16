//! Inverse kinematics.
//!
//! `solve_two_bone` is the port of `Renderer3D/IKSolver.cpp`: analytic, returns the
//! two joint rotations that place a two-bone chain's end effector on a target.
//! `solve_fabrik` is the position-space counterpart for chains of any length, with the
//! iteration cap and tolerance the C++ solver had no place for. Both are pure: joint
//! positions in, positions (or rotations) out.

use glam::{Quat, Vec3};

/// Output rotations of the analytic two-bone solve. Port of `TwoBoneIKResult`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TwoBoneIkResult {
    pub solved: bool,
    /// Global rotation adjustment for the root bone.
    pub root_rotation: Quat,
    /// Global rotation adjustment for the middle bone (the hinge).
    pub mid_rotation: Quat,
}

/// Iteration cap `solve_fabrik` uses when the caller has no opinion.
pub const DEFAULT_MAX_ITERATIONS: usize = 16;
/// Distance the end effector must reach, in linear units.
pub const DEFAULT_TOLERANCE: f32 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FabrikResult {
    /// True when the end effector reached the target within tolerance.
    pub solved: bool,
    /// Backward/forward passes performed.
    pub iterations: usize,
}

/// Analytic two-bone solve. Port of `IKSolver::SolveTwoBone`.
///
/// Out-of-reach targets are clamped to the reachable annulus `|l1 - l2| < d < l1 + l2`,
/// so the hinge never degenerates; a zero-length bone or a target on the root joint
/// returns `solved: false` and identity rotations.
pub fn solve_two_bone(
    root: Vec3,
    mid: Vec3,
    end: Vec3,
    target: Vec3,
    pole: Vec3,
) -> TwoBoneIkResult {
    let mut result = TwoBoneIkResult::default();

    let root_to_mid = mid - root;
    let mid_to_end = end - mid;
    let upper_length = root_to_mid.length();
    let lower_length = mid_to_end.length();
    if upper_length < f32::EPSILON || lower_length < f32::EPSILON {
        return result;
    }

    let root_to_end = end - root;
    let root_to_target = target - root;
    let target_distance = root_to_target.length();
    if target_distance < f32::EPSILON {
        return result;
    }

    // Keep the target inside the reachable annulus `|l1 - l2| < d < l1 + l2`, in
    // `glm::clamp`'s `max(lo)` then `min(hi)` order: a very short lower bone can push
    // the two bounds past each other, and `f32::clamp` would panic on that.
    let max_reach = (upper_length + lower_length) * (1.0 - 1e-4);
    let min_reach = (upper_length - lower_length).abs() * (1.0 + 1e-4) + f32::EPSILON;
    let mut reach = target_distance;
    if reach < min_reach {
        reach = min_reach;
    }
    if reach > max_reach {
        reach = max_reach;
    }
    let target_dir = root_to_target / target_distance;

    // Interior angle of the root-mid-end triangle at the root (law of cosines).
    let root_angle = ((upper_length * upper_length + reach * reach - lower_length * lower_length)
        / (2.0 * upper_length * reach))
        .clamp(-1.0, 1.0)
        .acos();

    // Bend plane: contains the chain and the pole. Fall back to the current bend
    // direction when the pole is collinear with the chain.
    let pole_dir = pole - root;
    let mut bend_normal = target_dir.cross(pole_dir);
    if bend_normal.length() < 1e-6 {
        let current_bend = root_to_end.cross(root_to_mid);
        bend_normal = if current_bend.length() > 1e-6 {
            current_bend
        } else {
            Vec3::Z
        };
        if bend_normal.normalize().dot(target_dir).abs() > 0.999 {
            bend_normal = Vec3::Y;
        }
        if bend_normal.normalize().dot(target_dir).abs() > 0.999 {
            bend_normal = Vec3::X;
        }
    }
    let bend_normal = bend_normal.normalize();
    // Direction in the bend plane, perpendicular to the target direction.
    let bend_dir = bend_normal.cross(target_dir).normalize_or_zero();

    // New joint placement: the mid joint sits off the root->target axis by root_angle.
    let new_mid =
        root + upper_length * (root_angle.cos() * target_dir + root_angle.sin() * bend_dir);
    let new_end = root + reach * target_dir;

    // World-space rotations taking the current bone directions to the solved ones.
    let mid_rotation = Quat::from_rotation_arc(
        mid_to_end.normalize_or_zero(),
        (new_end - new_mid).normalize_or_zero(),
    );

    let new_root_to_mid = (new_mid - root).normalize_or_zero();
    let current_root_to_mid = root_to_mid.normalize_or_zero();
    result.root_rotation = if current_root_to_mid.dot(new_root_to_mid) > -0.9999 {
        Quat::from_rotation_arc(current_root_to_mid, new_root_to_mid)
    } else {
        Quat::from_axis_angle(bend_normal, std::f32::consts::PI)
    };

    // The root rotation only steers the upper bone; the mid rotation must be
    // expressed relative to the rotated upper bone.
    result.mid_rotation = result.root_rotation.inverse() * mid_rotation;
    result.solved = true;
    result
}

/// Position-space solve for a chain of any length. `joints` are world positions in
/// root-to-tip order and are rewritten in place; the root is pinned. `pole`, when
/// given, is a point the hinge plane is rotated towards after convergence.
///
/// An unreachable target leaves the chain stretched straight at the target and returns
/// `solved: false`. A degenerate chain (zero total length) is left alone.
pub fn solve_fabrik(
    joints: &mut [Vec3],
    target: Vec3,
    pole: Option<Vec3>,
    max_iterations: usize,
    tolerance: f32,
) -> FabrikResult {
    let unsolved = FabrikResult {
        solved: false,
        iterations: 0,
    };
    if joints.len() < 2 {
        return unsolved;
    }

    let root = joints[0];
    let last = joints.len() - 1;
    let lengths: Vec<f32> = joints
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).length())
        .collect();
    let total: f32 = lengths.iter().sum();
    if total <= tolerance || !total.is_finite() {
        return unsolved;
    }

    // Out of reach: every bone points at the target and the effector stops short.
    if (target - root).length() > total {
        let direction = (target - root).normalize_or_zero();
        let mut position = root;
        for (i, length) in lengths.iter().enumerate() {
            position += direction * *length;
            joints[i + 1] = position;
        }
        if let Some(pole) = pole {
            apply_pole(joints, pole);
        }
        return unsolved;
    }

    let mut iterations = 0;
    let mut solved = (joints[last] - target).length() <= tolerance;
    while !solved && iterations < max_iterations {
        iterations += 1;

        // Backward: pin the end effector to the target and walk to the root.
        joints[last] = target;
        for (i, length) in lengths.iter().enumerate().rev() {
            if *length <= 0.0 {
                continue;
            }
            let direction = (joints[i] - joints[i + 1]).normalize_or_zero();
            if direction != Vec3::ZERO {
                joints[i] = joints[i + 1] + direction * *length;
            }
        }

        // Forward: pin the root back and walk to the tip.
        joints[0] = root;
        for (i, length) in lengths.iter().enumerate() {
            if *length <= 0.0 {
                continue;
            }
            let direction = (joints[i + 1] - joints[i]).normalize_or_zero();
            if direction != Vec3::ZERO {
                joints[i + 1] = joints[i] + direction * *length;
            }
        }

        solved = (joints[last] - target).length() <= tolerance;
    }

    if let Some(pole) = pole {
        apply_pole(joints, pole);
    }

    FabrikResult { solved, iterations }
}

/// Rotates every interior joint rigidly about the root->tip axis, so the hinge plane
/// contains the pole. Root, tip and every bone length are left untouched.
fn apply_pole(joints: &mut [Vec3], pole: Vec3) {
    if joints.len() < 3 {
        return;
    }
    let root = joints[0];
    let last = joints.len() - 1;
    let axis = (joints[last] - root).normalize_or_zero();
    if axis == Vec3::ZERO {
        return;
    }

    // Measure the rotation on the interior joint furthest off the root->tip axis; the
    // whole interior then rotates by the same angle, which keeps the chain rigid.
    let mut pivot = Vec3::ZERO;
    let mut best = 0.0;
    for joint in &joints[1..last] {
        let offset = *joint - root;
        let perpendicular = offset - axis * offset.dot(axis);
        let length = perpendicular.length();
        if length > best {
            best = length;
            pivot = perpendicular;
        }
    }
    let pole_offset = pole - root;
    let pole_perpendicular = pole_offset - axis * pole_offset.dot(axis);
    if best <= 1e-6 || pole_perpendicular.length() <= 1e-6 {
        return;
    }

    let angle = pivot.angle_between(pole_perpendicular);
    let signed = if pivot.cross(pole_perpendicular).dot(axis) < 0.0 {
        -angle
    } else {
        angle
    };
    let rotation = Quat::from_axis_angle(axis, signed);

    for joint in &mut joints[1..last] {
        let offset = *joint - root;
        let along = axis * offset.dot(axis);
        *joint = root + along + rotation * (offset - along);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_two_bone_chain_reports_unsolved() {
        let result = solve_two_bone(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::X, Vec3::Y);
        assert!(!result.solved);
        assert!(!result.root_rotation.is_nan());
        assert!(!result.mid_rotation.is_nan());
    }

    #[test]
    fn a_degenerate_fabrik_chain_stays_finite() {
        let mut joints = [Vec3::ZERO, Vec3::ZERO, Vec3::ZERO];
        let result = solve_fabrik(
            &mut joints,
            Vec3::X,
            None,
            DEFAULT_MAX_ITERATIONS,
            DEFAULT_TOLERANCE,
        );
        assert!(!result.solved);
        for joint in joints {
            assert!(!joint.is_nan());
        }
    }

    #[test]
    fn an_unreachable_target_is_left_short_without_nan() {
        let mut joints = [Vec3::ZERO, Vec3::Y, Vec3::new(0.0, 2.0, 0.0)];
        let target = Vec3::new(100.0, 0.0, 0.0);
        let result = solve_fabrik(&mut joints, target, None, 4, 1e-3);
        assert!(!result.solved);
        let direction = (joints[2] - joints[0]).normalize();
        assert!(direction.dot(Vec3::X).abs() > 0.999);
        for joint in joints {
            assert!(!joint.is_nan());
        }
    }
}
