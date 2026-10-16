//! Track sampling, pose blending and bone-matrix evaluation.
//!
//! Port of `Animator.cpp`'s sampling half (`SamplePosition` / `SampleRotation` /
//! `SampleScale` / `Hermite` / `SampleClip` / `BlendPoses`) plus the hierarchy walk
//! (`BuildChildLists` / `CalculateBoneTransform` / `ComputeFinalMatrices`), kept
//! free of state so every function is a pure function of its inputs.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::animation::clip::{
    Bone, BonePose, Clip, Interpolation, Keyframe, Pose, QuaternionTrack, Skeleton, Skin,
    VectorTrack,
};

/// Cubic Hermite basis on `[a, b]`, with `a`'s outgoing tangent and `b`'s incoming
/// tangent. Layout follows `Animator::Hermite`.
pub fn hermite_vec3(
    a: &Keyframe<Vec3>,
    b: &Keyframe<Vec3>,
    in_tangent: Vec3,
    out_tangent: Vec3,
    time: f32,
) -> Vec3 {
    let span = b.time - a.time;
    let t = if span > 0.0 {
        (time - a.time) / span
    } else {
        0.0
    };
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * a.value + h10 * span * out_tangent + h01 * b.value + h11 * span * in_tangent
}

/// Component-wise Hermite, renormalised to stay a unit rotation.
pub fn hermite_quat(
    a: &Keyframe<Quat>,
    b: &Keyframe<Quat>,
    in_tangent: Quat,
    out_tangent: Quat,
    time: f32,
) -> Quat {
    let span = b.time - a.time;
    let t = if span > 0.0 {
        (time - a.time) / span
    } else {
        0.0
    };
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    (a.value * h00 + out_tangent * (h10 * span) + b.value * h01 + in_tangent * (h11 * span))
        .normalize()
}

pub fn sample_position(time: f32, track: &VectorTrack) -> Vec3 {
    let keys = &track.keyframes;
    if keys.is_empty() {
        return Vec3::ZERO;
    }
    if keys.len() == 1 {
        return keys[0].value;
    }
    // First key strictly after `time`, i.e. `std::upper_bound`.
    let i = keys.partition_point(|k| k.time <= time);
    if i == 0 {
        return keys[0].value;
    }
    if i >= keys.len() {
        return keys[keys.len() - 1].value;
    }

    if track.interpolation == Interpolation::Step {
        return keys[i - 1].value;
    }
    if track.interpolation == Interpolation::CubicSpline
        && track.out_tangents.len() == keys.len()
        && track.in_tangents.len() == keys.len()
    {
        return hermite_vec3(
            &keys[i - 1],
            &keys[i],
            track.in_tangents[i],
            track.out_tangents[i - 1],
            time,
        );
    }

    let span = keys[i].time - keys[i - 1].time;
    let u = if span > 0.0 {
        (time - keys[i - 1].time) / span
    } else {
        0.0
    };
    keys[i - 1].value.lerp(keys[i].value, u)
}

pub fn sample_rotation(time: f32, track: &QuaternionTrack) -> Quat {
    let keys = &track.keyframes;
    if keys.is_empty() {
        return Quat::IDENTITY;
    }
    if keys.len() == 1 {
        return keys[0].value;
    }
    let i = keys.partition_point(|k| k.time <= time);
    if i == 0 {
        return keys[0].value;
    }
    if i >= keys.len() {
        return keys[keys.len() - 1].value;
    }

    if track.interpolation == Interpolation::Step {
        return keys[i - 1].value;
    }
    if track.interpolation == Interpolation::CubicSpline
        && track.out_tangents.len() == keys.len()
        && track.in_tangents.len() == keys.len()
    {
        return hermite_quat(
            &keys[i - 1],
            &keys[i],
            track.in_tangents[i],
            track.out_tangents[i - 1],
            time,
        );
    }

    let span = keys[i].time - keys[i - 1].time;
    let u = if span > 0.0 {
        (time - keys[i - 1].time) / span
    } else {
        0.0
    };
    keys[i - 1].value.slerp(keys[i].value, u)
}

pub fn sample_scale(time: f32, track: &VectorTrack) -> Vec3 {
    let keys = &track.keyframes;
    if keys.is_empty() {
        return Vec3::ONE;
    }
    if keys.len() == 1 {
        return keys[0].value;
    }
    let i = keys.partition_point(|k| k.time <= time);
    if i == 0 {
        return keys[0].value;
    }
    if i >= keys.len() {
        return keys[keys.len() - 1].value;
    }

    if track.interpolation == Interpolation::Step {
        return keys[i - 1].value;
    }
    if track.interpolation == Interpolation::CubicSpline
        && track.out_tangents.len() == keys.len()
        && track.in_tangents.len() == keys.len()
    {
        return hermite_vec3(
            &keys[i - 1],
            &keys[i],
            track.in_tangents[i],
            track.out_tangents[i - 1],
            time,
        );
    }

    let span = keys[i].time - keys[i - 1].time;
    let u = if span > 0.0 {
        (time - keys[i - 1].time) / span
    } else {
        0.0
    };
    keys[i - 1].value.lerp(keys[i].value, u)
}

/// Samples every track of `clip` at `time` into `pose`. Bones no track targets keep
/// the values they already hold, which is how the caller applies a rest pose first.
pub fn sample_clip(clip: &Clip, time: f32, pose: &mut Pose) {
    for track in &clip.position_tracks {
        if track.target_bone < 0 {
            continue;
        }
        if let Some(bone) = pose.bones.get_mut(track.target_bone as usize) {
            bone.translation = sample_position(time, track);
            bone.animated = true;
        }
    }
    for track in &clip.rotation_tracks {
        if track.target_bone < 0 {
            continue;
        }
        if let Some(bone) = pose.bones.get_mut(track.target_bone as usize) {
            bone.rotation = sample_rotation(time, track);
            bone.animated = true;
        }
    }
    for track in &clip.scale_tracks {
        if track.target_bone < 0 {
            continue;
        }
        if let Some(bone) = pose.bones.get_mut(track.target_bone as usize) {
            bone.scale = sample_scale(time, track);
            bone.animated = true;
        }
    }
}

/// Convenience wrapper: a fresh pose of `bone_count` bones sampled from `clip`.
pub fn sample_pose(clip: &Clip, time: f32, bone_count: usize) -> Pose {
    let mut pose = Pose::new(bone_count);
    sample_clip(clip, time, &mut pose);
    pose
}

/// Blend of two poses over the shorter of the two. Weight 0 and 1 are copies, which
/// is the fast path for a single clip played at full weight.
pub fn blend_poses(from: &Pose, to: &Pose, weight: f32, out: &mut Pose) {
    let count = from.bones.len().min(to.bones.len());
    if weight <= 0.0 {
        out.bones.clear();
        out.bones.extend_from_slice(&from.bones[..count]);
        return;
    }
    if weight >= 1.0 {
        out.bones.clear();
        out.bones.extend_from_slice(&to.bones[..count]);
        return;
    }
    out.resize(count);
    for i in 0..count {
        let a = from.bones[i];
        let b = to.bones[i];
        out.bones[i] = BonePose {
            translation: a.translation.lerp(b.translation, weight),
            rotation: a.rotation.slerp(b.rotation, weight),
            scale: a.scale.lerp(b.scale, weight),
            animated: a.animated || b.animated,
        };
    }
}

/// Per bone, the bones that name it as parent. Port of `Animator::BuildChildLists`.
pub fn build_child_lists(skeleton: &Skeleton) -> Vec<Vec<i32>> {
    let mut children: Vec<Vec<i32>> = vec![Vec::new(); skeleton.bones.len()];
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let parent = bone.parent_id;
        if parent >= 0 && (parent as usize) < children.len() {
            children[parent as usize].push(i as i32);
        }
    }
    children
}

/// The rest pose: each bone's local transform decomposed into translation, rotation
/// and scale. Matrices glam cannot decompose (a zero scale, for instance) leave the
/// bone at identity, the same guard `glm::decompose` provided.
pub fn rest_pose(skeleton: &Skeleton) -> Pose {
    let mut pose = Pose::new(skeleton.bones.len());
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let (scale, rotation, translation) = bone.local_transform.to_scale_rotation_translation();
        if !scale.is_nan() && !rotation.is_nan() && !translation.is_nan() {
            pose.bones[i].translation = translation;
            pose.bones[i].rotation = rotation;
            pose.bones[i].scale = scale;
            pose.bones[i].animated = false;
        }
    }
    pose
}

fn node_transform(bone: &Bone, pose: &Pose, bone_id: i32) -> Mat4 {
    let posed = pose
        .bones
        .get(bone_id as usize)
        .copied()
        .filter(|p| p.animated);
    match posed {
        Some(p) => Mat4::from_scale_rotation_translation(p.scale, p.rotation, p.translation),
        None => bone.local_transform,
    }
}

pub(crate) fn calculate_bone_transform(
    skeleton: &Skeleton,
    children: &[Vec<i32>],
    pose: &Pose,
    root_motion: bool,
    bone_id: i32,
    parent_transform: Mat4,
    globals: &mut [Mat4],
) {
    if bone_id < 0 {
        return;
    }
    let bone_id_usize = bone_id as usize;
    let Some(bone) = skeleton.bones.get(bone_id_usize) else {
        return;
    };
    if bone_id_usize >= globals.len() {
        return;
    }

    let node = node_transform(bone, pose, bone_id);
    let mut global = parent_transform * node;

    // Root motion: the root joint stays at the origin and the extracted delta is
    // applied to the entity by gameplay code instead.
    if root_motion && bone_id == skeleton.root_bone_id {
        global.w_axis = Vec4::new(0.0, 0.0, 0.0, 1.0);
    }

    globals[bone_id_usize] = global;

    if let Some(kids) = children.get(bone_id_usize) {
        for &child in kids {
            calculate_bone_transform(
                skeleton,
                children,
                pose,
                root_motion,
                child,
                global,
                globals,
            );
        }
    }
}

/// World transforms for every bone, from each root down. Port of
/// `Animator::TraverseSkeletonRoots`.
pub fn global_transforms(
    skeleton: &Skeleton,
    children: &[Vec<i32>],
    pose: &Pose,
    root_motion: bool,
    globals: &mut Vec<Mat4>,
) {
    globals.clear();
    globals.resize(skeleton.bones.len(), Mat4::IDENTITY);
    for (i, bone) in skeleton.bones.iter().enumerate() {
        if bone.parent_id < 0 {
            calculate_bone_transform(
                skeleton,
                children,
                pose,
                root_motion,
                i as i32,
                Mat4::IDENTITY,
                globals,
            );
        }
    }
}

/// Port of `Animator::ComputeFinalMatrices`: world transforms times the selected
/// skin's inverse binds, falling back to the skeleton's own joint list.
pub fn skin_matrices(
    skeleton: &Skeleton,
    globals: &[Mat4],
    skin: Option<&Skin>,
    out: &mut Vec<Mat4>,
) {
    if let Some(skin) = skin {
        if !skin.joint_nodes.is_empty()
            && skin.inverse_bind_matrices.len() == skin.joint_nodes.len()
        {
            out.clear();
            out.reserve(skin.joint_nodes.len());
            for (&bone, &inverse_bind) in skin.joint_nodes.iter().zip(&skin.inverse_bind_matrices) {
                let global = globals
                    .get(bone as usize)
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                out.push(global * inverse_bind);
            }
            return;
        }
    }

    out.clear();
    out.reserve(skeleton.joint_nodes.len());
    for &bone in &skeleton.joint_nodes {
        let global = globals
            .get(bone as usize)
            .copied()
            .unwrap_or(Mat4::IDENTITY);
        let inverse_bind = skeleton
            .bones
            .get(bone as usize)
            .map_or(Mat4::IDENTITY, |b| b.inverse_bind_matrix);
        out.push(global * inverse_bind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector_track(interpolation: Interpolation) -> VectorTrack {
        let mut track = VectorTrack::new(0);
        track.interpolation = interpolation;
        track.set_keys(&[0.0, 1.0], &[Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)]);
        track
    }

    #[test]
    fn linear_keys_hit_exactly_at_the_key_and_halve_between() {
        let track = vector_track(Interpolation::Linear);
        assert_eq!(sample_position(0.0, &track), Vec3::ZERO);
        assert_eq!(sample_position(1.0, &track), Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(sample_position(0.5, &track), Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn sampling_clamps_outside_the_key_range() {
        let track = vector_track(Interpolation::Linear);
        assert_eq!(sample_position(-5.0, &track), Vec3::ZERO);
        assert_eq!(sample_position(9.0, &track), Vec3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn step_tracks_hold_the_previous_key() {
        let track = vector_track(Interpolation::Step);
        assert_eq!(sample_position(0.99, &track), Vec3::ZERO);
        assert_eq!(sample_position(1.0, &track), Vec3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn empty_scale_track_reads_one() {
        let track = VectorTrack::new(0);
        assert_eq!(sample_scale(0.0, &track), Vec3::ONE);
    }

    #[test]
    fn blend_copies_at_the_ends_and_mixes_between() {
        let mut from = Pose::new(1);
        from.bones[0].translation = Vec3::ZERO;
        let mut to = Pose::new(1);
        to.bones[0].translation = Vec3::new(4.0, 0.0, 0.0);
        let mut out = Pose::new(1);

        blend_poses(&from, &to, 0.0, &mut out);
        assert_eq!(out.bones[0].translation, Vec3::ZERO);
        blend_poses(&from, &to, 1.0, &mut out);
        assert_eq!(out.bones[0].translation, Vec3::new(4.0, 0.0, 0.0));
        blend_poses(&from, &to, 0.5, &mut out);
        assert_eq!(out.bones[0].translation, Vec3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn parented_bones_accumulate_the_parent_transform() {
        let mut skeleton = Skeleton::new();
        let root = Bone {
            id: 0,
            local_transform: Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            ..Default::default()
        };
        let child = Bone {
            id: 1,
            parent_id: 0,
            local_transform: Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
            ..Default::default()
        };
        skeleton.bones.push(root);
        skeleton.bones.push(child);

        let children = build_child_lists(&skeleton);
        let pose = rest_pose(&skeleton);
        let mut globals = Vec::new();
        global_transforms(&skeleton, &children, &pose, false, &mut globals);

        assert_eq!(globals[1].w_axis.truncate(), Vec3::new(1.0, 2.0, 0.0));
    }
}
