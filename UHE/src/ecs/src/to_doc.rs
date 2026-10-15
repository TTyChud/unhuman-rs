//! World → document.
//!
//! ## The round trip is the contract
//!
//! A scene that saves and reloads differently has a bug that only shows up in a
//! file: two people editing one scene produce a conflict nobody can read, and a
//! merge tool rewrites a scene because the tool ordered two entities differently.
//! So [`write`] is held to `doc → world → doc == doc`, byte for byte, by
//! `tests/doc_roundtrip.rs`.
//!
//! ## Two things have to be preserved that storage does not keep
//!
//! **Entity order.** hecs groups entities by archetype, so its iteration order
//! depends on which components an entity happens to have and reshuffles whenever
//! one is added. [`World::entities`] therefore reads the stored
//! [`Sequence`](crate::components::Sequence) instead of asking the storage, and
//! this is the function that depends on it.
//!
//! **Component order, within an entity.** Storage genuinely cannot do this: a
//! `Sprite` and a `Camera2d` live in columns, and there is no column order to
//! preserve. Components come back in [`crate::components::KINDS`] order — the
//! declaration order in `vault` — so a document already in that order round trips
//! exactly and one that is not round trips to the same scene with its components
//! sorted. The alternative is a stored per-entity list of component orders, which
//! is a second copy of every component list on disk to preserve a distinction no
//! consumer reads.
//!
//! ## The C++'s serializer is 30 KB and none of it is the hard part
//!
//! `SceneSerializer.cpp` is 900 lines of YAML emission and deserialisation, and
//! the parts worth keeping are the ones that are *not* about text: the entity
//! list, the parent links, and the rule that identity comes from the file. Those
//! are 30 lines here, and [`vault::scene_text`] owns the text.

use vault::doc::{EntityDoc, SceneDoc};

use crate::components::{LocalTransform, Name, Parent, Tags};
use crate::entity::Entity;
use crate::world::World;

/// Write a world out as a scene document.
///
/// Read-only: the world is not changed, so this can be called on a `&World` that
/// something else is iterating, and a save cannot change what it saved.
pub fn write(world: &World) -> SceneDoc {
    let entities = world
        .entities()
        .into_iter()
        .map(|entity| describe(world, entity))
        .collect();

    SceneDoc { entities }
}

/// One entity, as a document row.
fn describe(world: &World, entity: Entity) -> EntityDoc {
    EntityDoc {
        id: entity.uuid(),
        // The parent link, written straight from the component rather than
        // resolved through `hierarchy`, so a dangling parent survives the round
        // trip as the dangling link it is. Resolving it to "root" would quietly
        // repair a document on load -- and then a merge that produced the
        // dangling pointer would be lost on the next save.
        parent: world
            .component::<Parent>(entity)
            .map(|parent| parent.0)
            .unwrap_or(vault::doc::Uuid::NONE),
        name: world
            .component::<Name>(entity)
            .map(|name| name.0.clone())
            .unwrap_or_default(),
        tags: world
            .component::<Tags>(entity)
            .map(|tags| tags.0.clone())
            .unwrap_or_default(),
        transform: world
            .component::<LocalTransform>(entity)
            .map(|local| local.0)
            .unwrap_or_default(),
        components: crate::components::read(world, entity),
    }
}
