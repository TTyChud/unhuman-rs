//! Where a sound is in space, and how loud it is from where the listener is.
//!
//! These two files hold **all** of the spatial arithmetic in `wire`, and neither
//! of them can name a device, a file, or a clock. That is the whole point of
//! splitting them out: attenuation and panning are the parts of audio that are
//! actually wrong in games, and they are the parts that cannot be tested on a
//! build machine with no sound card if they are entangled with playback.
//!
//! ## One rule they both follow: a NaN must never reach the mixer
//!
//! The mixer *sums*. One `NaN` sample makes the running sum `NaN` for the rest
//! of the buffer, and a stream of `NaN` samples leaves the audio device
//! outputting garbage with no error anywhere. Every function here is therefore
//! **total**: it is defined for every `f32`, including `NaN`, `inf` and
//! negative input, and it returns a finite gain rather than propagating the
//! poison. `inf` distance means "gone", which is the right answer and is also
//! what falls out of the arithmetic; `NaN` distance means the caller has a bug,
//! and the right answer there is silence because silence is loud enough to
//! investigate and `NaN` is not.
//!
//! `sanitise_gain` is that policy in one place, because "clamp, then handle
//! non-finite" repeated in six files is six chances to get it subtly different.

/// `pi / 4`, the half-width of the equal-power panning wedge.
///
/// `1.0` rather than `0.0`, because a non-finite gain is nearly always a
/// multiply-by-uninitialised or a divide-by-zero somewhere upstream, and
/// silence makes that bug invisible while unity at least keeps the mix honest
/// and lets the caller hear that something is wrong with the right sound.
pub const UNITY: f32 = 1.0;

/// Coerce a caller-supplied gain into something that can be multiplied into a
/// mix without poisoning it.
///
/// Negative gains are clamped to zero rather than inverted: a negative volume
/// is a sign error somewhere upstream, and playing the waveform upside down is
/// a far worse failure mode than playing nothing. The upper bound is
/// deliberately **not** 1.0 -- a per-sound pre-gain above unity is legitimate
/// headroom and a bus fader above unity is not, which is why
/// [`crate::mixer::Mixer`] applies its own clamp and this function does not.
pub fn sanitise_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.max(0.0)
    } else {
        UNITY
    }
}

/// Coerce a value into `[-1, 1]`, mapping `NaN` to `0.0`.
///
/// Used for anything that arrives as a dot product of two unit vectors. The
/// clamp is belt-and-braces -- dot products of unit vectors are already in
/// range -- and the `NaN` arm is not, because a non-finite direction must not
/// become a non-finite pan.
pub(crate) fn clamp_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Below this, a squared length is treated as zero and a direction is undefined.
///
/// Not `f32::EPSILON`: that is the smallest *denormal-ish* step and comparing a
/// squared length against it means a genuinely tiny-but-real vector normalises
/// fine, whereas a length of exactly `0.0` is the only thing that actually
/// fails to normalise. `1e-12` squared is `1e-6` units, far below any distance
/// a game world distinguishes, and comfortably above the squared lengths of
/// uninitialised memory.
pub(crate) const DEGENERATE: f32 = 1e-12;

/// A distance, pitch or time that must be finite and non-negative.
///
/// `dt` from a frame loop is the classic source: a `NaN` from a division by a
/// zero timestep arrives here, and it must not become a `NaN` elapsed time that
/// makes every voice look finished forever after.
pub(crate) fn sanitise_delta(delta: f32) -> f32 {
    if delta.is_finite() {
        delta.max(0.0)
    } else {
        0.0
    }
}
