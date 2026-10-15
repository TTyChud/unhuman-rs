//! How to start a sound.
//!
//! Plain data with a builder, so a call site reads as the two or three things it
//! actually changes:
//!
//! ```no_run
//! # use wire::{PlayInstanceParams, Bus};
//! # fn demo() {
//! let params = PlayInstanceParams::default()
//!     .with_bus(Bus::Sfx)
//!     .with_volume(0.8)
//!     .at_position(glam::Vec3::new(2.0, 0.0, -4.0));
//! # let _ = params;
//! # }
//! ```
//!
//! Every field has a default that means "do nothing surprising". `Bus::Sfx`
//! because most engine sounds are effects and the ones that are not are the
//! ones an author has to think about; volume and pitch at 1.0 because that is
//! unmodified playback; 2D because a position is something you opt into.
//!
//! The C++ takes `(filepath, volume, loop)` positionally (`AudioEngine.h:16`),
//! which is fine at three parameters and falls apart at six: adding pitch means
//! every call site changes, and a caller who wants pitch has to pass `true` for
//! loop to reach the argument after it.

use glam::Vec3;

use crate::attenuation::Attenuation;
use crate::bus::Bus;
use crate::gain::{sanitise_gain, UNITY};

/// How to start a sound.
///
/// Every field has a default that means "do nothing surprising", and the builder
/// exists so a call site reads as the two or three things it actually changes.
/// The C++ takes `(filepath, volume, loop)` positionally
/// (`AudioEngine.h:16`) which means adding pitch means every call site changes
/// and a caller who wants pitch must pass `true` for loop to get at it.
#[derive(Clone, PartialEq, Debug)]
pub struct PlayInstanceParams {
    pub(crate) bus: Bus,
    pub(crate) volume: f32,
    pub(crate) pitch: f32,
    pub(crate) looping: bool,
    pub(crate) position: Option<Vec3>,
    pub(crate) attenuation: Attenuation,
}

impl Default for PlayInstanceParams {
    fn default() -> Self {
        PlayInstanceParams {
            bus: Bus::Sfx,
            volume: UNITY,
            pitch: UNITY,
            looping: false,
            position: None,
            attenuation: Attenuation::default(),
        }
    }
}

impl PlayInstanceParams {
    /// Route this sound to a bus. Defaults to [`Bus::Sfx`], which is right for
    /// the majority of engine sounds and wrong for exactly the ones an author
    /// has to think about.
    #[must_use]
    pub fn with_bus(mut self, bus: Bus) -> Self {
        self.bus = bus;
        self
    }

    /// This sound's own volume, *before* its bus and the master. Allowed above
    /// unity, which is how a sound is louder than the rest of its category
    /// without moving the category.
    #[must_use]
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = sanitise_gain(volume);
        self
    }

    /// Playback rate, which is also pitch. `2.0` is an octave up and half as
    /// long; `0.5` is an octave down and twice as long. Below `0.0` is
    /// meaningless and is treated as silence rather than as reverse playback,
    /// because a reverse-played sound is a feature nobody has asked for.
    #[must_use]
    pub fn with_pitch(mut self, pitch: f32) -> Self {
        self.pitch = if pitch.is_finite() {
            pitch.max(0.0)
        } else {
            UNITY
        };
        self
    }

    /// Restart when the sound reaches its end. A looping sound is never
    /// finished by [`SoundInstance::advance`]; it stops through
    /// [`crate::AudioEngine::stop`] like anything else.
    #[must_use]
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Make this a 3D sound at a world position, or leave it 2D.
    ///
    /// The attenuation curve comes from [`Self::with_attenuation`]; a 3D sound
    /// with the default curve is one metre of full volume gone by thirty, which
    /// is the assumption every engine sound starts with.
    #[must_use]
    pub fn at_position(mut self, position: Vec3) -> Self {
        self.position = Some(position);
        self
    }

    /// Override the distance curve for this sound.
    #[must_use]
    pub fn with_attenuation(mut self, attenuation: Attenuation) -> Self {
        self.attenuation = attenuation;
        self
    }
}
