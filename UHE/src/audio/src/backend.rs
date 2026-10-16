//! rodio output: the counterpart of `MiniAudioBackend`.
//!
//! Everything that touches the sound card lives here — the open sink, and one
//! `Player` per voice — so [`crate::AudioEngine`] above it is plain bookkeeping that
//! runs with no device, no thread and no driver at all.

use std::collections::HashMap;

use rodio::source::UniformSourceIterator;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

use adam::VoiceId;

use crate::sound::Sound;
use crate::spatial::{Pan, PanHandle, CHANNELS};
use crate::voice::Voice;

/// An open output device and the voices playing through it.
///
/// Dropping this is the C++ `Shutdown`: `players` is declared first so it drops
/// first, and the sounds stop while the stream that plays them is still open.
pub struct Backend {
    players: HashMap<VoiceId, Player>,
    sink: MixerDeviceSink,
}

impl Backend {
    /// Open the default output device, or `None` when this machine has none.
    ///
    /// A machine with no sound card is a configuration, not an error: the caller
    /// runs silent, which is what the C++ did too — its `Init` logged an error and
    /// returned with `s_Initialized` false, and every later call became a no-op.
    pub fn open() -> Option<Self> {
        match DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                // rodio prints on drop to catch a sink that was dropped by accident.
                // This one is owned for the engine's whole life.
                sink.log_on_drop(false);
                log::info!("wire: audio output open ({:?})", sink.config());
                Some(Self {
                    players: HashMap::new(),
                    sink,
                })
            }
            Err(error) => {
                log::warn!("wire: no audio output device, running silent: {error}");
                None
            }
        }
    }

    pub fn has(&self, id: VoiceId) -> bool {
        self.players.contains_key(&id)
    }

    /// Decode a clip and start it on this device. `false` when the clip cannot be
    /// decoded — the C++ `MA_SUCCESS` check, which logged the path and freed the
    /// sound it had allocated.
    pub fn start(&mut self, voice: &Voice, sound: &Sound) -> bool {
        if voice.spec.looping {
            // Told to loop by the decoder, which seeks back to the start. Looping the
            // source instead (`repeat_infinite`) buffers the whole clip in memory.
            let decoder = match Decoder::new_looped(sound.reader()) {
                Ok(decoder) => decoder,
                Err(error) => {
                    log::warn!("wire: cannot decode sound {}: {error}", sound.id().index());
                    return false;
                }
            };
            let player = Player::connect_new(self.sink.mixer());
            player.append(stereo(decoder, &voice.pan));
            self.players.insert(voice.id, player);
        } else {
            let decoder = match Decoder::new(sound.reader()) {
                Ok(decoder) => decoder,
                Err(error) => {
                    log::warn!("wire: cannot decode sound {}: {error}", sound.id().index());
                    return false;
                }
            };
            let player = Player::connect_new(self.sink.mixer());
            player.append(stereo(decoder, &voice.pan));
            self.players.insert(voice.id, player);
        }
        true
    }

    /// Gain and pitch of a playing voice. Panning is applied per sample by the
    /// voice's own source, and the distance falloff is already in `volume`.
    pub fn set_gain(&mut self, id: VoiceId, volume: f32, speed: f32) {
        if let Some(player) = self.players.get(&id) {
            player.set_volume(volume);
            player.set_speed(speed);
        }
    }

    /// Stop a voice now: dropping its player ends the sound.
    pub fn stop(&mut self, id: VoiceId) -> bool {
        self.players.remove(&id).is_some()
    }

    /// The voices whose sound has played to the end — the C++ sweep over
    /// `s_activeSounds`, which freed every `ma_sound` that had stopped playing.
    pub fn finished(&self) -> Vec<VoiceId> {
        self.players
            .iter()
            .filter(|(_, player)| player.empty())
            .map(|(id, _)| *id)
            .collect()
    }
}

/// Pan and attenuate in stereo, whatever the clip was recorded in: `Pan` needs two
/// channels, and miniaudio spatialised into two as well.
fn stereo<S>(decoder: S, pan: &PanHandle) -> Pan<UniformSourceIterator<S>>
where
    S: Source,
{
    let rate = decoder.sample_rate();
    Pan::new(
        UniformSourceIterator::new(decoder, CHANNELS, rate),
        pan.clone(),
    )
}
