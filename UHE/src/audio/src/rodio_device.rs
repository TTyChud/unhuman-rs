//! The real device. Together with nothing else, the only file that names `rodio`.
//!
//! Three jobs: decode a file, start one rodio `Player` per voice, and write the
//! gains the mixer computed into it. That is all.
//!
//! ## Why the maths is *not* here, even though rodio will do it
//!
//! rodio ships a `SpatialPlayer` that takes an emitter position and two ear
//! positions and does the attenuation and the panning itself, and using it would
//! be less code than this crate has. It would also be untestable: every value a
//! rodio spatialiser produces comes out of a running audio stream, so the only
//! way to assert on it is to play a sound through a device that does not exist
//! in CI. That is precisely how the C++ ended up with spatial audio that nobody
//! can check -- miniaudio's `ma_engine` does the same job for the same reason,
//! and `AimLabLayer.cpp` has been unable to tell whether its gunshots pan
//! correctly since it was written.
//!
//! So the gains are computed in [`crate::attenuation`], [`crate::pan`] and
//! [`crate::listener`], where they are pure functions of `f32`, and rodio is
//! asked to multiply by a number it was handed.
//!
//! ## Getting a frame-rate gain onto an audio-thread source
//!
//! A `rodio::Source` is an iterator consumed on the audio thread, so it cannot
//! be handed a `&mut` from the frame loop. rodio's answer is
//! `source::periodic`, a modifier closure handed `&mut` to the inner source every
//! [`PAN_INTERVAL_MS`]; this device uses it to copy a shared pan cell into a
//! `ChannelVolume` on each side. This is the same mechanism rodio's own
//! `SpatialPlayer` uses, for the same reason, and it is why
//! [`AudioDevice::set_pan`](crate::AudioDevice::set_pan) exists at all.
//!
//! ## Decoded-sound cache, because the C++ re-decodes on every shot
//!
//! `MiniAudioBackend::PlaySound3D` calls `ma_sound_init_from_file` every single
//! time (`MiniAudioBackend.cpp:98-101`), so every gunshot in `AimLabLayer.cpp`
//! re-opens the file and re-decodes it from disk on the frame the trigger was
//! pulled. Upstream's own design doc calls this "the largest audio defect". A
//! `SamplesBuffer` clone is an `Arc` bump, so caching decoded samples by path
//! turns a repeat play into a pointer copy.
//!
//! The cache is unbounded, which is a real limitation and not a hidden one: in
//! the target design sound lifetime belongs to `vault`'s asset registry, and
//! this `HashMap` is the placeholder for that lookup until it exists. Until then
//! it costs one buffer per distinct sound file the process has ever played,
//! which for a game is a bounded set.

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Source;

/// How often a spatial voice re-reads its pan, in milliseconds.
///
/// rodio runs the source chain on its audio thread and this is how the frame
/// thread's gain reaches it -- the same trick rodio's own `SpatialPlayer` uses.
/// Ten milliseconds is below the ~20 ms at which a gain step becomes audible as
/// a zipper noise, and it is not below it on purpose: a shorter period means
/// taking a mutex on the audio thread more often, and a panic inside that mutex
/// would kill the audio stream.
const PAN_INTERVAL_MS: u64 = 10;

use crate::device::{AudioDevice, DeviceSpec, PlayRequest, SoundInfo};
use crate::gain::sanitise_gain;
use crate::pan::StereoPan;
use crate::{Error, Result};

/// A rodio sink and one `Player` per voice.
///
/// The decoded-sample cache is the reason this type exists rather than a bare
/// `MixerDeviceSink`: `MiniAudioBackend` re-decodes from disk on every shot,
/// and an engine that does that cannot keep a frame rate under gunfire.
pub struct RodioDevice {
    sink: rodio::MixerDeviceSink,
    voices: Vec<Option<RodioVoice>>,
    decoded: HashMap<PathBuf, rodio::buffer::SamplesBuffer>,
}

/// One rodio player, and the cell the frame thread writes its pan through.
struct RodioVoice {
    player: rodio::Player,
    /// `None` for a 2D sound, which has no stereo path at all.
    ///
    /// Shared with a `rodio::source::periodic` running on the audio thread; see
    /// [`PAN_INTERVAL_MS`].
    pan: Option<Arc<Mutex<StereoPan>>>,
}

impl RodioDevice {
    /// Open the platform's default output device.
    ///
    /// `Err` on a machine with no sound card is the expected case, not an
    /// exceptional one, and it is why [`AudioEngine::headless`](crate::AudioEngine::headless)
    /// exists. The error carries rodio's own message, because "no audio device"
    /// and "the device is busy" have opposite fixes and cpal reports both as an
    /// opaque string.
    pub fn open_default() -> Result<Self> {
        let sink = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|error| Error::no_device(error.to_string()))?;
        Ok(RodioDevice {
            sink,
            voices: Vec::new(),
            decoded: HashMap::new(),
        })
    }

    /// Decoded samples for a path, decoding once.
    ///
    /// A `SamplesBuffer` clone is an `Arc` bump, so handing the same buffer to
    /// a hundred concurrent voices costs a hundred pointer copies and no memory.
    fn decode(&mut self, path: &Path) -> Result<rodio::buffer::SamplesBuffer> {
        if let Some(cached) = self.decoded.get(path) {
            return Ok(cached.clone());
        }
        let file = std::fs::File::open(path).map_err(|source| Error::decode(path, source))?;
        let decoder =
            rodio::Decoder::try_from(BufReader::new(file)).map_err(|e| Error::decode(path, e))?;
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let samples: Vec<f32> = decoder.collect();
        if samples.is_empty() {
            return Err(Error::empty_sound(path));
        }
        let buffer = rodio::buffer::SamplesBuffer::new(channels, sample_rate, samples);
        self.decoded.insert(path.to_path_buf(), buffer.clone());
        Ok(buffer)
    }

    fn with_voice(&self, slot: u32, f: impl FnOnce(&RodioVoice)) {
        if let Some(voice) = self.voices.get(slot as usize).and_then(Option::as_ref) {
            f(voice);
        }
    }
}

impl AudioDevice for RodioDevice {
    fn spec(&self) -> DeviceSpec {
        DeviceSpec {
            sample_rate: self.sink.config().sample_rate().get(),
            channel_count: self.sink.config().channel_count().get(),
        }
    }

    fn open(&mut self, slot: u32, request: &PlayRequest) -> Result<SoundInfo> {
        let samples = self.decode(&request.path)?;
        let duration = samples
            .total_duration()
            .map(|d: Duration| d.as_secs_f32())
            .filter(|d| d.is_finite() && *d > 0.0);

        let player = rodio::Player::connect_new(self.sink.mixer());
        let source: rodio::Player = player;
        source.set_volume(sanitise_gain(request.volume));
        source.set_speed(request.pitch.max(0.0));

        let pan = if request.spatial {
            // `ChannelVolume` folds the input to mono and then applies a gain
            // per output channel, which is the right model for a point emitter:
            // one signal radiating, two ears receiving it at different levels. It
            // is the *wrong* model for a stereo music track, which is exactly why
            // a 2D sound does not go through it.
            let cell = Arc::new(Mutex::new(StereoPan::CENTRE));
            let panelled = rodio::source::ChannelVolume::new(
                samples,
                vec![StereoPan::CENTRE.left, StereoPan::CENTRE.right],
            )
            .periodic_access(Duration::from_millis(PAN_INTERVAL_MS), {
                let cell = Arc::clone(&cell);
                move |inner: &mut rodio::source::ChannelVolume<_>| {
                    let pan = *cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    inner.set_volume(0, pan.left);
                    inner.set_volume(1, pan.right);
                }
            });
            if request.looping {
                source.append(panelled.repeat_infinite());
            } else {
                source.append(panelled);
            }
            Some(cell)
        } else if request.looping {
            source.append(samples.repeat_infinite());
            None
        } else {
            source.append(samples);
            None
        };

        if self.voices.len() <= slot as usize {
            self.voices.resize_with(slot as usize + 1, || None);
        }
        self.voices[slot as usize] = Some(RodioVoice {
            player: source,
            pan,
        });
        Ok(SoundInfo { duration })
    }

    fn close(&mut self, slot: u32) {
        if let Some(voice) = self.voices.get_mut(slot as usize).and_then(Option::take) {
            voice.player.stop();
        }
    }

    fn pause(&mut self, slot: u32) {
        self.with_voice(slot, |voice| voice.player.pause());
    }

    fn resume(&mut self, slot: u32) {
        self.with_voice(slot, |voice| voice.player.play());
    }

    fn set_volume(&mut self, slot: u32, volume: f32) {
        let volume = sanitise_gain(volume);
        self.with_voice(slot, |voice| voice.player.set_volume(volume));
    }

    fn set_pitch(&mut self, slot: u32, pitch: f32) {
        let pitch = pitch.max(0.0);
        self.with_voice(slot, |voice| voice.player.set_speed(pitch));
    }

    fn set_pan(&mut self, slot: u32, pan: StereoPan) {
        self.with_voice(slot, |voice| {
            // A poisoned mutex means a pan write panicked while the audio
            // thread held it. Recovering the value is better than propagating
            // the panic into the audio callback, which would kill the stream.
            if let Some(cell) = &voice.pan {
                *cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = pan;
            }
        });
    }

    fn poll(&mut self, _dt: f32) -> Vec<u32> {
        // rodio plays on its own thread and a `Player` stops when its source
        // ends, so there is nothing to report. The engine's own clock already
        // knows about the voice's length.
        Vec::new()
    }
}
