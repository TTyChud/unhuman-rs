//! The clips the engine can play and the gain groups they play through.
//!
//! `vault` owns the bytes; the table keeps one `Arc` copy per clip so a voice can
//! be started without copying the file, and remembers which `AssetId` it came
//! from so a scene component only ever names the asset.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use vault::{AssetId, Assets};

/// Handle to a clip registered with the engine, stable for the engine's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundId(u64);

impl SoundId {
    pub fn new(index: u64) -> Self {
        Self(index)
    }

    pub fn index(self) -> u64 {
        self.0
    }
}

/// A gain group, so pulling one kind of sound down leaves the others alone. Every
/// voice is on one bus; [`crate::VoiceSpec`] puts scene sources on [`Bus::Sfx`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bus {
    #[default]
    Sfx,
    Music,
    Ui,
}

impl Bus {
    /// Every bus, in the order the engine stores their gains.
    pub const ALL: [Bus; 3] = [Bus::Sfx, Bus::Music, Bus::Ui];

    /// How many gains an engine holds.
    pub const COUNT: usize = Bus::ALL.len();

    pub(crate) fn slot(self) -> usize {
        match self {
            Bus::Sfx => 0,
            Bus::Music => 1,
            Bus::Ui => 2,
        }
    }
}

/// A registered clip: the bytes `vault` handed over, kept for decoding.
#[derive(Clone, Debug)]
pub struct Sound {
    id: SoundId,
    asset: AssetId,
    bytes: Arc<[u8]>,
}

impl Sound {
    pub fn id(&self) -> SoundId {
        self.id
    }

    pub fn asset(&self) -> AssetId {
        self.asset
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// A reader for `rodio::Decoder` sharing the clip's bytes rather than copying
    /// them, so a second shot at the same clip costs no memory.
    pub fn reader(&self) -> Cursor<Arc<[u8]>> {
        Cursor::new(Arc::clone(&self.bytes))
    }
}

/// The engine's clip table.
#[derive(Debug, Default)]
pub struct SoundTable {
    sounds: Vec<Sound>,
    by_asset: HashMap<AssetId, SoundId>,
}

impl SoundTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.sounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sounds.is_empty()
    }

    pub fn get(&self, id: SoundId) -> Option<&Sound> {
        self.sounds.get(id.index() as usize).filter(|s| s.id == id)
    }

    pub fn id_for(&self, asset: AssetId) -> Option<SoundId> {
        self.by_asset.get(&asset).copied()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Sound> {
        self.sounds.iter()
    }

    pub fn clear(&mut self) {
        self.sounds.clear();
        self.by_asset.clear();
    }

    /// Register a clip under `asset`. Registering the same asset twice keeps the
    /// first copy and returns its id, so a scene that asks every frame copies each
    /// clip exactly once.
    pub fn register(&mut self, asset: AssetId, bytes: &[u8]) -> SoundId {
        if let Some(id) = self.by_asset.get(&asset) {
            return *id;
        }
        let id = SoundId::new(self.sounds.len() as u64);
        self.sounds.push(Sound {
            id,
            asset,
            bytes: Arc::from(bytes),
        });
        self.by_asset.insert(asset, id);
        id
    }

    /// The engine's resolver: the clip's id once `vault` has its bytes, and `None`
    /// while the load is still in flight, which leaves the source for a later frame.
    pub fn resolve(&mut self, assets: &Assets, asset: AssetId) -> Option<SoundId> {
        if let Some(id) = self.by_asset.get(&asset) {
            return Some(*id);
        }
        let bytes = assets.bytes(asset)?;
        Some(self.register(asset, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(index: u32) -> AssetId {
        AssetId::from_parts(index, 1)
    }

    #[test]
    fn registering_the_same_asset_twice_keeps_one_copy() {
        let mut table = SoundTable::new();
        let first = table.register(asset(7), &[1, 2, 3, 4]);
        let second = table.register(asset(7), &[9, 9, 9, 9]);
        assert_eq!(first, second);
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(first).expect("registered").bytes(), &[1, 2, 3, 4]);
        assert_eq!(table.id_for(asset(7)), Some(first));
        assert_eq!(table.id_for(asset(8)), None);
    }

    #[test]
    fn sound_readers_share_the_bytes() {
        let mut table = SoundTable::new();
        let id = table.register(asset(1), &[1, 2, 3, 4]);
        let sound = table.get(id).expect("registered");
        assert_eq!(sound.len(), 4);
        assert!(!sound.is_empty());
        assert_eq!(sound.reader().get_ref().len(), 4);
    }

    #[test]
    fn a_later_registration_gets_the_next_id() {
        let mut table = SoundTable::new();
        let first = table.register(asset(1), &[0]);
        let second = table.register(asset(2), &[0]);
        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(table.iter().count(), 2);
        table.clear();
        assert!(table.is_empty());
        assert!(table.get(first).is_none());
    }
}
