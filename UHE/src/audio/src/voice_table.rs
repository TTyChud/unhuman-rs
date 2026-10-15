//! The voice table: reserve a slot, find a sound, free a slot.
//!
//! Split out of [`AudioEngine`](crate::AudioEngine) because it is the half that
//! owns handle identity, and handle identity has to live in exactly one place.
//!
//! ## Why this is the interesting part
//!
//! The C++ identifies a playing sound by **pointer**. `s_ActiveSounds` is a
//! `std::vector<ma_sound*>` (`MiniAudioBackend.cpp:18`) and
//! `ma_sound_uninit(sound); delete sound;` hands the address straight back to the
//! allocator. The next `PlaySound3D` very plausibly gets the same address. Any
//! handle built from that pointer would name the wrong sound, so the C++ has no
//! handles at all and solves the problem by having no control surface.
//!
//! A table with a generation has the same slot twice over time and can tell the
//! two apart, so a stale handle is *detectably* stale rather than silently
//! correct-looking. That is why [`PlayHandle`] is two words and why every
//! handle-taking method on the engine returns a `bool`.

use crate::error::{Error, Result};
use crate::instance::{PlayHandle, SoundInstance};

/// One row of the voice table.
///
/// `Default` gives generation `0`, which is never handed out — see
/// [`PlayHandle::new`].
#[derive(Clone, Default, Debug)]
pub struct Slot {
    /// Which occupant of this slot the current handle refers to.
    pub(crate) generation: u32,
    /// The sound, or `None` when the slot is free.
    pub(crate) instance: Option<SoundInstance>,
}

/// The voice table.
///
/// A `Vec<Slot>` with a live count, and nothing else. The scan is linear and
/// that is the right answer: a voice count is tens, and a free list is a second
/// structure that can disagree with the table, which is a bug class this
/// codebase has already paid for in `ObjectPool.h` (a 0-byte file).
#[derive(Debug, Default)]
pub(crate) struct SlotTable {
    slots: Vec<Slot>,
    live: usize,
}

impl SlotTable {
    /// An empty table.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reserve the lowest free slot and bump its generation.
    pub(crate) fn reserve(&mut self) -> Result<u32> {
        match self.slots.iter().position(|slot| slot.instance.is_none()) {
            Some(index) => {
                self.slots[index].generation = self.slots[index].generation.wrapping_add(1).max(1);
                Ok(index as u32)
            }
            None => {
                let index = u32::try_from(self.slots.len())
                    .map_err(|_| Error::too_many_voices(self.slots.len()))?;
                self.slots.push(Slot {
                    generation: 1,
                    instance: None,
                });
                Ok(index)
            }
        }
    }

    /// The handle a freshly reserved slot will be known by.
    ///
    /// Separate from [`Self::insert`] because a [`SoundInstance`] carries its own
    /// handle and therefore has to be constructed *before* it is stored, which
    /// is the one unavoidable ordering wrinkle in
    /// [`AudioEngine::play`](crate::AudioEngine::play).
    pub(crate) fn handle_at(&self, index: u32) -> PlayHandle {
        PlayHandle::new(index, self.slots[index as usize].generation)
    }

    /// Put a sound into a reserved slot and hand back its handle.
    ///
    /// Takes the index rather than being called right after [`Self::reserve`],
    /// because the device is opened *between* the two and an open that fails has
    /// to give the slot back through [`Self::abandon`].
    pub(crate) fn insert(&mut self, index: u32, instance: SoundInstance) -> PlayHandle {
        let handle = instance.handle();
        self.slots[index as usize].instance = Some(instance);
        self.live += 1;
        handle
    }

    /// Give a reserved slot back after a failed open.
    pub(crate) fn abandon(&mut self, index: u32) {
        if let Some(slot) = self.slots.get_mut(index as usize) {
            slot.instance = None;
        }
    }

    /// The sound a handle names, if it is still that sound.
    ///
    /// `None` for a stale handle is the entire point of the generation.
    pub(crate) fn get(&self, handle: PlayHandle) -> Option<&SoundInstance> {
        self.slots
            .get(handle.index() as usize)
            .and_then(|slot| slot.instance.as_ref())
            .filter(|instance| instance.handle().is_same_sound(&handle))
    }

    /// The sound a handle names, if it is still that sound.
    pub(crate) fn get_mut(&mut self, handle: PlayHandle) -> Option<&mut SoundInstance> {
        self.slots
            .get_mut(handle.index() as usize)
            .and_then(|slot| slot.instance.as_mut())
            .filter(|instance| instance.handle().is_same_sound(&handle))
    }

    /// Release a slot by number, ignoring generations entirely.
    ///
    /// Generation-blind on purpose: only the engine's `stop` and `update` reach
    /// it and both have already established the slot is live. A check here would
    /// be a second implementation of handle identity.
    pub(crate) fn close(&mut self, index: u32) {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return;
        };
        if slot.instance.take().is_some() {
            self.live = self.live.saturating_sub(1);
        }
    }

    /// How many slots the table has ever allocated, occupied or not.
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    /// How many slots are occupied.
    pub(crate) fn live(&self) -> usize {
        self.live
    }

    /// Every occupied slot, mutably, paired with its index.
    ///
    /// The index is returned alongside because the frame loop has to tell the
    /// device *which* voice it is talking to, and reaching back into the table
    /// to find it would mean walking it twice per frame.
    pub(crate) fn instances_mut(&mut self) -> impl Iterator<Item = (u32, &mut SoundInstance)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| slot.instance.as_mut().map(|i| (index as u32, i)))
    }
}
