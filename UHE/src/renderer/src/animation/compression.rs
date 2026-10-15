//! Keyframe reduction, ported from `Renderer3D/AnimationCompression.cpp` (CPU path).
//!
//! A greedy linear reduction: an interior key is dropped when the straight blend
//! between the last kept key and the next original key stays within tolerance at
//! every original sample time, so the reduced track never deviates by more than the
//! configured tolerance where the original had a key. Step tracks are preserved
//! verbatim — their semantics change if interior keys go.

use glam::{Quat, Vec3};

use crate::animation::clip::{Clip, Interpolation, Keyframe, QuaternionTrack, VectorTrack};

/// Error tolerances. Port of `CompressSettings`.
#[derive(Clone, Copy, Debug)]
pub struct CompressSettings {
    /// Linear units.
    pub position_tolerance: f32,
    /// Scale units.
    pub scale_tolerance: f32,
    /// Radians.
    pub rotation_tolerance: f32,
}

impl Default for CompressSettings {
    fn default() -> Self {
        Self {
            position_tolerance: 1e-3,
            scale_tolerance: 1e-4,
            rotation_tolerance: 0.02,
        }
    }
}

/// Port of `CompressionStats`. Byte counts use the C++ element sizes so the two
/// trees' estimates stay comparable.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompressionStats {
    pub keys_before: usize,
    pub keys_after: usize,
    pub tracks_processed: usize,
    pub bytes_before: usize,
    pub bytes_after: usize,
}

impl CompressionStats {
    pub fn key_reduction(&self) -> f32 {
        if self.keys_before > 0 {
            1.0 - self.keys_after as f32 / self.keys_before as f32
        } else {
            0.0
        }
    }

    pub fn byte_reduction(&self) -> f32 {
        if self.bytes_before > 0 {
            1.0 - self.bytes_after as f32 / self.bytes_before as f32
        } else {
            0.0
        }
    }

    fn merge(&mut self, other: &Self) {
        self.keys_before += other.keys_before;
        self.keys_after += other.keys_after;
        self.tracks_processed += other.tracks_processed;
        self.bytes_before += other.bytes_before;
        self.bytes_after += other.bytes_after;
    }
}

/// The two key value types a track can hold. Static dispatch only: the C++ used
/// templates for exactly this.
trait TrackValue: Copy {
    /// `sizeof(Keyframe<T>)`: time plus value.
    const KEY_BYTES: usize;
    /// `sizeof(T)` for one tangent.
    const TANGENT_BYTES: usize;

    fn blend(a: Self, b: Self, u: f32) -> Self;
    fn error(a: Self, b: Self) -> f32;
    /// One Hermite segment from `a` to `b` in normalised `u`.
    fn spline(a: Self, b: Self, a_out: Self, b_in: Self, span: f32, u: f32) -> Self;
}

impl TrackValue for Vec3 {
    const KEY_BYTES: usize = 16;
    const TANGENT_BYTES: usize = 12;

    fn blend(a: Self, b: Self, u: f32) -> Self {
        a.lerp(b, u)
    }

    fn error(a: Self, b: Self) -> f32 {
        (a - b).length()
    }

    fn spline(a: Self, b: Self, a_out: Self, b_in: Self, span: f32, u: f32) -> Self {
        let (h00, h10, h01, h11) = hermite_basis(u);
        h00 * a + h10 * span * a_out + h01 * b + h11 * span * b_in
    }
}

impl TrackValue for Quat {
    const KEY_BYTES: usize = 20;
    const TANGENT_BYTES: usize = 16;

    fn blend(a: Self, b: Self, u: f32) -> Self {
        a.slerp(b, u)
    }

    fn error(a: Self, b: Self) -> f32 {
        let d = a.dot(b).abs().clamp(0.0, 1.0);
        2.0 * d.acos()
    }

    fn spline(a: Self, b: Self, a_out: Self, b_in: Self, span: f32, u: f32) -> Self {
        let (h00, h10, h01, h11) = hermite_basis(u);
        (a * h00 + a_out * (h10 * span) + b * h01 + b_in * (h11 * span)).normalize()
    }
}

fn hermite_basis(t: f32) -> (f32, f32, f32, f32) {
    let t2 = t * t;
    let t3 = t2 * t;
    (
        2.0 * t3 - 3.0 * t2 + 1.0,
        t3 - 2.0 * t2 + t,
        -2.0 * t3 + 3.0 * t2,
        t3 - t2,
    )
}

/// Value the track takes at `time` when jumping straight from key `a` to key `b`:
/// linear for Linear tracks, one long Hermite segment for CubicSpline tracks.
fn sample_between<T: TrackValue>(
    a: &Keyframe<T>,
    b: &Keyframe<T>,
    a_out_tangent: T,
    b_in_tangent: T,
    spline: bool,
    time: f32,
) -> T {
    let span = b.time - a.time;
    let u = if span > 0.0 {
        ((time - a.time) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if spline {
        T::spline(a.value, b.value, a_out_tangent, b_in_tangent, span, u)
    } else {
        T::blend(a.value, b.value, u)
    }
}

/// The indices to keep: always the first and the last key.
fn compute_keep_mask<T: TrackValue>(
    keys: &[Keyframe<T>],
    in_tangents: &[T],
    out_tangents: &[T],
    spline: bool,
    tolerance: f32,
) -> Vec<usize> {
    let mut keep = Vec::new();
    if keys.is_empty() {
        return keep;
    }
    if keys.len() <= 2 {
        keep.extend(0..keys.len());
        return keep;
    }

    // Zero vector / identity quaternion fallback for missing tangents.
    let empty_tangent = T::blend(keys[0].value, keys[0].value, 0.0);
    let tangent_at = |tangents: &[T], i: usize| -> T {
        if i < tangents.len() {
            tangents[i]
        } else {
            empty_tangent
        }
    };

    keep.push(0);
    let mut last_kept = 0;
    let mut i = 1;
    while i + 1 < keys.len() {
        let a = &keys[last_kept];
        let b = &keys[i + 1];

        // Recheck all intermediate skipped keys against the candidate segment.
        let mut all_within_tolerance = true;
        for key in &keys[(last_kept + 1)..=i] {
            let blended = sample_between(
                a,
                b,
                tangent_at(out_tangents, last_kept),
                tangent_at(in_tangents, i + 1),
                spline,
                key.time,
            );
            if T::error(key.value, blended) > tolerance {
                all_within_tolerance = false;
                break;
            }
        }

        if all_within_tolerance {
            i += 1; // drop key i
        } else {
            keep.push(i);
            last_kept = i;
            i += 1;
        }
    }
    keep.push(keys.len() - 1);
    keep
}

/// The entries `keep` names, in order. Missing indices are dropped rather than
/// panicking, so a track whose tangents and keys disagree cannot crash a load.
fn filter_by_mask<U: Copy>(values: &[U], keep: &[usize]) -> Vec<U> {
    let mut kept = Vec::with_capacity(keep.len());
    for &index in keep {
        if let Some(value) = values.get(index) {
            kept.push(*value);
        }
    }
    kept
}

/// Reduces one track. `reduce` is false for STEP tracks, whose step semantics would
/// change if interior keys were dropped.
fn process_track<T: TrackValue>(
    keys: &mut Vec<Keyframe<T>>,
    in_tangents: &mut Vec<T>,
    out_tangents: &mut Vec<T>,
    tolerance: f32,
    reduce: bool,
    apply: bool,
) -> CompressionStats {
    let mut stats = CompressionStats {
        keys_before: keys.len(),
        tracks_processed: 1,
        ..CompressionStats::default()
    };

    let spline = !in_tangents.is_empty()
        && in_tangents.len() == keys.len()
        && out_tangents.len() == keys.len();
    let before_tangent_bytes = if spline {
        2 * keys.len() * T::TANGENT_BYTES
    } else {
        0
    };
    stats.bytes_before = keys.len() * T::KEY_BYTES + before_tangent_bytes;
    stats.keys_after = keys.len();
    stats.bytes_after = stats.bytes_before;

    if reduce && keys.len() > 2 {
        let keep = compute_keep_mask(keys, in_tangents, out_tangents, spline, tolerance);
        let after_tangent_bytes = if spline {
            2 * keep.len() * T::TANGENT_BYTES
        } else {
            0
        };
        stats.keys_after = keep.len();
        stats.bytes_after = keep.len() * T::KEY_BYTES + after_tangent_bytes;
        if apply && keep.len() < keys.len() {
            let kept_keys = filter_by_mask(keys, &keep);
            let kept_in = filter_by_mask(in_tangents, &keep);
            let kept_out = filter_by_mask(out_tangents, &keep);
            keys.clear();
            keys.extend(kept_keys);
            in_tangents.clear();
            in_tangents.extend(kept_in);
            out_tangents.clear();
            out_tangents.extend(kept_out);
        }
    }

    stats
}

fn process_vector_track(track: &mut VectorTrack, tolerance: f32, apply: bool) -> CompressionStats {
    let reduce = track.interpolation == Interpolation::Linear;
    process_track(
        &mut track.keyframes,
        &mut track.in_tangents,
        &mut track.out_tangents,
        tolerance,
        reduce,
        apply,
    )
}

fn process_quaternion_track(
    track: &mut QuaternionTrack,
    tolerance: f32,
    apply: bool,
) -> CompressionStats {
    let reduce = track.interpolation == Interpolation::Linear;
    process_track(
        &mut track.keyframes,
        &mut track.in_tangents,
        &mut track.out_tangents,
        tolerance,
        reduce,
        apply,
    )
}

fn run_reduction(clip: &mut Clip, settings: &CompressSettings, apply: bool) -> CompressionStats {
    let mut total = CompressionStats::default();
    for track in &mut clip.position_tracks {
        let stats = process_vector_track(track, settings.position_tolerance, apply);
        total.merge(&stats);
    }
    for track in &mut clip.rotation_tracks {
        let stats = process_quaternion_track(track, settings.rotation_tolerance, apply);
        total.merge(&stats);
    }
    for track in &mut clip.scale_tracks {
        let stats = process_vector_track(track, settings.scale_tolerance, apply);
        total.merge(&stats);
    }
    total
}

/// Reduces the keyframe counts of `clip` in place.
pub fn compress_clip(clip: &mut Clip, settings: &CompressSettings) -> CompressionStats {
    run_reduction(clip, settings, true)
}

/// Measures the reduction `compress_clip` would achieve, without touching `clip`.
pub fn measure_clip(clip: &Clip, settings: &CompressSettings) -> CompressionStats {
    let mut copy = clip.clone();
    run_reduction(&mut copy, settings, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_position_clip(values: &[Vec3]) -> Clip {
        let mut clip = Clip::new("test");
        let times: Vec<f32> = (0..values.len()).map(|i| i as f32).collect();
        clip.position_track_mut(0).set_keys(&times, values);
        clip.update_duration();
        clip
    }

    #[test]
    fn a_straight_track_reduces_to_its_ends() {
        let values = [
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ];
        let mut clip = linear_position_clip(&values);
        let stats = compress_clip(&mut clip, &CompressSettings::default());
        assert_eq!(clip.position_tracks[0].keyframes.len(), 2);
        assert_eq!(stats.keys_before, 4);
        assert_eq!(stats.keys_after, 2);
        assert_eq!(stats.tracks_processed, 1);
        assert!(stats.key_reduction() > 0.4);
    }

    #[test]
    fn a_curved_track_keeps_the_key_that_carries_the_curve() {
        let values = [
            Vec3::ZERO,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::ZERO,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::ZERO,
        ];
        let mut clip = linear_position_clip(&values);
        compress_clip(&mut clip, &CompressSettings::default());
        assert!(clip.position_tracks[0].keyframes.len() >= 4);
    }

    #[test]
    fn step_tracks_are_left_alone() {
        let values = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
        let mut clip = linear_position_clip(&values);
        clip.position_tracks[0].interpolation = Interpolation::Step;
        let stats = compress_clip(&mut clip, &CompressSettings::default());
        assert_eq!(clip.position_tracks[0].keyframes.len(), 4);
        assert_eq!(stats.keys_after, 4);
    }

    #[test]
    fn measure_does_not_touch_the_clip() {
        let values = [Vec3::ZERO, Vec3::X, Vec3::new(2.0, 0.0, 0.0)];
        let clip = linear_position_clip(&values);
        let stats = measure_clip(&clip, &CompressSettings::default());
        assert_eq!(clip.position_tracks[0].keyframes.len(), 3);
        assert_eq!(stats.keys_after, 2);
    }

    #[test]
    fn a_short_track_is_untouched() {
        let values = [Vec3::ZERO, Vec3::X];
        let mut clip = linear_position_clip(&values);
        compress_clip(&mut clip, &CompressSettings::default());
        assert_eq!(clip.position_tracks[0].keyframes.len(), 2);
    }
}
