//! A sound that is playing: its handle, its state, its volume and its pitch.
//!
//! ## What a handle is for, given that the C++ has none
//!
//! `MiniAudioBackend::PlaySound3D` returns `void`
//! (`MiniAudioBackend.cpp:94`). There is no way to stop a sound, pause it, fade
//! it out, ask whether it is still playing, or find out that a decode failed.
//! `AimLabLayer.cpp:215` plays a reload and the engine has no handle to that
//! reload -- so a reload cannot be cut short when the player cancels it, and
//! cannot be found at all once the vector is swept.
//!
//! [`PlayHandle`] is that missing piece, and it is an index plus a generation
//! rather than a bare index. A bare index is wrong the moment a slot is reused:
//! the sound that was slot 3 finished, slot 3 was given a gunshot, and the
//! reload handle the game is still holding now silently stops the gunshot.
//! The generation makes the stale handle simply *not match*, which is why
//! [`PlayHandle::is_same_sound`] exists and why every handle-taking method on
//! [`crate::AudioEngine`] reports whether it did anything.
//!
//! Upstream's own design doc reaches the same conclusion independently, with
//! `struct VoiceHandle { u32 index; u32 generation; }` in
//! `docs/architecture/assets-and-audio.md`.
//!
//! ## Two kinds of sound, and why they are one type
//!
//! A 2D sound has no position and therefore no attenuation and no pan; a 3D one
//! has both. That is a single `Option<Vec3>`, not two types, because the C++
//! has two functions where one would do (`PlaySound2D` and `PlaySound3D` are
//! byte-identical except for three lines) and because a game genuinely moves a
//! sound between the two -- a looping ambience that becomes positional when the
//! player walks into the room it is in -- which two types cannot express without
//! a conversion that has to invent the attenuation the new type wanted.
//!
//! ## The clock is the authority on when a sound ends
//!
//! [`SoundInstance::advance`] is what retires a one-shot. The device also gets
//! asked, and either answer stops the sound, but the engine's own clock is what
//! it is designed around. That is because the C++'s only cleanup is
//! `MiniAudioBackend::Update()`, which is called from one place in one binary
//! (`Application.cpp:126`) and is skipped entirely when the window is
//! minimised -- so a sound that finishes while the window is minimised is never
//! swept, and `MiniAudioBackend::Shutdown` is the only thing that frees it.

use glam::Vec3;

use crate::attenuation::Attenuation;
use crate::bus::Bus;
use crate::gain::{sanitise_delta, sanitise_gain, UNITY};
use crate::play_params::PlayInstanceParams;

/// A stable reference to one playing sound.
///
/// `Copy`, small, and meaningless on its own -- it is only meaningful with a
/// [`crate::AudioEngine`], which owns the slot table. Two handles to different
/// sounds are different even when their indices match, because their
/// generations do not.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PlayHandle {
    index: u32,
    generation: u32,
}

impl PlayHandle {
    /// A handle for a slot and the generation that currently occupies it.
    ///
    /// Generation `0` is reserved: it is what a default-constructed value would
    /// be, and it is never handed out, so `PlayHandle { index: 3,
    /// generation: 0 }` cannot alias a live sound.
    pub fn new(index: u32, generation: u32) -> Self {
        PlayHandle {
            index,
            generation: generation.max(1),
        }
    }

    /// The slot this handle names.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Which occupant of that slot this handle refers to.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Whether this handle refers to `other`'s occupant of the same slot.
    ///
    /// False means the slot has been reused and this handle is stale. Read it
    /// as "do these name the same sound", which is the only question that
    /// matters when a caller is about to stop the wrong one.
    pub fn is_same_sound(&self, other: &PlayHandle) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

/// What a sound is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InstanceState {
    /// Producing samples, or about to.
    #[default]
    Playing,
    /// Held at its current position. Resumable with
    /// [`SoundInstance::resume`].
    Paused,
    /// Finished, stopped, or cancelled. Terminal, and reclaimed at the next
    /// [`crate::AudioEngine::update`].
    Stopped,
}

impl InstanceState {
    /// Whether the sound is over and its slot can be reused.
    pub fn is_terminal(self) -> bool {
        self == InstanceState::Stopped
    }
}

/// One playing sound, as the engine sees it.
///
/// Holds no samples and touches no device. Everything here is bookkeeping the
/// mixer needs in order to decide what the device should be told, which is what
/// makes it constructible in a test.
#[derive(Clone, PartialEq, Debug)]
pub struct SoundInstance {
    handle: PlayHandle,
    params: PlayInstanceParams,
    state: InstanceState,
    elapsed: f32,
    duration: Option<f32>,
}

impl SoundInstance {
    /// A playing sound with the given duration.
    ///
    /// `duration` is `None` when the device could not say -- a stream, or a
    /// decoder that does not report one. Such a sound never self-finishes and
    /// relies on the device to report that it ended, which is the honest
    /// arrangement: guessing a duration and stopping the sound early is worse
    /// than not knowing.
    pub fn new(handle: PlayHandle, params: PlayInstanceParams, duration: Option<f32>) -> Self {
        SoundInstance {
            handle,
            params,
            state: InstanceState::Playing,
            elapsed: 0.0,
            duration: duration.filter(|d| d.is_finite() && *d > 0.0),
        }
    }

    /// Which sound this is.
    pub fn handle(&self) -> PlayHandle {
        self.handle
    }

    /// Its bus.
    pub fn bus(&self) -> Bus {
        self.params.bus
    }

    /// Its own volume, before its bus and the master.
    pub fn volume(&self) -> f32 {
        self.params.volume
    }

    /// Change its own volume mid-flight. This is the C++'s missing `SetVolume`.
    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume = sanitise_gain(volume);
    }

    /// Its playback rate and therefore its pitch.
    pub fn pitch(&self) -> f32 {
        self.params.pitch
    }

    /// Change its pitch mid-flight. Pitch changes the *duration* of a one-shot
    /// as it goes: doubling the pitch halves how long it takes to finish, and
    /// the clock here follows the pitch because a sound played at 2x that is
    /// retired on wall-clock time stops halfway through.
    pub fn set_pitch(&mut self, pitch: f32) {
        self.params.pitch = if pitch.is_finite() {
            pitch.max(0.0)
        } else {
            UNITY
        };
    }

    /// Whether it restarts at its end.
    pub fn is_looping(&self) -> bool {
        self.params.looping
    }

    /// Whether it is 3D.
    pub fn is_spatial(&self) -> bool {
        self.params.position.is_some()
    }

    /// Its world position, or `None` if it is a 2D sound.
    pub fn position(&self) -> Option<Vec3> {
        self.params.position
    }

    /// Move it, or make a 2D sound 3D (or the reverse) by passing `None`.
    pub fn set_position(&mut self, position: Option<Vec3>) {
        self.params.position = position;
    }

    /// Its distance curve.
    pub fn attenuation(&self) -> Option<Attenuation> {
        self.is_spatial().then_some(self.params.attenuation)
    }

    /// How long it has been playing, in seconds of *sound* rather than of wall
    /// clock -- see [`Self::set_pitch`].
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    /// Its length in seconds, if the device said.
    pub fn duration(&self) -> Option<f32> {
        self.duration
    }

    /// How far through it is, in `[0, 1]`. `None` for a looping sound, whose
    /// progress is not meaningful, and for a sound of unknown length.
    pub fn progress(&self) -> Option<f32> {
        if self.is_looping() {
            return None;
        }
        let duration = self.duration?;
        Some((self.elapsed / duration).clamp(0.0, 1.0))
    }

    /// What it is doing.
    pub fn state(&self) -> InstanceState {
        self.state
    }

    /// Whether it is playing right now, i.e. not paused and not over.
    pub fn is_playing(&self) -> bool {
        self.state == InstanceState::Playing
    }

    /// Hold it where it is. Not present in the C++ at all.
    pub fn pause(&mut self) {
        if self.state == InstanceState::Playing {
            self.state = InstanceState::Paused;
        }
    }

    /// Let it play again. Does not restart it -- resume is not replay, and
    /// conflating them is how a paused sound ends up at the wrong sample.
    pub fn resume(&mut self) {
        if self.state == InstanceState::Paused {
            self.state = InstanceState::Playing;
        }
    }

    /// End it. Terminal.
    pub fn stop(&mut self) {
        self.state = InstanceState::Stopped;
    }

    /// Advance its clock by `dt` seconds.
    ///
    /// Returns whether it has finished. A looping sound never finishes, a sound
    /// of unknown length never finishes, and a paused one does not advance --
    /// so `dt` here is the frame time the sound was *actually* playing for.
    pub fn advance(&mut self, dt: f32) -> bool {
        if self.state != InstanceState::Playing {
            return false;
        }
        let pitch = self.params.pitch;
        if pitch <= 0.0 {
            // A zero-pitch sound makes no progress and never ends. Retiring it
            // is the only answer that does not leave a voice alive forever.
            self.state = InstanceState::Stopped;
            return true;
        }
        self.elapsed += sanitise_delta(dt) * pitch;
        if let Some(duration) = self.duration {
            if !self.params.looping && self.elapsed >= duration {
                self.state = InstanceState::Stopped;
                return true;
            }
        }
        false
    }

    /// Whether this sound has run out. Does not advance the clock.
    pub fn is_finished(&self) -> bool {
        self.state.is_terminal()
    }
}
