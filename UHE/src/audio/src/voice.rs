//! The active voices: one per entity, in a table that cannot grow without bound.

use adam::VoiceId;
use glam::Vec3;
use meats::{AudioSource, Entity};

use crate::sound::{Bus, SoundId};
use crate::spatial::PanHandle;

/// The range a spatial voice falls off over when nothing names one.
pub const DEFAULT_RANGE: f32 = 50.0;

/// How one voice sounds. In the live path this is copied off the scene's
/// `AudioSource` once a frame, which is how a gain changed in the editor reaches a
/// sound that is already playing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceSpec {
    pub gain: f32,
    pub pitch: f32,
    pub looping: bool,
    pub spatial: bool,
    pub range: f32,
    pub position: Vec3,
    pub bus: Bus,
}

impl VoiceSpec {
    /// What a scene source asks for, at the entity's world position — the component
    /// carries no position of its own.
    pub fn from_source(source: &AudioSource, position: Vec3) -> Self {
        Self {
            gain: source.gain,
            pitch: source.pitch,
            looping: source.looping,
            spatial: source.spatial,
            range: source.range,
            position,
            bus: Bus::Sfx,
        }
    }
}

impl Default for VoiceSpec {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pitch: 1.0,
            looping: false,
            spatial: false,
            range: DEFAULT_RANGE,
            position: Vec3::ZERO,
            bus: Bus::Sfx,
        }
    }
}

/// One playing sound.
#[derive(Debug)]
pub struct Voice {
    pub id: VoiceId,
    pub sound: SoundId,
    /// The entity this voice follows, if it came from the scene rather than from a
    /// direct `play`.
    pub entity: Option<Entity>,
    pub spec: VoiceSpec,
    /// Written by the engine once a frame, read by the audio thread.
    pub pan: PanHandle,
    /// Set while [`VoicePool::reconcile`] walks the scene, sweeping the voices whose
    /// entity it did not visit.
    seen: bool,
}

/// The active voices.
///
/// Bounded on purpose: a spawn into a full pool drops the voice that has been
/// playing longest, so a scene that starts a sound every frame cannot grow the
/// table without limit. The C++ `s_activeSounds` was an unbounded vector that only
/// shrank when a sound ended, and a `CreateEntity`/`DestroyEntity` loop that started
/// sounds leaked one `ma_sound` per iteration.
#[derive(Debug)]
pub struct VoicePool {
    capacity: usize,
    next_id: VoiceId,
    voices: Vec<Voice>,
}

impl VoicePool {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next_id: 1,
            voices: Vec::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.voices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    pub fn voices(&self) -> &[Voice] {
        &self.voices
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Voice> {
        self.voices.iter()
    }

    pub fn get(&self, id: VoiceId) -> Option<&Voice> {
        self.voices.iter().find(|voice| voice.id == id)
    }

    pub fn get_mut(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|voice| voice.id == id)
    }

    pub fn voice_of(&self, entity: Entity) -> Option<&Voice> {
        self.voices.iter().find(|voice| voice.entity == Some(entity))
    }

    /// Add a voice. `None` when the pool has no room at all: a capacity of zero
    /// means "audio off", not "evict everything".
    ///
    /// An entity already sounding is replaced rather than shadowed, so a re-used
    /// entity never inherits a stale voice.
    pub fn spawn(&mut self, entity: Option<Entity>, sound: SoundId, spec: VoiceSpec) -> Option<VoiceId> {
        if self.capacity == 0 {
            return None;
        }
        if let Some(entity) = entity {
            if let Some(index) = self.voices.iter().position(|voice| voice.entity == Some(entity)) {
                self.voices.remove(index);
            }
        }
        while self.voices.len() >= self.capacity {
            // Voice ids only increase, so the front of the table is the oldest.
            self.voices.remove(0);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.voices.push(Voice {
            id,
            sound,
            entity,
            spec,
            pan: PanHandle::default(),
            seen: true,
        });
        Some(id)
    }

    /// Drop one voice, if it is still there.
    pub fn remove(&mut self, id: VoiceId) -> bool {
        match self.voices.iter().position(|voice| voice.id == id) {
            Some(index) => {
                self.voices.remove(index);
                true
            }
            None => false,
        }
    }

    /// Keep only the voices the predicate accepts — the sweep over the voices whose
    /// sound has played to the end.
    pub fn retain(&mut self, mut keep: impl FnMut(&Voice) -> bool) {
        self.voices.retain(|voice| keep(voice));
    }

    pub fn clear(&mut self) {
        self.voices.clear();
    }

    /// One frame of bookkeeping against the scene's `AudioSource` components: a
    /// playing source that has no voice gets one, a live voice picks up the
    /// component's gain, pitch, looping, spatial flag and range, and a source that
    /// stopped loses its voice. Voices whose entity the scene no longer has are
    /// swept.
    ///
    /// `resolve` decides whether a source can be played at all. The engine's
    /// resolver looks the clip up in `vault` and answers `None` while its bytes are
    /// still loading, which leaves the source for a later frame.
    pub fn reconcile<'a, I, F>(&mut self, sources: I, mut resolve: F)
    where
        I: IntoIterator<Item = (Entity, &'a AudioSource, Vec3)>,
        F: FnMut(&AudioSource) -> Option<SoundId>,
    {
        for voice in self.voices.iter_mut() {
            voice.seen = false;
        }
        for (entity, source, position) in sources {
            let spec = VoiceSpec::from_source(source, position);
            match self.voices.iter().position(|voice| voice.entity == Some(entity)) {
                Some(index) => {
                    if source.playing {
                        self.voices[index].spec = spec;
                        self.voices[index].seen = true;
                    } else {
                        self.voices.remove(index);
                    }
                }
                None => {
                    if source.playing {
                        if let Some(sound) = resolve(source) {
                            self.spawn(Some(entity), sound, spec);
                        }
                    }
                }
            }
        }
        // Voices started directly, with no entity behind them, are not the scene's
        // to sweep.
        self.voices.retain(|voice| voice.seen || voice.entity.is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meats::Scene;

    fn source(playing: bool) -> AudioSource {
        AudioSource {
            clip: None,
            clip_path: String::new(),
            playing,
            looping: false,
            gain: 1.0,
            pitch: 1.0,
            spatial: true,
            range: DEFAULT_RANGE,
        }
    }

    /// Entities are `meats`', not wire's: the only way to make one is a scene.
    fn entity(name: &str) -> Entity {
        Scene::new().spawn(name)
    }

    #[test]
    fn spawn_replaces_the_voice_of_a_re_used_entity() {
        let mut pool = VoicePool::new(4);
        let entity = entity("source");
        let first = pool
            .spawn(Some(entity), SoundId::new(1), VoiceSpec::default())
            .expect("room");
        let second = pool
            .spawn(Some(entity), SoundId::new(2), VoiceSpec::default())
            .expect("room");
        assert_ne!(first, second);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.voice_of(entity).expect("one voice").sound, SoundId::new(2));
    }

    #[test]
    fn a_zero_capacity_pool_refuses_every_voice() {
        let mut pool = VoicePool::new(0);
        assert!(pool.spawn(None, SoundId::new(1), VoiceSpec::default()).is_none());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn reconcile_updates_a_running_voice_in_place() {
        let mut pool = VoicePool::new(4);
        let entity = entity("source");
        let quiet = source(true);
        pool.reconcile([(entity, &quiet, Vec3::ZERO)], |_| Some(SoundId::new(1)));
        let id = pool.voice_of(entity).expect("spawned").id;

        let mut loud = source(true);
        loud.gain = 0.25;
        loud.looping = true;
        let position = Vec3::new(1.0, 0.0, 2.0);
        pool.reconcile([(entity, &loud, position)], |_| Some(SoundId::new(1)));

        assert_eq!(pool.len(), 1);
        let voice = pool.get(id).expect("same voice");
        assert_eq!(voice.spec.gain, 0.25);
        assert!(voice.spec.looping);
        assert_eq!(voice.spec.position, position);
    }

    #[test]
    fn reconcile_sweeps_a_voice_whose_entity_is_gone() {
        let mut pool = VoicePool::new(4);
        let entity = entity("source");
        let playing = source(true);
        pool.reconcile([(entity, &playing, Vec3::ZERO)], |_| Some(SoundId::new(1)));
        assert_eq!(pool.len(), 1);

        pool.reconcile(Vec::new(), |_| Some(SoundId::new(1)));
        assert_eq!(pool.len(), 0);
    }
}
