//! `wire` — audio.
//!
//! A listener, a set of sounds with lifetimes, buses, and one seam to a device.
//!
//! ## What the C++ had, and what this is instead
//!
//! Upstream's audio is `AudioEngine` (a static pass-through) over
//! `MiniAudioBackend` (miniaudio, a global `ma_engine` and a global
//! `std::vector<ma_sound*>`) — 4 public functions, about 130 lines with the
//! backend. What it has never had, according to its own design doc
//! (`docs/architecture/assets-and-audio.md`): any control surface, any mixing,
//! any thread safety, any reuse of decoded audio, any format beyond WAV.
//!
//! **One correction to our own survey, because it changes what parity means
//! here.** `docs/TRANSFER.md` and `docs/PARITY.md` both record that
//! `PlaySound2D`, `PlaySound3D` and `SetListenerPosition` have "zero call sites
//! outside `Audio/`", and that `Update()` "reaps a permanently empty vector".
//! That is wrong. `UHEGAME/src/AimLabLayer.cpp` calls `PlaySound3D` three times
//! (`:215`, `:255`, `:281`) and `SetListenerPosition` once per frame (`:316`)
//! with the camera's position, forward and up; `UHE/src/UHE/Core/Application.cpp`
//! calls `Init` (`:39`), `Shutdown` (`:50`) and `Update` (`:126`); and `UHEGAME`
//! is a built target (`CMakeLists.txt:363`), not an unbuilt directory. So:
//!
//! - `PlaySound2D` genuinely has no call sites. It is dead.
//! - `PlaySound3D` and `SetListenerPosition` are **wired and in use** — the only
//!   spatial audio in the project, and it works.
//! - `Update()` does **not** reap a permanently empty vector. `s_ActiveSounds`
//!   fills with every gunshot and reload and is swept every frame.
//! - The real defects are the ones the design doc lists, and the expensive one
//!   comes first: `ma_sound_init_from_file` on *every* play
//!   (`MiniAudioBackend.cpp:98-101`), so every shot the player fires re-decodes
//!   a WAV from disk on the frame the trigger was pulled.
//!
//! So parity here is not "copy 130 lines of miniaudio". It is "keep the three
//! things that work — 2D playback, 3D playback, a per-frame listener — and add
//! the control surface the C++ never had". [`AudioEngine::play_2d`],
//! [`AudioEngine::play_3d`] and [`AudioEngine::set_listener`] *are* the C++'s API
//! surface, same arguments, same defaults. [`PlayHandle`] is the thing it
//! lacks, and so is every other method on [`AudioEngine`].
//!
//! ## The layout, and the question each file answers
//!
//! The split is by concern, and the concern that matters is **which of these
//! can be tested with no sound card**. Everything except the last row can, and
//! everything except the last row is.
//!
//! | module | concern | needs a device? |
//! |---|---|---|
//! | [`gain`] | the NaN policy every gain function shares | no |
//! | [`attenuation`] | distance to gain. Pure. | no |
//! | [`pan`] | direction to stereo gains. Pure. | no |
//! | [`listener`] | the frame of reference a direction is measured in. Pure. | no |
//! | [`bus`] | where a sound is routed | no |
//! | [`play_params`] | how to start a sound | no |
//! | [`instance`] | a sound that is playing | no |
//! | [`mixer`] | bus volumes, the master fader, the per-frame mix | no |
//! | [`voice_table`] | handle identity: slot, generation, staleness | no |
//! | [`engine`] | the façade that owns the rest | no |
//! | [`device`] | the [`AudioDevice`] seam | the trait only |
//! | [`headless`] | a device that opens nothing | no |
//! | [`rodio_device`] | rodio | **yes** |
//!
//! That is more files than the C++ has translation units, and deliberately: the
//! C++'s four-file split is four files because four files was all the code needed.
//! This split is by *testability*, so a piece of pure arithmetic is its own file
//! precisely so that it can be exercised on a runner with no audio hardware.
//! [`gain`], [`bus`], [`play_params`] and [`voice_table`] each hold one noun and
//! exist so the files that hold the interesting arithmetic stay under the 300
//! lines `docs/TRANSFER.md` §3 sets.
//!
//! ## Headless is a supported mode, not a test trick
//!
//! There is no audio device in CI. If the maths sat behind the device, the tests
//! would have nothing to assert on and would pass by measuring nothing — which
//! has happened in this codebase before, with Vulkan validation. So the device
//! is behind a trait, the attenuation and the panning are plain functions of
//! `f32`, and [`HeadlessDevice`] implements the trait by remembering everything.
//!
//! ```no_run
//! use wire::{AudioEngine, Bus, PlayInstanceParams};
//!
//! // The whole engine, with no sound hardware. This is a supported build.
//! let mut audio = AudioEngine::headless();
//! audio.set_master_volume(0.5);
//! audio.set_bus_volume(Bus::Music, 0.3);
//! audio.play_2d("footstep.wav", PlayInstanceParams::default())?;
//! audio.update(1.0 / 60.0);
//! # Ok::<(), wire::Error>(())
//! ```
//!
//! ## The mix is control-rate, not sample-rate
//!
//! Read [`mixer`] before assuming this sums samples. It does not, and it must
//! not: a sample sum on the frame thread is a resampler, a clock and a ring
//! buffer, which is reimplementing rodio and would put a 48 kHz stereo mix on
//! the thread trying to hold 120 fps. Once per frame, per voice, this crate
//! decides a gain; the device multiplies by it at the sample rate. FMOD and
//! Wwise do the same thing.
//!
//! The consequence worth knowing: **pan and attenuation update at frame rate.**
//! A source sweeping past the camera can be heard to step between stereo
//! positions on a slow machine. The fix is to run the same [`listener`] maths
//! inside the audio thread, which rodio's `source::periodic` supports and which
//! [`rodio_device`] already uses to push pans — but then the maths would no
//! longer be testable without a device, which is the trade this crate makes the
//! other way on purpose.
//!
//! ## Threading
//!
//! [`AudioDevice`] is `Send` and not `Sync`, and every method takes `&mut self`
//! except the spec. An [`AudioEngine`] can be moved to a dedicated audio thread
//! and driven from there, but cannot be driven from two threads at once without
//! the owner synchronising. See [`engine`] for why that is a contract and not an
//! enforcement.
//!
//! ## Not implemented
//!
//! Each of these is a real gap, listed so "not there" is not read as "not
//! thought about".
//!
//! - **A shared decoded-sound cache.** [`rodio_device`] caches by path, which
//!   fixes the C++'s re-decode-per-shot for any one process, but the target
//!   design puts sound lifetime in `vault`'s content-addressed registry and that
//!   is not wired.
//! - **Ducking.** Music does not duck under a voice line. It needs a second
//!   parallel bus summing into the first, which is a different shape from the
//!   two levels in [`bus`] and is the first thing to add there.
//! - **HRTF.** Two speakers physically cannot distinguish a source in front from
//!   one behind. [`pan::rear_attenuation`] is the cheap level cue games ship
//!   instead; real binaural rendering is a per-ear convolver and needs a
//!   measured impulse-response set.
//! - **Streaming.** Long music is decoded whole into memory. rodio defaults
//!   cover WAV, FLAC, MP3, MP4 and Vorbis, where the C++ compiles MP3 and FLAC
//!   *out* (`MA_NO_MP3`, `MA_NO_FLAC` — a GCC `-O0` SSE4.1 workaround) and so
//!   supports WAV only.
//! - **Microphone input.** rodio has it; nothing here uses it.
//! - **ECS components.** An `AudioSourceComponent` on an emitter and an
//!   `AudioListenerComponent` on the camera are what upstream's design doc
//!   wants, and they belong to `meats` and the composition root rather than to
//!   a library that has no business knowing what an entity is.

#![deny(missing_docs)]

pub mod attenuation;
pub mod bus;
pub mod device;
pub mod engine;
pub mod error;
pub mod gain;
pub mod headless;
pub mod instance;
pub mod listener;
pub mod mixer;
pub mod pan;
pub mod play_params;
pub mod rodio_device;
mod voice_control;
pub mod voice_table;

pub use attenuation::{Attenuation, Falloff};
pub use bus::Bus;
pub use device::{AudioDevice, DeviceSpec, PlayRequest, SoundInfo};
pub use engine::AudioEngine;
pub use error::{Error, Result};
pub use gain::sanitise_gain;
pub use headless::{HeadlessDevice, HeadlessVoice};
pub use instance::{InstanceState, PlayHandle, SoundInstance};
pub use listener::{Listener, SpatialSample};
pub use mixer::{Mixer, VoiceMix};
pub use pan::{rear_attenuation, stereo_pan, StereoPan, REAR_FLOOR};
pub use play_params::PlayInstanceParams;
pub use rodio_device::RodioDevice;
