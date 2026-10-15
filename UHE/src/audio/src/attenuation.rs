//! How loud a sound is from how far away it is.
//!
//! One pure function, [`Attenuation::gain_at`], from a distance in world units
//! to a linear gain in `[0, 1]`. No device, no listener, no clock -- which is
//! the reason it is its own file rather than a method on a playing sound.
//!
//! ## The shape, and why it has those endpoints
//!
//! Every model here is written as a function of `t`, the normalised position in
//! the distance range:
//!
//! ```text
//! t = (distance - min) / (max - min)      clamped to [0, 1]
//! ```
//!
//! and every one of them is **exact at both endpoints**: `gain(min) == 1.0` and
//! `gain(max) == 0.0`, for every falloff, for every rolloff. That is a design
//! choice and it is the load-bearing one.
//!
//! The physically obvious law -- inverse distance, `1/d` -- fails both
//! endpoints. It is `inf` at zero, so a sound at the listener is undefined; and
//! it has no finite distance at which it goes silent, so a level-based cull can
//! never be built on it. Games do not want the physical law, they want a curve
//! with a full-volume radius and a cut-off radius, because those are the two
//! numbers an artist can reason about and the two numbers a culler can use.
//!
//! So `t` first, falloff second, and both endpoints exact.
//!
//! ## Why `min_distance` is a full-volume radius and not a reference distance
//!
//! `1/d` normalised to be 1.0 at `min` still has infinite slope at `min`: move
//! one centimetre and the gain drops by a fifth. Games clamp, and clamping to
//! unity inside `min` gives every emitter a small sphere of *constant* loudness
//! around it. That is the behaviour you want for the sounds that carry
//! positional information -- footsteps, a reload, a door -- because the
//! information in "it is at my feet" is not "it is 40% as loud as it was". It
//! is also why the default `min_distance` is 1.0: one world unit is usually one
//! metre, and one metre of constant loudness is about the size of a thing that
//! is "on" the player.
//!
//! ## Why the range is a range and not a point
//!
//! `max <= min` is a misconfiguration, and it is not an error to report at
//! runtime on every frame of every voice. It means the author never configured
//! a distance, so [`Attenuation::gain_at`] returns unity: the sound is audible,
//! and the problem is visible and audible instead of being a sound that silently
//! vanishes. [`Attenuation::NONE`] is the explicit way to say the same thing.

use crate::gain::{sanitise_delta, UNITY};

/// How gain decays between [`Attenuation::min_distance`] and
/// [`Attenuation::max_distance`].
///
/// All four are the same normalised curve with a different shape between the
/// endpoints, so switching between them never changes whether a sound is
/// audible at a given distance -- only how quickly it goes away.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Falloff {
    /// `rolloff`-weighted hyperbolic falloff. At `rolloff = 1` this is
    /// `(1 - t) / (1 + t)`: gentle near the listener, steep past half range,
    /// and the curve most engines ship as "inverse distance".
    ///
    /// `rolloff = 0` collapses to [`Falloff::Linear`], which is a useful sanity
    /// check and not a special case.
    #[default]
    InverseDistance,
    /// `1 - t`. Straight line in distance. Sounds like a physical object
    /// receding, and sounds *wrong* for one, which is why it is not the
    /// default.
    Linear,
    /// `(1 - t)²`. Aggressive; useful for a source that should be effectively
    /// gone before it reaches `max_distance`, such as a footstep.
    LinearSquared,
    /// No distance dependence at all. A 3D sound on this curve pans but does
    /// not fade, which is the right model for an engine-wide announcement that
    /// happens to be positioned.
    None,
}

/// A distance-to-gain curve.
///
/// Cheap to copy and to store per sound, which is why it is a value rather than
/// something the device owns: the whole point is that `wire` can evaluate the
/// curve on a sound with no device present.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Attenuation {
    falloff: Falloff,
    rolloff: f32,
    min_distance: f32,
    max_distance: f32,
}

impl Default for Attenuation {
    /// One metre of full volume, gone by thirty. The numbers every engine
    /// starts at, and the numbers that are right far more often than they are
    /// tuned away.
    fn default() -> Self {
        Attenuation {
            falloff: Falloff::default(),
            rolloff: 1.0,
            min_distance: 1.0,
            max_distance: 30.0,
        }
    }
}

impl Attenuation {
    /// Full-volume out to `min_distance`, silent from `max_distance` out.
    ///
    /// The arguments are stored as given and are *not* clamped; [`Self::gain_at`]
    /// is defined for any pair, and the degenerate cases are documented there
    /// rather than hidden here. Clamping in the constructor would mean
    /// `Attenuation::new(f32::NAN, f32::NAN)` quietly becomes a working
    /// attenuation instead of an obviously broken one.
    pub fn new(min_distance: f32, max_distance: f32) -> Self {
        Attenuation {
            falloff: Falloff::default(),
            rolloff: 1.0,
            min_distance,
            max_distance,
        }
    }

    /// A curve that never fades. See [`Falloff::None`].
    pub const NONE: Self = Attenuation {
        falloff: Falloff::None,
        rolloff: 1.0,
        min_distance: 0.0,
        max_distance: f32::INFINITY,
    };

    /// Choose the curve between the endpoints.
    #[must_use]
    pub fn falloff(mut self, falloff: Falloff) -> Self {
        self.falloff = falloff;
        self
    }

    /// Weight the [`Falloff::InverseDistance`] curve.
    ///
    /// `0.0` is the flattest curve that still reaches zero, `1.0` the default,
    /// and larger values make the sound collapse toward `min_distance`. Negative
    /// and non-finite values are floored to `0.0` rather than rejected, because
    /// a rolloff read out of a config file should not be able to make the curve
    /// rise with distance.
    #[must_use]
    pub fn rolloff(mut self, rolloff: f32) -> Self {
        self.rolloff = if rolloff.is_finite() {
            rolloff.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// The radius inside which nothing attenuates.
    pub fn min_distance(&self) -> f32 {
        self.min_distance
    }

    /// The radius at which the sound is silent.
    pub fn max_distance(&self) -> f32 {
        self.max_distance
    }

    /// Which curve this is, for diagnostics and for tests that assert a curve
    /// is the one you think it is.
    pub fn curve(&self) -> Falloff {
        self.falloff
    }

    /// The linear gain for a sound `distance` world units from the listener.
    ///
    /// Total by construction. The cases, and what each one means:
    ///
    /// | input | result | why |
    /// |---|---|---|
    /// | `NaN` | `0.0` | a caller bug. Silence is loud enough to notice; a `NaN` gain poisons the mix and is not. |
    /// | `-` or `0.0` | `1.0` | coincident with the listener, which is inside `min_distance` |
    /// | `<= min_distance` | `1.0` | the full-volume radius |
    /// | `>= max_distance` | `0.0` | the cut-off, exact, for every curve |
    /// | `inf` | `0.0` | via the cut-off, and this is the answer "as far away as possible" wants |
    /// | non-finite range | `1.0` | misconfigured; audible beats vanished |
    /// | `max <= min` | `1.0` | an empty range means no distance was configured |
    pub fn gain_at(&self, distance: f32) -> f32 {
        if !distance.is_finite() {
            // `inf` is a legitimate "unreachable" distance and wants silence.
            // `NaN` is a bug and gets the same answer, deliberately.
            return 0.0;
        }

        let distance = sanitise_delta(distance);

        if self.falloff == Falloff::None {
            return UNITY;
        }
        if !self.min_distance.is_finite() || !self.max_distance.is_finite() {
            return UNITY;
        }
        let span = self.max_distance - self.min_distance;
        if span <= 0.0 {
            return UNITY;
        }

        let t = ((distance - self.min_distance) / span).clamp(0.0, 1.0);
        match self.falloff {
            Falloff::None => UNITY,
            Falloff::Linear => 1.0 - t,
            Falloff::LinearSquared => {
                let falloff = 1.0 - t;
                falloff * falloff
            }
            Falloff::InverseDistance => (1.0 - t) / (1.0 + self.rolloff * t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_distance_is_full_volume() {
        assert_eq!(Attenuation::default().gain_at(0.0), 1.0);
    }

    #[test]
    fn infinite_distance_is_silent() {
        assert_eq!(Attenuation::default().gain_at(f32::INFINITY), 0.0);
    }

    #[test]
    fn half_distance() {
        let a = Attenuation::new(0.0, 10.0).falloff(Falloff::Linear);
        assert!((a.gain_at(5.0) - 0.5).abs() < 1e-6);
    }
}
