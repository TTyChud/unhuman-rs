use std::mem::size_of;

use crate::mesh::{AnimationData, QuaternionTrack, VectorTrack};

/// Keyframe reduction tolerances, in the units each track is measured in.
///
/// Port of `CompressSettings` (`AnimationCompression.h:24`): translations and
/// scales are compared by Euclidean distance, rotations by angular difference
/// in radians.
#[derive(Clone, Copy, Debug)]
pub struct CompressSettings {
    pub position_tolerance: f32,
    pub scale_tolerance: f32,
    pub rotation_tolerance: f32,
}

impl Default for CompressSettings {
    fn default() -> Self {
        Self {
            position_tolerance: 1e-3,
            scale_tolerance: 1e-4,
            // ~1.15 degrees, which is under what a 16-bit quantised orientation
            // can express anyway.
            rotation_tolerance: 0.02,
        }
    }
}

/// What a reduction pass did, in keys and in bytes.
///
/// Port of `CompressionStats` (`AnimationCompression.h:33`). The byte figures
/// count the stride this crate stores a key in — an `f32` time plus the value,
/// and one value per tangent — which is the layout a renderer uploads.
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
        if self.keys_before == 0 {
            0.0
        } else {
            1.0 - self.keys_after as f32 / self.keys_before as f32
        }
    }

    pub fn byte_reduction(&self) -> f32 {
        if self.bytes_before == 0 {
            0.0
        } else {
            1.0 - self.bytes_after as f32 / self.bytes_before as f32
        }
    }

    fn merge(&mut self, part: CompressionStats) {
        self.keys_before += part.keys_before;
        self.keys_after += part.keys_after;
        self.tracks_processed += part.tracks_processed;
        self.bytes_before += part.bytes_before;
        self.bytes_after += part.bytes_after;
    }
}

/// Greedy key reduction over one clip, in place. Port of
/// `AnimationCompressor::CompressClip` (`AnimationCompression.cpp:196`).
///
/// `CubicSpline` and `Step` tracks are measured but never reduced: a step
/// track's semantics change if an interior key is dropped, and the C++ only
/// reduced tracks that interpolate linearly.
pub fn compress(clip: &mut AnimationData, settings: CompressSettings) -> CompressionStats {
    let mut stats = CompressionStats::default();
    for track in &mut clip.position_tracks {
        let reduce = track.interpolation == crate::mesh::Interpolation::Linear;
        stats.merge(process_track(
            &mut track.times,
            &mut track.values,
            &mut track.in_tangents,
            &mut track.out_tangents,
            settings.position_tolerance,
            reduce,
            true,
        ));
    }
    for track in &mut clip.rotation_tracks {
        let reduce = track.interpolation == crate::mesh::Interpolation::Linear;
        stats.merge(process_track(
            &mut track.times,
            &mut track.values,
            &mut track.in_tangents,
            &mut track.out_tangents,
            settings.rotation_tolerance,
            reduce,
            true,
        ));
    }
    for track in &mut clip.scale_tracks {
        let reduce = track.interpolation == crate::mesh::Interpolation::Linear;
        stats.merge(process_track(
            &mut track.times,
            &mut track.values,
            &mut track.in_tangents,
            &mut track.out_tangents,
            settings.scale_tolerance,
            reduce,
            true,
        ));
    }
    stats
}

/// The same measurement without changing the clip, so a caller can decide
/// whether compressing is worth it. Port of `AnimationCompressor::MeasureClip`.
pub fn measure(clip: &AnimationData, settings: CompressSettings) -> CompressionStats {
    let mut stats = CompressionStats::default();
    for track in &clip.position_tracks {
        stats.merge(measure_track::<[f32; 3]>(
            &track.times,
            &track.in_tangents,
            &track.out_tangents,
            settings.position_tolerance,
            track.interpolation,
        ));
    }
    for track in &clip.rotation_tracks {
        stats.merge(measure_track::<[f32; 4]>(
            &track.times,
            &track.in_tangents,
            &track.out_tangents,
            settings.rotation_tolerance,
            track.interpolation,
        ));
    }
    for track in &clip.scale_tracks {
        stats.merge(measure_track::<[f32; 3]>(
            &track.times,
            &track.in_tangents,
            &track.out_tangents,
            settings.scale_tolerance,
            track.interpolation,
        ));
    }
    stats
}

/// A track's values and tangents cost the same per key as `T`.
fn measure_track<T: TrackValue>(
    times: &[f32],
    in_tangents: &[T],
    out_tangents: &[T],
    tolerance: f32,
    interpolation: crate::mesh::Interpolation,
) -> CompressionStats {
    let mut stats = CompressionStats::default();
    stats.keys_before = times.len();
    stats.keys_after = times.len();
    stats.tracks_processed = 1;
    let spline = is_spline(times.len(), in_tangents, out_tangents);
    stats.bytes_before = bytes_of::<T>(times.len(), spline);
    stats.bytes_after = stats.bytes_before;

    if interpolation == crate::mesh::Interpolation::Linear && times.len() > 2 {
        let keep = keep_count(times, tolerance, spline, in_tangents, out_tangents);
        stats.keys_after = keep;
        stats.bytes_after = bytes_of::<T>(keep, spline);
    }
    stats
}

fn bytes_of<T>(keys: usize, spline: bool) -> usize {
    keys * (size_of::<f32>() + size_of::<T>())
        + if spline {
            2 * keys * size_of::<T>()
        } else {
            0
        }
}

fn process_track<T: TrackValue>(
    times: &mut Vec<f32>,
    values: &mut Vec<T>,
    in_tangents: &mut Vec<T>,
    out_tangents: &mut Vec<T>,
    tolerance: f32,
    reduce: bool,
    apply: bool,
) -> CompressionStats {
    let spline = is_spline(times.len(), in_tangents, out_tangents);

    let mut stats = CompressionStats::default();
    stats.keys_before = times.len();
    stats.keys_after = times.len();
    stats.tracks_processed = 1;
    stats.bytes_before = bytes_of::<T>(times.len(), spline);
    stats.bytes_after = stats.bytes_before;

    if reduce && times.len() > 2 {
        let keep = keep_mask(times, values, in_tangents, out_tangents, spline, tolerance);
        stats.keys_after = keep.len();
        stats.bytes_after = bytes_of::<T>(keep.len(), spline);
        if apply && keep.len() < times.len() {
            apply_mask(times, &keep);
            apply_mask(values, &keep);
            apply_mask(in_tangents, &keep);
            apply_mask(out_tangents, &keep);
        }
    }

    stats
}

fn is_spline<T>(keys: usize, in_tangents: &[T], out_tangents: &[T]) -> bool {
    !in_tangents.is_empty() && in_tangents.len() == keys && out_tangents.len() == keys
}

fn keep_count<T: TrackValue>(
    times: &[f32],
    tolerance: f32,
    spline: bool,
    in_tangents: &[T],
    out_tangents: &[T],
) -> usize {
    if times.len() <= 2 {
        return times.len();
    }
    let values = vec![T::ZERO; times.len()];
    keep_mask(times, &values, in_tangents, out_tangents, spline, tolerance).len()
}

/// The indices to keep: greedy, walking the track once, keeping a key when the
/// straight blend from the last kept key to the next original key would deviate
/// from the original by more than `tolerance`.
///
/// Port of `ComputeKeepMask` (`AnimationCompression.cpp:88`). Every key that is
/// dropped was checked against the final segment that spans it, so the reduced
/// track is within `tolerance` of the original at every original sample time.
fn keep_mask<T: TrackValue>(
    times: &[f32],
    values: &[T],
    in_tangents: &[T],
    out_tangents: &[T],
    spline: bool,
    tolerance: f32,
) -> Vec<usize> {
    let mut keep = Vec::new();
    if times.is_empty() {
        return keep;
    }
    if times.len() <= 2 {
        return (0..times.len()).collect();
    }

    let tangent_in = |index: usize| -> T {
        in_tangents.get(index).copied().unwrap_or(T::ZERO)
    };
    let tangent_out = |index: usize| -> T {
        out_tangents.get(index).copied().unwrap_or(T::ZERO)
    };

    keep.push(0);
    let mut last_kept = 0usize;
    let mut i = 1usize;
    while i + 1 < times.len() {
        let mut within = true;
        for k in last_kept + 1..=i {
            let blended = sample_between(
                (times[last_kept], values[last_kept], tangent_out(last_kept)),
                (times[i + 1], values[i + 1], tangent_in(i + 1)),
                spline,
                times[k],
            );
            if T::error(values[k], blended) > tolerance {
                within = false;
                break;
            }
        }

        if within {
            i += 1;
        } else {
            keep.push(i);
            last_kept = i;
            i += 1;
        }
    }
    keep.push(times.len() - 1);
    keep
}

/// Value the track takes at `time` when jumping straight from key `a` to key
/// `b`: linear for `Linear` tracks, one long Hermite segment for `CubicSpline`
/// tracks. Port of `SampleBetween` (`AnimationCompression.cpp:42`).
fn sample_between<T: TrackValue>(a: (f32, T, T), b: (f32, T, T), spline: bool, time: f32) -> T {
    let (a_time, a_value, a_out) = a;
    let (b_time, b_value, b_in) = b;
    let span = b_time - a_time;
    let u = if span > 0.0 {
        ((time - a_time) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };

    if !spline {
        return T::blend(a_value, b_value, u);
    }
    T::hermite(a_value, b_value, a_out, b_in, span, u)
}

fn apply_mask<T: Copy>(values: &mut Vec<T>, keep: &[usize]) {
    if values.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(keep.len());
    for index in keep {
        if let Some(value) = values.get(*index) {
            kept.push(*value);
        }
    }
    *values = kept;
}

/// A value a track can hold, with the two operations the reduction needs.
///
/// This is the `template <typename T>` of `AnimationCompression.cpp` spelled
/// out for the two types glTF actually has. Rotations are unit quaternions in
/// `[x, y, z, w]` order and are blended on the sphere; vectors are blended
/// component-wise.
pub(crate) trait TrackValue: Copy {
    const ZERO: Self;

    fn blend(a: Self, b: Self, u: f32) -> Self;
    fn error(a: Self, b: Self) -> f32;
    /// Cubic Hermite, with the tangents already scaled by the segment duration.
    fn hermite(a: Self, b: Self, a_out: Self, b_in: Self, span: f32, t: f32) -> Self;
}

impl TrackValue for [f32; 3] {
    const ZERO: Self = [0.0; 3];

    fn blend(a: Self, b: Self, u: f32) -> Self {
        [
            a[0] + (b[0] - a[0]) * u,
            a[1] + (b[1] - a[1]) * u,
            a[2] + (b[2] - a[2]) * u,
        ]
    }

    fn error(a: Self, b: Self) -> f32 {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    fn hermite(a: Self, b: Self, a_out: Self, b_in: Self, span: f32, t: f32) -> Self {
        let (h00, h10, h01, h11) = hermite_basis(t);
        let mut out = [0.0; 3];
        for i in 0..3 {
            out[i] =
                h00 * a[i] + h10 * span * a_out[i] + h01 * b[i] + h11 * span * b_in[i];
        }
        out
    }
}

impl TrackValue for [f32; 4] {
    const ZERO: Self = [0.0, 0.0, 0.0, 0.0];

    fn blend(a: Self, b: Self, u: f32) -> Self {
        slerp(a, b, u)
    }

    fn error(a: Self, b: Self) -> f32 {
        let a = normalize(a);
        let b = normalize(b);
        let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs();
        2.0 * dot.clamp(0.0, 1.0).acos()
    }

    fn hermite(a: Self, b: Self, a_out: Self, b_in: Self, span: f32, t: f32) -> Self {
        let (h00, h10, h01, h11) = hermite_basis(t);
        let mut out = [0.0; 4];
        for i in 0..4 {
            out[i] =
                h00 * a[i] + h10 * span * a_out[i] + h01 * b[i] + h11 * span * b_in[i];
        }
        normalize(out)
    }
}

/// Hermite basis at `t`, as `(h00, h10, h01, h11)`.
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

/// Unit quaternion, or identity when the input cannot be normalised.
///
/// A zero-length quaternion in a file would otherwise turn every blend of that
/// track into NaN and every `acos` into a panic-free but meaningless angle.
pub(crate) fn normalize(q: [f32; 4]) -> [f32; 4] {
    let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if !length.is_finite() || length < 1e-8 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / length, q[1] / length, q[2] / length, q[3] / length]
}

/// Shortest-arc spherical interpolation, written out rather than taken from a
/// math library so a degenerate input cannot panic in a debug build.
pub(crate) fn slerp(a: [f32; 4], b: [f32; 4], u: f32) -> [f32; 4] {
    let a = normalize(a);
    let mut b = normalize(b);
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
        dot = -dot;
    }

    // Nearly parallel: the spherical formula divides by ~0.
    if dot > 0.9995 {
        return normalize([
            a[0] + (b[0] - a[0]) * u,
            a[1] + (b[1] - a[1]) * u,
            a[2] + (b[2] - a[2]) * u,
            a[3] + (b[3] - a[3]) * u,
        ]);
    }

    let theta = dot.clamp(-1.0, 1.0).acos();
    let sin_theta = theta.sin();
    let wa = ((1.0 - u) * theta).sin() / sin_theta;
    let wb = (u * theta).sin() / sin_theta;
    normalize([
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
        a[3] * wa + b[3] * wb,
    ])
}

/// The value a position or scale track takes at `time`.
///
/// This is the sampling convention the tolerances above are stated against:
/// times are the keyframe times, the segment is found by the first key after
/// `time`, and `Step` holds the previous value, `Linear` blends, `CubicSpline`
/// runs one Hermite segment with the keys' tangents scaled by its duration.
/// The C++ runtime sampler lives in `Animator.cpp`; this is the same convention
/// in the loader's own data so a compressed clip can be checked where it is
/// made.
impl VectorTrack {
    pub fn sample(&self, time: f32) -> [f32; 3] {
        sample_value(
            &self.times,
            &self.values,
            &self.in_tangents,
            &self.out_tangents,
            self.interpolation,
            time,
        )
    }
}

impl QuaternionTrack {
    pub fn sample(&self, time: f32) -> [f32; 4] {
        sample_value(
            &self.times,
            &self.values,
            &self.in_tangents,
            &self.out_tangents,
            self.interpolation,
            time,
        )
    }
}

fn sample_value<T: TrackValue>(
    times: &[f32],
    values: &[T],
    in_tangents: &[T],
    out_tangents: &[T],
    interpolation: crate::mesh::Interpolation,
    time: f32,
) -> T {
    if times.is_empty() || values.is_empty() {
        return T::ZERO;
    }
    if times.len() == 1 || values.len() == 1 {
        return values[0];
    }

    // First key strictly after `time`, i.e. `upper_bound`.
    let index = times.partition_point(|key| *key <= time);
    if index == 0 {
        return values[0];
    }
    if index >= times.len() {
        return values[values.len() - 1];
    }

    let a = index - 1;
    let b = index;
    let span = times[b] - times[a];
    let u = if span > 0.0 {
        ((time - times[a]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };

    match interpolation {
        crate::mesh::Interpolation::Step => values[a],
        crate::mesh::Interpolation::Linear => T::blend(values[a], values[b], u),
        crate::mesh::Interpolation::CubicSpline => {
            let a_out = out_tangents.get(a).copied().unwrap_or(T::ZERO);
            let b_in = in_tangents.get(b).copied().unwrap_or(T::ZERO);
            T::hermite(values[a], values[b], a_out, b_in, span, u)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Interpolation;

    fn linear_position_track(samples: usize, amplitude: f32) -> VectorTrack {
        let mut track = VectorTrack {
            interpolation: Interpolation::Linear,
            ..Default::default()
        };
        for i in 0..samples {
            let t = i as f32 / (samples - 1) as f32;
            track.times.push(t);
            track
                .values
                .push([amplitude * t * t, (amplitude * t).sin(), 0.0]);
        }
        track
    }

    #[test]
    fn a_linear_track_loses_its_middle_keys_and_stays_within_tolerance() {
        let track = linear_position_track(65, 1.0);
        let mut clip = AnimationData {
            duration: 1.0,
            position_tracks: vec![track],
            ..Default::default()
        };
        let original = clip.position_tracks[0].values.clone();
        let stats = compress(&mut clip, CompressSettings::default());
        let reduced = &clip.position_tracks[0];

        assert!(stats.keys_after < stats.keys_before);
        assert_eq!(stats.keys_after, reduced.times.len());
        assert_eq!(stats.tracks_processed, 1);
        // The bound is stated at the original sample times, which is where the
        // keep-mask checked it.
        for (time, value) in clip_original_times(&original) {
            let sampled = reduced.sample(time);
            let error = <[f32; 3] as TrackValue>::error(value, sampled);
            assert!(
                error <= CompressSettings::default().position_tolerance + 1e-6,
                "error {error} at {time}"
            );
        }
    }

    fn clip_original_times(values: &[[f32; 3]]) -> Vec<(f32, [f32; 3])> {
        let count = values.len();
        values
            .iter()
            .enumerate()
            .map(|(i, value)| (i as f32 / (count - 1) as f32, *value))
            .collect()
    }

    #[test]
    fn a_step_track_is_not_reduced() {
        let mut track = linear_position_track(33, 1.0);
        track.interpolation = Interpolation::Step;
        let before = track.times.len();
        let mut clip = AnimationData {
            position_tracks: vec![track],
            ..Default::default()
        };
        let stats = compress(&mut clip, CompressSettings::default());
        assert_eq!(stats.keys_after, before);
        assert_eq!(clip.position_tracks[0].times.len(), before);
    }

    #[test]
    fn measure_reports_what_compress_would_do_without_touching_the_clip() {
        let clip = AnimationData {
            position_tracks: vec![linear_position_track(65, 1.0)],
            ..Default::default()
        };
        let settings = CompressSettings::default();
        let measured = measure(&clip, settings);
        let mut copied = clip.clone();
        let applied = compress(&mut copied, settings);
        assert_eq!(measured.keys_before, applied.keys_before);
        assert_eq!(measured.keys_after, applied.keys_after);
        assert_eq!(clip.position_tracks[0].times.len(), 65);
    }

    #[test]
    fn rotation_reduction_stays_within_its_angular_tolerance() {
        let mut track = QuaternionTrack {
            interpolation: Interpolation::Linear,
            ..Default::default()
        };
        let samples = 49;
        for i in 0..samples {
            let t = i as f32 / (samples - 1) as f32;
            let angle = t * std::f32::consts::FRAC_PI_2;
            track.times.push(t);
            track
                .values
                .push([0.0, (angle * 0.5).sin(), 0.0, (angle * 0.5).cos()]);
        }
        let original = track.values.clone();
        let times = track.times.clone();
        let mut clip = AnimationData {
            rotation_tracks: vec![track],
            ..Default::default()
        };
        compress(&mut clip, CompressSettings::default());
        let reduced = &clip.rotation_tracks[0];

        let tolerance = CompressSettings::default().rotation_tolerance;
        for (index, value) in original.iter().enumerate() {
            let error = <[f32; 4] as TrackValue>::error(*value, reduced.sample(times[index]));
            assert!(error <= tolerance + 1e-4, "error {error} at key {index}");
        }
    }

    #[test]
    fn sampling_clamps_outside_the_track() {
        let track = linear_position_track(3, 1.0);
        assert_eq!(track.sample(-1.0), track.values[0]);
        assert_eq!(track.sample(9.0), track.values[2]);
    }

    #[test]
    fn slerp_of_a_degenerate_quaternion_does_not_produce_nan() {
        let blended = slerp([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], 0.5);
        assert!(blended.iter().all(|value| value.is_finite()));
    }
}
