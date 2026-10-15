//! Which ear a sound is closer to, and the front/back cue.
//!
//! Two independent pure functions. They are separate because they answer
//! different questions and because one of them is the honest answer to a
//! question the other one cannot answer.
//!
//! ## Equal-power panning, and why not linear
//!
//! [`stereo_pan`] is the MPEG pan law: with `lateral` in `[-1, 1]`,
//!
//! ```text
//! right = sin((lateral + 1) * pi/4)
//! left  = sin((1 - lateral) * pi/4)
//! ```
//!
//! Two speakers reproduce one channel, so a sound panned dead centre has to be
//! played at `1/sqrt(2)` on both or it is 3 dB louder than panned hard left --
//! a level jump that makes a sound "pop" to the side as it moves past you. The
//! `sin`/`sin` law is constant-power: `left² + right² == 1` at every position,
//! so a sound crossing the listener's head does not change loudness.
//!
//! Linear panning (`1 - x`, `1 + x`) is a vertex on a triangle instead of a
//! point on a circle: cheaper, and it loses 3 dB in the middle. It is here as
//! [`stereo_pan_linear`] for A/B comparison and because the C++ -- which does
//! not pan at all, see `MiniAudioBackend.cpp` -- is the baseline this replaces.
//!
//! ## Why there is a separate front/back term, and what it cannot fix
//!
//! Two speakers are physically incapable of distinguishing a source directly in
//! front of the listener from one directly behind. Both have `lateral == 0`,
//! both pan to centre, and no amount of gain law fixes it. The real fix is
//! HRTF -- per-ear filtering with the shadowing of the head, which is what
//! OpenAL's `HRTF` extension and miniaudio's binaural mode are for. That is a
//! convolver per ear and it is deliberately **not** in this crate: it is a
//! filter design problem, not a maths problem, and it needs a measured
//! impulse-response set to do properly.
//!
//! What games ship instead is a level cue. The ear is *worse* at localising a
//! quiet source, so attenuating the rear hemisphere makes rear sources read as
//! "further away" and, in practice, as "behind", while front sources read as
//! near. [`rear_attenuation`] is that cue: flat at unity through the whole
//! front hemisphere, ramping to [`REAR_FLOOR`] directly behind. It is weak on
//! its own and this file should not pretend otherwise.
//!
//! ## The gain is folded in here, not applied twice
//!
//! [`StereoPan::scaled`] multiplies both channels by one number. The mix uses
//! it to carry *distance attenuation times* the rear cue, so the device applies
//! `pan.left` and `pan.right` as the per-channel gains and the scalar voice gain
//! separately. Applying the spatial term in both places would square it, which
//! is the kind of double-mapping that sounds like "the falloff seems too
//! aggressive" and takes an afternoon to find.

use crate::gain::{clamp_unit, sanitise_gain, UNITY};

/// How much quieter a source directly behind the listener is.
///
/// About `-3.1 dB`. Chosen for two reasons: it is audible as a *distance*
/// change, which is the only front/back cue a stereo pair has, and it is not
/// enough to sound like a mistake. Bigger values are the usual failure mode --
/// a rear source becomes obviously absent rather than obviously behind.
pub const REAR_FLOOR: f32 = 0.7;

/// A stereo position as a gain per channel.
///
/// Gains, not a `-1..1` pan position: this is what reaches the device, and the
/// device has no use for a position to convert back into a gain. Keeping it as
/// gains means the law that produced them is visible at the call site.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StereoPan {
    /// Gain for the left channel.
    pub left: f32,
    /// Gain for the right channel.
    pub right: f32,
}

impl StereoPan {
    /// The same power in both ears. What a 2D sound, a source dead ahead and a
    /// source dead behind all resolve to.
    pub const CENTRE: StereoPan = StereoPan {
        left: core::f32::consts::FRAC_1_SQRT_2,
        right: core::f32::consts::FRAC_1_SQRT_2,
    };

    /// Hard left.
    pub const LEFT: StereoPan = StereoPan {
        left: 1.0,
        right: 0.0,
    };

    /// Hard right.
    pub const RIGHT: StereoPan = StereoPan {
        left: 0.0,
        right: 1.0,
    };

    /// Nothing at all, for a sound out of range or explicitly muted.
    pub const SILENT: StereoPan = StereoPan {
        left: 0.0,
        right: 0.0,
    };

    /// Build from per-channel gains, clamping each to a usable gain.
    ///
    /// The clamp is not defensive noise. [`stereo_pan`] produces exactly
    /// `[0, 1]`, and this constructor also accepts the output of
    /// [`StereoPan::scaled`] and anything a caller wrote by hand -- and a
    /// negative channel gain is an inverted waveform, not a quiet one.
    pub fn new(left: f32, right: f32) -> Self {
        StereoPan {
            left: sanitise_gain(left),
            right: sanitise_gain(right),
        }
    }

    /// Both channels scaled by one gain. This is how a single spatial factor
    /// reaches two channels exactly once.
    pub fn scaled(self, gain: f32) -> Self {
        let gain = sanitise_gain(gain);
        StereoPan {
            left: self.left * gain,
            right: self.right * gain,
        }
    }

    /// Whether this is [`StereoPan::CENTRE`] to within `f32` noise.
    ///
    /// Exists for the "is this 2D after all" check and for tests. Not for
    /// production control flow: `f32` equality on a value that came through a
    /// multiply is never true enough to branch on.
    pub fn is_centred(self) -> bool {
        const EPSILON: f32 = 1e-6;
        (self.left - Self::CENTRE.left).abs() < EPSILON
            && (self.right - Self::CENTRE.right).abs() < EPSILON
    }
}

impl Default for StereoPan {
    fn default() -> Self {
        StereoPan::CENTRE
    }
}

/// Equal-power panning from a lateral position.
///
/// `lateral` is `dot(to_sound, listener_right)`, so **`0.0` is dead ahead,
/// `1.0` is hard right and `-1.0` is hard left**. `NaN` maps to centre, and
/// values outside `[-1, 1]` clamp -- see [`crate::gain::clamp_unit`] for why
/// that arm exists.
///
/// The constant-power property is exact, not approximate: for any `lateral`,
/// `left² + right² == 1`.
pub fn stereo_pan(lateral: f32) -> StereoPan {
    let lateral = clamp_unit(lateral);
    let quarter_pi = core::f32::consts::FRAC_PI_4;
    StereoPan {
        right: ((lateral + 1.0) * quarter_pi).sin(),
        left: ((1.0 - lateral) * quarter_pi).sin(),
    }
}

/// Linear panning from a lateral position. For comparison, not for shipping.
///
/// `left² + right²` falls to `0.5` at centre, which is the 3 dB dip the
/// equal-power law exists to avoid. Kept because "why does panning sound quieter
/// in the middle" is a question that is much easier to answer with both laws
/// visible than with one law and a citation.
pub fn stereo_pan_linear(lateral: f32) -> StereoPan {
    let lateral = clamp_unit(lateral);
    StereoPan {
        left: 1.0 - lateral.max(0.0),
        right: 1.0 + lateral.min(0.0),
    }
}

/// How much a source's frontality attenuates it.
///
/// `frontality` is `dot(to_sound, listener_forward)`: **`1.0` is dead ahead,
/// `0.0` is directly to either side, `-1.0` is directly behind**.
///
/// Returns [`UNITY`] across the entire front hemisphere and ramps linearly to
/// [`REAR_FLOOR`] directly behind. The kink at the sides is intentional: it
/// keeps the front half of the sphere untouched, which is the property that
/// makes this safe to leave on permanently -- a source in front of the player
/// is never made quieter for being in front of the player.
pub fn rear_attenuation(frontality: f32) -> f32 {
    let frontality = clamp_unit(frontality);
    if frontality >= 0.0 {
        return UNITY;
    }
    REAR_FLOOR + (UNITY - REAR_FLOOR) * (1.0 + frontality) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pans_left() {
        assert_eq!(stereo_pan(-1.0), StereoPan::LEFT);
    }

    #[test]
    fn pans_right() {
        assert_eq!(stereo_pan(1.0), StereoPan::RIGHT);
    }

    #[test]
    fn behind_is_quietest_cue() {
        assert!((rear_attenuation(-1.0) - REAR_FLOOR).abs() < 1e-6);
    }
}
