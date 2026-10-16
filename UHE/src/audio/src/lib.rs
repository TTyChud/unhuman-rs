//! Audio: the mixer, the backend, spatialisation.
//!
//! Ported from `UHE/src/UHE/Audio/`, where `AudioEngine` was a facade of statics
//! forwarding to `MiniAudioBackend`, which kept `ma_engine` and a
//! `std::vector<ma_sound*> s_ActiveSounds` in file scope. Here [`AudioEngine`] owns
//! the voices and the clip table, [`backend::Backend`] owns the rodio device, and
//! [`WirePlugin`] installs the engine into `Engine::audio`.
//!
//! Two behaviours differ from the C++ because the C++ left them to a library
//! default. Attenuation is linear over the source's `range`: miniaudio's default
//! inverse falloff with `maxDistance = FLT_MAX` only reaches silence at infinity, so
//! a spatial source had no volume it could be inaudible at. And the number of
//! simultaneous voices is bounded, where `s_activeSounds` grew until a sound ended.

pub mod backend;
pub mod engine;
pub mod sound;
pub mod spatial;
pub mod voice;

/// The listener pose and the voice handle are `adam`'s: they cross the
/// `AudioBackend` seam, and a second definition here would only be a second thing
/// to convert between.
pub use adam::{Listener, VoiceId};

pub use engine::{listener_from, AudioEngine, WirePlugin, DEFAULT_VOICES};
pub use sound::{Bus, Sound, SoundId, SoundTable};
pub use spatial::{attenuation, channel_gains, pan, Pan, PanHandle, MIN_CHANNEL_GAIN};
pub use voice::{Voice, VoicePool, VoiceSpec, DEFAULT_RANGE};
