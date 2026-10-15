//! An audio device that opens nothing.
//!
//! ## Why this is a library type and not a test fixture
//!
//! Because it is not only for tests. A dedicated server, a headless simulation
//! of a game loop, and an integration test of gameplay code that happens to make
//! a noise all want to run the real engine with no audio. Handing them
//! `HeadlessDevice` means every one of those is *the same code path* with one
//! substituted object -- the listener maths, the attenuation, the bus
//! arithmetic and the voice lifecycle all execute, and every value they produce
//! is readable through [`HeadlessVoice`].
//!
//! It is the same reasoning as `eyes::HeadlessWindow`, and the C++ has neither.
//! `Application` is a singleton, so the C++ cannot run its frame loop at all
//! without a display server and an audio device, which is why upstream's audio
//! is untested and, being untested, why nobody noticed it re-decodes a gunshot
//! from disk every time one is fired.
//!
//! ## It is a fake, and a fake that lies is worse than no fake
//!
//! It does not produce samples, so it cannot prove that anything is audible.
//! What it can do is hold every value the engine asked for -- the gain, the
//! pan, the pitch, the state, the elapsed time -- and let a test assert on it.
//! That is enough to pin the arithmetic, which is where the bugs are, and it is
//! worth being explicit that it is not enough to pin the output.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::device::{AudioDevice, DeviceSpec, PlayRequest, SoundInfo};
use crate::gain::{sanitise_delta, sanitise_gain};
use crate::instance::InstanceState;
use crate::pan::StereoPan;
use crate::Result;

/// One voice, as a [`HeadlessDevice`] sees it.
///
/// Public and inspectable because the point of the headless device is that a
/// test can assert on what the engine asked for, rather than on what came out
/// of a speaker.
#[derive(Clone, PartialEq, Debug)]
pub struct HeadlessVoice {
    /// The slot this voice occupies.
    pub slot: u32,
    /// What it was asked to play.
    pub path: PathBuf,
    /// The scalar gain the engine last set.
    pub volume: f32,
    /// The playback rate the engine last set.
    pub pitch: f32,
    /// The per-channel gains the engine last set.
    pub pan: StereoPan,
    /// Whether it was opened as a 3D sound.
    pub spatial: bool,
    /// Whether it restarts at its end.
    pub looping: bool,
    /// What it is doing.
    pub state: InstanceState,
    /// How far through it is, in seconds.
    pub elapsed: f32,
    /// How long it is, if the device reported a length.
    pub duration: Option<f32>,
}

/// An [`AudioDevice`] that opens nothing, plays nothing, and remembers
/// everything.
///
/// This is what makes `wire` testable in CI. A machine with no sound card still
/// runs the whole engine: the listener, the attenuation, the panning, the bus
/// arithmetic and the voice lifecycle all execute, and every value they produce
/// is readable here.
///
/// It is not only for tests. A game server, a headless simulation, and an
/// integration test of gameplay code that happens to make a noise all want the
/// engine with no audio, and this is that: identical code paths, one
/// substituted device.
#[derive(Clone, Debug)]
pub struct HeadlessDevice {
    spec: DeviceSpec,
    voices: BTreeMap<u32, HeadlessVoice>,
    duration: Option<f32>,
    opened: u64,
    polls: u64,
}

impl Default for HeadlessDevice {
    fn default() -> Self {
        HeadlessDevice {
            spec: DeviceSpec {
                sample_rate: 48_000,
                channel_count: 2,
            },
            voices: BTreeMap::new(),
            duration: Some(1.0),
            opened: 0,
            polls: 0,
        }
    }
}

impl HeadlessDevice {
    /// A 48 kHz stereo device that reports every sound as one second long.
    pub fn new() -> Self {
        Self::default()
    }

    /// Change the length every opened sound reports.
    ///
    /// `None` makes every sound a voice that only the device can retire, which
    /// is the shape a stream has.
    #[must_use]
    pub fn with_duration(mut self, duration: Option<f32>) -> Self {
        self.duration = duration;
        self
    }

    /// Change the reported format.
    #[must_use]
    pub fn with_spec(mut self, spec: DeviceSpec) -> Self {
        self.spec = spec;
        self
    }

    /// One voice, or `None` if that slot is free.
    pub fn voice(&self, slot: u32) -> Option<&HeadlessVoice> {
        self.voices.get(&slot)
    }

    /// How many voices are open.
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// How many sounds have been opened over this device's life, including ones
    /// that have since been closed. The number a test asserts on to prove a
    /// decoded sound was *reused* rather than reopened.
    pub fn opened(&self) -> u64 {
        self.opened
    }

    /// How many times the engine has run a frame.
    pub fn polls(&self) -> u64 {
        self.polls
    }

    /// Forget every voice, as a device losing its stream would.
    pub fn clear(&mut self) {
        self.voices.clear();
    }
}

impl AudioDevice for HeadlessDevice {
    fn spec(&self) -> DeviceSpec {
        self.spec
    }

    fn open(&mut self, slot: u32, request: &PlayRequest) -> Result<SoundInfo> {
        self.opened += 1;
        self.voices.insert(
            slot,
            HeadlessVoice {
                slot,
                path: request.path.clone(),
                volume: sanitise_gain(request.volume),
                pitch: request.pitch,
                pan: StereoPan::CENTRE,
                spatial: request.spatial,
                looping: request.looping,
                state: InstanceState::Playing,
                elapsed: 0.0,
                duration: self.duration,
            },
        );
        Ok(SoundInfo {
            duration: self.duration,
        })
    }

    fn close(&mut self, slot: u32) {
        self.voices.remove(&slot);
    }

    fn pause(&mut self, slot: u32) {
        if let Some(voice) = self.voices.get_mut(&slot) {
            voice.state = InstanceState::Paused;
        }
    }

    fn resume(&mut self, slot: u32) {
        if let Some(voice) = self.voices.get_mut(&slot) {
            voice.state = InstanceState::Playing;
        }
    }

    fn set_volume(&mut self, slot: u32, volume: f32) {
        if let Some(voice) = self.voices.get_mut(&slot) {
            voice.volume = sanitise_gain(volume);
        }
    }

    fn set_pitch(&mut self, slot: u32, pitch: f32) {
        if let Some(voice) = self.voices.get_mut(&slot) {
            voice.pitch = pitch;
        }
    }

    fn set_pan(&mut self, slot: u32, pan: StereoPan) {
        if let Some(voice) = self.voices.get_mut(&slot) {
            voice.pan = pan;
        }
    }

    fn poll(&mut self, dt: f32) -> Vec<u32> {
        self.polls += 1;
        let dt = sanitise_delta(dt);
        let mut finished = Vec::new();
        for voice in self.voices.values_mut() {
            if voice.state != InstanceState::Playing || voice.looping {
                continue;
            }
            voice.elapsed += dt * voice.pitch;
            if voice
                .duration
                .is_some_and(|duration| voice.elapsed >= duration)
            {
                finished.push(voice.slot);
            }
        }
        finished
    }
}
