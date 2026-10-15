//! The entity handle, and where a fresh UUID comes from.
//!
//! ## Why the handle is a UUID and not a slot
//!
//! The C++'s `Entity` is `entt::entity` — a 32-bit index plus a generation —
//! and it is wrapped in a pointer to the `Scene` that owns it. It is a *slot*,
//! and a slot means two things this port refuses:
//!
//! * it is meaningless outside the world that minted it, so a scene cannot be
//!   saved with parent links and reloaded, and
//! * the hierarchy would be keyed on it, which is the bug
//!   `docs/TRANSFER.md` calls out: "an index-based parent means reordering an
//!   entity list silently reparents everything".
//!
//! [`Entity`] is therefore a `Uuid` and nothing else. `World` keeps the
//! `uuid → slot` index that turns a handle into storage, which is what the C++'s
//! `Scene::m_UUIDIndex` does, and which the C++ only added after UUID scans
//! started showing up in frame times.
//!
//! The handle is `Copy`, so passing one to a system cannot invalidate a
//! [`World::each`] walk — which is exactly the point of [`World::queue_mutation`]
//! for the cases where mutating is wanted.

use crate::components::Id;
use crate::world::{Error, World};
use vault::doc::Uuid;

/// A handle to one entity.
///
/// Equality and hashing are the UUID's, so two handles from two different
/// worlds compare equal if they name the same identity, and a handle whose
/// entity was despawned is simply a handle that resolves to nothing — there is
/// no dangling generation to check and no "is this still valid" that can be
/// forgotten.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Entity(Uuid);

impl Entity {
    /// The identity this handle names.
    pub fn uuid(self) -> Uuid {
        self.0
    }

    /// A handle built from an identity, for code that holds ids rather than
    /// handles: undo/redo, a document load, a save file, an editor's selection.
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Whether this handle names an entity in `world`.
    pub fn is_alive(self, world: &World) -> bool {
        world.contains(self)
    }

    /// The identity the world has recorded for this handle.
    ///
    /// Every entity gets an [`Id`] at spawn, so this is the same UUID as
    /// [`Entity::uuid`] — it exists so the ECS has a *component* to query on,
    /// because [`crate::to_doc::write`] has to read identity out of an
    /// iteration and cannot call a method per handle.
    pub fn id(self, world: &World) -> Result<Id, Error> {
        world
            .component::<Id>(self)
            .copied()
            .ok_or(Error::MissingComponent {
                entity: self,
                kind: std::any::type_name::<Id>(),
            })
    }
}

impl From<Uuid> for Entity {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl From<Entity> for Uuid {
    fn from(entity: Entity) -> Self {
        entity.0
    }
}

impl std::fmt::Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// A 64-bit mixer, and a UUID built from two of its outputs.
///
/// ## What this is not
///
/// Not a CSPRNG and not trying to be. A scene's identity has to be *unique*,
/// never *unguessable*: nothing in the engine treats an entity id as a secret,
/// and a saved scene file is full of them in plain text. What it must not be is
/// a fresh `u64` counter, which is what the C++ mints — and a counter collides
/// the moment two scenes are merged, or a scene is loaded twice, or an
/// `AssetID` is compared across runs. `docs/PARITY.md` lists "IDs are re-minted
/// each run" as one of upstream's bugs for exactly this reason.
///
/// ## The mixer
///
/// `splitmix64`: three shifts, two multiplies, no state table and no
/// cryptographic claim. It is here because it is one function long and its
/// output has no visible correlation between consecutive draws, which is all a
/// UUID v4 field needs. Version and variant nibbles are set so the result is a
/// well-formed RFC 4122 UUID rather than 16 random bytes.
#[derive(Clone)]
pub(crate) struct UuidMinter {
    state: u64,
}

impl Default for UuidMinter {
    fn default() -> Self {
        Self::seeded()
    }
}

impl UuidMinter {
    /// Seed from the clock and the heap address of this world.
    ///
    /// Both, because either alone is guessable and this is the only place a
    /// seed is needed. Two worlds started in the same nanosecond on the same
    /// thread get the same stream, which is harmless — they are different
    /// processes' worth of ids in different files, and `from_doc` mints
    /// nothing.
    pub(crate) fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        let here = &nanos as *const u64 as u64;
        Self {
            state: nanos ^ here.rotate_left(17),
        }
    }

    /// The next UUID.
    pub(crate) fn mint(&mut self) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.next().to_be_bytes());
        bytes[8..].copy_from_slice(&self.next().to_be_bytes());
        // Version 4, variant 1 (RFC 4122 §4.1.3 / §4.1.1). A UUID that claims
        // to be a v1 and is not one is worse than one that claims nothing,
        // because a tool will try to read a timestamp out of it.
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid(bytes)
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_minted_uuid_is_well_formed_and_unique() {
        let mut minter = UuidMinter::seeded();
        let mut seen = HashSet::with_capacity(4096);
        for _ in 0..4096 {
            let id = minter.mint();
            assert_eq!(id.0[6] >> 4, 4, "version nibble");
            assert_eq!(id.0[8] >> 6, 0b10, "variant bits");
            assert!(
                seen.insert(id),
                "splitmix64 collided after {} draws",
                seen.len()
            );
        }
    }
}
