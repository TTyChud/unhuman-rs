//! The audio engine, and the plugin that installs it.
//!
//! The C++ split this in two: `AudioEngine` was a facade of statics that forwarded
//! to `MiniAudioBackend`, which owned the device in file-scope globals
//! (`s_AudioEngine`, `s_ActiveSounds`) — so nothing but `Application` could ever
//! have an audio engine, and nothing could be tested without a device. Here one
//! owner holds the voices and the clip table, the device is an `Option`, and the
//! bookkeeping runs with the `Option` empty.

use glam::Vec3;

use adam::{AudioBackend, Engine, FrameCtx, Listener, Plugin, Result, VoiceId};
use meats::hecs;
use meats::{AudioSource, Scene};
use vault::{AssetId, Assets};

use crate::backend::Backend;
use crate::sound::{Bus, SoundId, SoundTable};
use crate::spatial::{attenuation, pan};
use crate::voice::{VoicePool, VoiceSpec};

/// Voices an engine runs at once when nothing asks for another number.
pub const DEFAULT_VOICES: usize = 64;

/// Playback: the voice table, the clips, and the device if there is one.
///
/// With no output device this is a silent engine: it answers every call, counts no
/// voices and plays nothing. That is what makes registering it safe on a build
/// machine, and it is the only reason `WirePlugin::build` cannot fail.
pub struct AudioEngine {
    listener: Listener,
    master_gain: f32,
    bus_gains: [f32; Bus::COUNT],
    pool: VoicePool,
    sounds: SoundTable,
    device: Option<Backend>,
}

impl AudioEngine {
    /// Open the default output device; silent if there is none.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_VOICES)
    }

    /// As [`AudioEngine::new`], with a cap on simultaneous voices. A capacity of
    /// zero refuses every voice, which is a valid way to switch audio off.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            listener: Listener::default(),
            master_gain: 1.0,
            bus_gains: [1.0; Bus::COUNT],
            pool: VoicePool::new(capacity),
            sounds: SoundTable::new(),
            device: Backend::open(),
        }
    }

    /// True when there was no output device to open.
    pub fn is_silent(&self) -> bool {
        self.device.is_none()
    }

    pub fn listener(&self) -> Listener {
        self.listener
    }

    pub fn set_listener(&mut self, listener: Listener) {
        self.listener = listener;
    }

    /// Voices playing right now. Zero in silent mode, because nothing was started.
    pub fn active_voices(&self) -> usize {
        self.pool.len()
    }

    pub fn bus_gain(&self, bus: Bus) -> f32 {
        self.bus_gains[bus.slot()]
    }

    pub fn set_bus_gain(&mut self, bus: Bus, gain: f32) {
        self.bus_gains[bus.slot()] = gain;
    }

    /// The clip id for an asset, registering it the first time `vault` has its
    /// bytes. `None` while the load is in flight.
    pub fn register(&mut self, assets: &Assets, asset: AssetId) -> Option<SoundId> {
        self.sounds.resolve(assets, asset)
    }

    /// Play a clip from nowhere in particular: the C++ `PlaySound2D(filepath, volume, loop)`.
    pub fn play_2d(
        &mut self,
        assets: &Assets,
        clip: AssetId,
        gain: f32,
        looping: bool,
    ) -> Option<VoiceId> {
        let sound = self.register(assets, clip)?;
        self.play_sound(
            sound,
            VoiceSpec {
                gain,
                looping,
                ..VoiceSpec::default()
            },
        )
    }

    /// Play a clip at a world position: the C++ `PlaySound3D(filepath, position, volume, loop)`,
    /// which spatialised through miniaudio's engine and never knew the listener.
    pub fn play_3d(
        &mut self,
        assets: &Assets,
        clip: AssetId,
        position: Vec3,
        gain: f32,
        looping: bool,
        range: f32,
    ) -> Option<VoiceId> {
        let sound = self.register(assets, clip)?;
        self.play_sound(
            sound,
            VoiceSpec {
                gain,
                looping,
                spatial: true,
                range,
                position,
                ..VoiceSpec::default()
            },
        )
    }

    /// Start an already-registered clip. `None` when the device refused it or the
    /// pool had no room.
    pub fn play_sound(&mut self, sound: SoundId, spec: VoiceSpec) -> Option<VoiceId> {
        self.device.as_ref()?;
        let id = self.pool.spawn(None, sound, spec)?;
        if self.start_player(id) {
            Some(id)
        } else {
            self.pool.remove(id);
            None
        }
    }

    /// Stop a voice now, whether the scene started it or a direct play did.
    pub fn stop(&mut self, voice: VoiceId) -> bool {
        if let Some(backend) = self.device.as_mut() {
            backend.stop(voice);
        }
        self.pool.remove(voice)
    }

    /// One frame: the voices are brought in line with the scene's playing sources,
    /// then the device is driven to match. A no-op with no device.
    pub fn update(&mut self, scene: &Scene, assets: &Assets) {
        if self.device.is_none() {
            return;
        }
        self.reconcile(scene, assets);
        self.drive();
    }

    /// Start the player of a voice that has none, failing when its clip cannot be
    /// decoded.
    fn start_player(&mut self, id: VoiceId) -> bool {
        let Some(voice) = self.pool.get(id) else {
            return false;
        };
        let Some(sound) = self.sounds.get(voice.sound) else {
            return false;
        };
        let Some(backend) = self.device.as_mut() else {
            return false;
        };
        backend.start(voice, sound)
    }

    /// Bring the voice pool in line with the scene: a source that started playing
    /// gets a voice at its entity's world position, a source that stopped loses it,
    /// and a running voice picks up whatever the component now says.
    fn reconcile(&mut self, scene: &Scene, assets: &Assets) {
        let world = scene.world();
        let mut query = world.query::<(hecs::Entity, &AudioSource)>();
        let sources = query.iter().filter_map(|(hecs_entity, source)| {
            let entity = scene.entity_of(hecs_entity)?;
            let position = scene.world_transform(entity).w_axis.truncate();
            Some((entity, source, position))
        });

        let pool = &mut self.pool;
        let sounds = &mut self.sounds;
        pool.reconcile(sources, |source| match source.clip {
            Some(clip) => sounds.resolve(assets, clip),
            None => None,
        });
    }

    /// Bring the device in line with the pool: start what has no player yet, apply
    /// gain, pitch and pan, and reap what has finished.
    fn drive(&mut self) {
        let listener = self.listener;
        let master = self.master_gain;
        let buses = self.bus_gains;

        // Voices the scene added this frame have no player yet.
        let pending: Vec<VoiceId> = match &self.device {
            Some(backend) => self
                .pool
                .voices()
                .iter()
                .filter(|voice| !backend.has(voice.id))
                .map(|voice| voice.id)
                .collect(),
            None => return,
        };
        for id in pending {
            // A clip that cannot be decoded is dropped rather than retried, and
            // logged, every frame.
            if !self.start_player(id) {
                self.pool.remove(id);
            }
        }

        let Some(backend) = self.device.as_mut() else {
            return;
        };

        for id in backend.finished() {
            backend.stop(id);
            self.pool.remove(id);
        }

        for voice in self.pool.voices() {
            let distance = (voice.spec.position - listener.position).length();
            if voice.spec.spatial {
                voice.pan.set_pan(pan(&listener, voice.spec.position));
            } else {
                voice.pan.set_gains(1.0, 1.0);
            }
            let falloff = if voice.spec.spatial {
                attenuation(distance, voice.spec.range)
            } else {
                1.0
            };
            let volume = voice.spec.gain * falloff * buses[voice.spec.bus.slot()] * master;
            backend.set_gain(voice.id, volume, voice.spec.pitch);
        }
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for AudioEngine {
    fn name(&self) -> &'static str {
        "wire"
    }

    fn listener(&self) -> Listener {
        self.listener
    }

    fn set_listener(&mut self, listener: Listener) {
        self.listener = listener;
    }

    /// `dt` is unused: rodio's clock is the device's, and the mixer has already been
    /// playing for however long the frame took.
    fn update(&mut self, scene: &Scene, assets: &Assets, _dt: f32) {
        AudioEngine::update(self, scene, assets);
    }

    fn play(&mut self, assets: &Assets, clip: AssetId, gain: f32) -> Option<VoiceId> {
        self.play_2d(assets, clip, gain, false)
    }

    fn stop(&mut self, voice: VoiceId) {
        AudioEngine::stop(self, voice);
    }

    fn active_voices(&self) -> usize {
        self.pool.len()
    }

    fn master_gain(&self) -> f32 {
        self.master_gain
    }

    fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain;
    }
}

/// Installs [`AudioEngine`] into `Engine::audio` and drives it once a frame.
pub struct WirePlugin {
    capacity: usize,
}

impl WirePlugin {
    pub fn new() -> Self {
        Self {
            capacity: DEFAULT_VOICES,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { capacity }
    }
}

impl Default for WirePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for WirePlugin {
    fn name(&self) -> &'static str {
        "wire"
    }

    fn build(&mut self, eng: &mut Engine) -> Result<()> {
        eng.audio = Some(Box::new(AudioEngine::with_capacity(self.capacity)));
        Ok(())
    }

    /// Once a frame, not once per fixed step: the listener follows the camera, which
    /// moves per frame, and a frame whose accumulator reached no step runs no
    /// substeps at all. The C++ called `AudioEngine::Update()` from the frame loop
    /// for the same reason.
    fn on_frame(&mut self, eng: &mut Engine, _ctx: &mut FrameCtx) {
        let Some(audio) = eng.audio.as_mut() else {
            return;
        };
        audio.set_listener(listener_from(&eng.scene));
        audio.update(&eng.scene, &eng.assets, 0.0);
    }
}

/// The primary camera's pose as the ear — what the C++ game layer did every frame
/// with `SetListenerPosition(camera.GetPosition(), forward, up)`.
pub fn listener_from(scene: &Scene) -> Listener {
    let Some(camera) = scene.primary_camera() else {
        return Listener::default();
    };
    let transform = scene.world_transform(camera);
    Listener::new(
        transform.w_axis.truncate(),
        // Right-handed: a camera looks down its own -Z.
        -transform.z_axis.truncate(),
        transform.y_axis.truncate(),
    )
}
