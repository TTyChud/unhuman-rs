//! Clip and skeleton data, ported from `Renderer3D/Animation.h`.
//!
//! A clip is a set of tracks (translation / rotation / scale), each a sorted list of
//! keys with an interpolation mode. `set_keys` / `set_spline_keys` build a track from
//! decoded accessor data, which is how `LoadModelAnimation.cpp::AppendTrackKeys` is
//! ported: the glTF reading stays with the loader, the track construction is here.

use glam::{Mat4, Quat, Vec3};

/// Keyframe interpolation mode, matching the glTF 2.0 animation sampler spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// Instantaneous state change at the keyframe time.
    Step,
    /// Linear (vectors) / spherical (rotations) blend between adjacent keys.
    Linear,
    /// Cubic Hermite spline with in/out tangents.
    CubicSpline,
}

/// Playback wrap behaviour when the play head reaches the duration boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode {
    /// Clamp and stop at the boundary.
    None,
    /// Wrap back to the start.
    Loop,
    /// Reverse direction at either end.
    PingPong,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe<T> {
    pub time: f32,
    pub value: T,
}

impl<T> Keyframe<T> {
    pub fn new(time: f32, value: T) -> Self {
        Self { time, value }
    }
}

/// Port of `VectorTrack` (`Animation.h`): positions or scales for one bone.
#[derive(Clone, Debug)]
pub struct VectorTrack {
    pub target_bone: i32,
    pub interpolation: Interpolation,
    pub keyframes: Vec<Keyframe<Vec3>>,
    /// CubicSpline only, parallel to `keyframes`. Empty for Step/Linear.
    pub in_tangents: Vec<Vec3>,
    /// CubicSpline only, parallel to `keyframes`.
    pub out_tangents: Vec<Vec3>,
}

impl Default for VectorTrack {
    fn default() -> Self {
        Self {
            target_bone: -1,
            interpolation: Interpolation::Linear,
            keyframes: Vec::new(),
            in_tangents: Vec::new(),
            out_tangents: Vec::new(),
        }
    }
}

impl VectorTrack {
    pub fn new(target_bone: i32) -> Self {
        Self {
            target_bone,
            ..Self::default()
        }
    }

    /// One value per time. Replaces whatever the track held.
    pub fn set_keys(&mut self, times: &[f32], values: &[Vec3]) {
        self.keyframes.clear();
        self.in_tangents.clear();
        self.out_tangents.clear();
        let count = times.len().min(values.len());
        self.keyframes.reserve(count);
        for i in 0..count {
            self.keyframes.push(Keyframe::new(times[i], values[i]));
        }
    }

    /// CUBICSPLINE layout: three values per key, `[inTangent, value, outTangent]`.
    pub fn set_spline_keys(&mut self, times: &[f32], values: &[Vec3]) {
        self.keyframes = vec![Keyframe::new(0.0, Vec3::ZERO); times.len()];
        self.in_tangents = vec![Vec3::ZERO; times.len()];
        self.out_tangents = vec![Vec3::ZERO; times.len()];
        for (i, value) in values.iter().enumerate() {
            let key = i / 3;
            if key >= times.len() {
                break;
            }
            match i % 3 {
                0 => self.in_tangents[key] = *value,
                1 => self.keyframes[key] = Keyframe::new(times[key], *value),
                _ => self.out_tangents[key] = *value,
            }
        }
    }

    pub fn duration(&self) -> f32 {
        self.keyframes.last().map_or(0.0, |k| k.time)
    }
}

/// Port of `QuaternionTrack` (`Animation.h`): orientations for one bone.
#[derive(Clone, Debug)]
pub struct QuaternionTrack {
    pub target_bone: i32,
    pub interpolation: Interpolation,
    pub keyframes: Vec<Keyframe<Quat>>,
    pub in_tangents: Vec<Quat>,
    pub out_tangents: Vec<Quat>,
}

impl Default for QuaternionTrack {
    fn default() -> Self {
        Self {
            target_bone: -1,
            interpolation: Interpolation::Linear,
            keyframes: Vec::new(),
            in_tangents: Vec::new(),
            out_tangents: Vec::new(),
        }
    }
}

impl QuaternionTrack {
    pub fn new(target_bone: i32) -> Self {
        Self {
            target_bone,
            ..Self::default()
        }
    }

    /// One unit quaternion per time. Values must already be normalised.
    pub fn set_keys(&mut self, times: &[f32], values: &[Quat]) {
        self.keyframes.clear();
        self.in_tangents.clear();
        self.out_tangents.clear();
        let count = times.len().min(values.len());
        self.keyframes.reserve(count);
        for i in 0..count {
            self.keyframes.push(Keyframe::new(times[i], values[i]));
        }
    }

    /// CUBICSPLINE layout: three quaternions per key, `[inTangent, value, outTangent]`.
    /// glTF rotation tangents are derivatives and stay unnormalised.
    pub fn set_spline_keys(&mut self, times: &[f32], values: &[Quat]) {
        self.keyframes = vec![Keyframe::new(0.0, Quat::IDENTITY); times.len()];
        self.in_tangents = vec![Quat::IDENTITY; times.len()];
        self.out_tangents = vec![Quat::IDENTITY; times.len()];
        for (i, value) in values.iter().enumerate() {
            let key = i / 3;
            if key >= times.len() {
                break;
            }
            match i % 3 {
                0 => self.in_tangents[key] = *value,
                1 => self.keyframes[key] = Keyframe::new(times[key], *value),
                _ => self.out_tangents[key] = *value,
            }
        }
    }

    pub fn duration(&self) -> f32 {
        self.keyframes.last().map_or(0.0, |k| k.time)
    }
}

/// Timeline marker fired when playback crosses its timestamp.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationEvent {
    pub time: f32,
    pub name: String,
}

impl AnimationEvent {
    pub fn new(time: f32, name: impl Into<String>) -> Self {
        Self {
            time,
            name: name.into(),
        }
    }
}

/// Port of `AnimationClip` (`Animation.h`).
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub loop_mode: LoopMode,
    pub position_tracks: Vec<VectorTrack>,
    pub rotation_tracks: Vec<QuaternionTrack>,
    pub scale_tracks: Vec<VectorTrack>,
    pub events: Vec<AnimationEvent>,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            name: String::new(),
            duration: 0.0,
            loop_mode: LoopMode::Loop,
            position_tracks: Vec::new(),
            rotation_tracks: Vec::new(),
            scale_tracks: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl Clip {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The track already targeting `target_bone`, or a new one. glTF may emit
    /// several channels for the same (bone, path) pair; merging them keeps the
    /// sampled pose well defined. Port of `FindOrAddTrack`.
    pub fn position_track_mut(&mut self, target_bone: i32) -> &mut VectorTrack {
        match self
            .position_tracks
            .iter()
            .position(|t| t.target_bone == target_bone)
        {
            Some(i) => &mut self.position_tracks[i],
            None => {
                self.position_tracks.push(VectorTrack::new(target_bone));
                self.position_tracks.last_mut().expect("just pushed")
            }
        }
    }

    pub fn rotation_track_mut(&mut self, target_bone: i32) -> &mut QuaternionTrack {
        match self
            .rotation_tracks
            .iter()
            .position(|t| t.target_bone == target_bone)
        {
            Some(i) => &mut self.rotation_tracks[i],
            None => {
                self.rotation_tracks.push(QuaternionTrack::new(target_bone));
                self.rotation_tracks.last_mut().expect("just pushed")
            }
        }
    }

    pub fn scale_track_mut(&mut self, target_bone: i32) -> &mut VectorTrack {
        match self
            .scale_tracks
            .iter()
            .position(|t| t.target_bone == target_bone)
        {
            Some(i) => &mut self.scale_tracks[i],
            None => {
                self.scale_tracks.push(VectorTrack::new(target_bone));
                self.scale_tracks.last_mut().expect("just pushed")
            }
        }
    }

    /// Recomputes `duration` as the last key time across every track.
    pub fn update_duration(&mut self) -> f32 {
        let mut duration: f32 = 0.0;
        for track in &self.position_tracks {
            duration = duration.max(track.duration());
        }
        for track in &self.rotation_tracks {
            duration = duration.max(track.duration());
        }
        for track in &self.scale_tracks {
            duration = duration.max(track.duration());
        }
        self.duration = duration;
        duration
    }
}

/// Port of `Bone` (`Animation.h`).
#[derive(Clone, Debug)]
pub struct Bone {
    pub name: String,
    pub id: i32,
    pub parent_id: i32,
    pub inverse_bind_matrix: Mat4,
    pub local_transform: Mat4,
}

impl Default for Bone {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: -1,
            parent_id: -1,
            inverse_bind_matrix: Mat4::IDENTITY,
            local_transform: Mat4::IDENTITY,
        }
    }
}

/// Port of `Skeleton` (`Animation.h`).
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
    pub joint_nodes: Vec<i32>,
    pub root_bone_id: i32,
}

impl Default for Skeleton {
    fn default() -> Self {
        Self::new()
    }
}

impl Skeleton {
    pub fn new() -> Self {
        Self {
            bones: Vec::new(),
            joint_nodes: Vec::new(),
            root_bone_id: -1,
        }
    }
}

/// Port of `Skin` (`Animation.h`): a joint subset with parallel inverse binds.
#[derive(Clone, Debug, Default)]
pub struct Skin {
    pub name: String,
    pub joint_nodes: Vec<i32>,
    /// Parallel to `joint_nodes`.
    pub inverse_bind_matrices: Vec<Mat4>,
}

/// Sampled local pose of one bone. Port of `BonePose` (`Animation.h`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePose {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// True when at least one track targets this bone.
    pub animated: bool,
}

impl Default for BonePose {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            animated: false,
        }
    }
}

/// Per-bone pose table for one sampled moment of a clip, indexed by bone ID.
/// Port of `AnimationPose` (`Animation.h`).
#[derive(Clone, Debug, Default)]
pub struct Pose {
    pub bones: Vec<BonePose>,
}

impl Pose {
    pub fn new(bone_count: usize) -> Self {
        let mut pose = Self::default();
        pose.resize(bone_count);
        pose
    }

    /// Resizes to `bone_count` and resets every entry to identity.
    pub fn resize(&mut self, bone_count: usize) {
        self.bones.clear();
        self.bones.resize(bone_count, BonePose::default());
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_lookup_reuses_the_existing_track() {
        let mut clip = Clip::new("walk");
        clip.position_track_mut(3).set_keys(&[0.0, 1.0], &[Vec3::ZERO, Vec3::X]);
        clip.position_track_mut(3)
            .set_spline_keys(&[0.0, 1.0], &[Vec3::ZERO; 6]);
        assert_eq!(clip.position_tracks.len(), 1);
        assert_eq!(clip.position_tracks[0].interpolation, Interpolation::Linear);
    }

    #[test]
    fn duration_is_the_last_key_across_tracks() {
        let mut clip = Clip::new("run");
        clip.position_track_mut(0).set_keys(&[0.0, 2.0], &[Vec3::ZERO; 2]);
        clip.rotation_track_mut(0).set_keys(&[0.0, 5.0], &[Quat::IDENTITY; 2]);
        assert_eq!(clip.update_duration(), 5.0);
        assert_eq!(clip.duration, 5.0);
    }

    #[test]
    fn spline_keys_land_in_value_and_tangent_slots() {
        let mut track = VectorTrack::new(0);
        let values = [
            Vec3::X, Vec3::Y, Vec3::Z, // key 0: in, value, out
            Vec3::ZERO, Vec3::ONE, Vec3::ZERO,
        ];
        track.set_spline_keys(&[0.0, 1.0], &values);
        assert_eq!(track.in_tangents[0], Vec3::X);
        assert_eq!(track.keyframes[0].value, Vec3::Y);
        assert_eq!(track.out_tangents[0], Vec3::Z);
        assert_eq!(track.keyframes[1].value, Vec3::ONE);
        assert_eq!(track.duration(), 1.0);
    }

    #[test]
    fn a_pose_resize_resets_to_identity() {
        let mut pose = Pose::new(2);
        assert_eq!(pose.bones.len(), 2);
        assert_eq!(pose.bones[0].scale, Vec3::ONE);
        assert_eq!(pose.bones[0].rotation, Quat::IDENTITY);
    }
}
