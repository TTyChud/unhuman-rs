//! Document → world.
//!
//! ## Why a world is built *from* a document and not the other way round
//!
//! `vault::doc::SceneDoc` is a plain value: no handles, no references, no `Rc`,
//! nothing that needs an ECS to exist. That is what makes it serializable,
//! diffable and testable, and it is why this direction is the primary one. The
//! C++ goes the other way — it has a live registry and calls
//! `SceneSerializer::Serialize` to get a YAML file out — so its serializer is the
//! only thing in the engine that can describe a scene, and every bug in it is a
//! bug that only ever appears in a file.
//!
//! [`crate::to_doc`] is therefore not the interesting inverse of this. It is the
//! *writer*, and the two are held to a round trip.
//!
//! ## This refuses a broken document; the loader does not
//!
//! [`vault::doc::SceneDoc::validate`] finds duplicate ids, dangling parents,
//! parent cycles and duplicate components. Its own docs say the ECS builder is
//! what refuses: the editor's job is to open a broken scene, not to reject it,
//! so the file loader reports and this builder refuses.
//!
//! ## Identity comes from the document
//!
//! Every entity is spawned with the UUID the document gave it, so a saved scene
//! keeps its identity and the parent links in it survive the trip. The C++ has
//! to be told this: `Scene::Copy` copies `IDComponent` and then has to unindex
//! the generated id and index the real one — three lines that exist only because
//! identity is not what a fresh entity gets.

use vault::components::Component;
use vault::doc::{InvalidScene, SceneDoc, Uuid};

use crate::components::{Children, LocalTransform, Parent, StoreError, Tags};
use crate::entity::Entity;
use crate::world::World;
use crate::world_transform;

/// Build a world from a scene document.
///
/// Entity order is the document's order, which is what [`World::entities`]
/// reports back, and which is what makes saving a loaded scene produce no diff.
pub fn build(document: &SceneDoc) -> Result<World, BuildError> {
    document.validate()?;

    let mut world = World::new();
    world.reserve(document.entities.len());

    // Pass one: every entity, with its identity, name, transform and parent. The
    // document is flat, so a child may be listed before its parent -- the normal
    // case for a hand-edited file -- and a UUID link does not care which order the
    // two were spawned in.
    for entity in &document.entities {
        world.spawn_bundled(
            entity.id,
            entity.name.clone(),
            LocalTransform(entity.transform),
            Parent(entity.parent),
        );
        let handle = Entity::from_uuid(entity.id);
        if !entity.tags.is_empty() {
            if let Some(tags) = world.component_mut::<Tags>(handle) {
                tags.0 = entity.tags.clone();
            }
        }
    }

    // Pass two: the child lists, now that every entity exists. Pushed in
    // document order, so a tree drawn from this looks like the file reads.
    for entity in &document.entities {
        if entity.parent.is_none() {
            continue;
        }
        let parent = Entity::from_uuid(entity.parent);
        if let Some(children) = world.component_mut::<Children>(parent) {
            children.0.push(entity.id);
        }
    }

    // Pass three: components. A separate pass because each one is its own insert
    // and its own archetype move, and a loader that interleaves them with the
    // spawns pays for two moves per entity instead of one.
    for entity in &document.entities {
        let handle = Entity::from_uuid(entity.id);
        for component in &entity.components {
            store(&mut world, handle, component)?;
        }
    }

    // A freshly built world has fresh transforms: nothing has moved since the
    // document was written, and a caller who forgets to call
    // [`world_transform::update`] should get right matrices rather than a screen
    // full of stacked entities.
    world_transform::update(&mut world);
    Ok(world)
}

/// Why a document could not become a world.
#[derive(Clone, PartialEq, Debug)]
pub enum BuildError {
    /// The document is not self-consistent. See [`vault::doc::InvalidScene`].
    Invalid(InvalidScene),
    /// A component kind `vault` has and this build of `meats` does not store.
    ///
    /// Reachable when `vault` gains a component and `meats` has not been rebuilt
    /// against it. It is a refusal rather than a skip because skipping drops
    /// data: the scene would load, look right, and lose the component on the next
    /// save with nothing to say so.
    UnknownComponent {
        /// The entity the component was on.
        entity: Uuid,
        /// The kind's name, as `vault` spells it.
        kind: &'static str,
    },
    /// Storage refused an insert.
    ///
    /// Unreachable in practice -- `build` spawns every entity before storing
    /// anything on it -- and still not a panic, because a loader that panics is
    /// how one corrupt file becomes an editor that will not start.
    Storage(crate::Error),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Invalid(error) => write!(f, "{error}"),
            BuildError::UnknownComponent { entity, kind } => write!(
                f,
                "{entity:?} has a {kind} component, which this build of meats cannot store"
            ),
            BuildError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BuildError::Invalid(error) => Some(error),
            BuildError::UnknownComponent { .. } => None,
            BuildError::Storage(error) => Some(error),
        }
    }
}

impl From<InvalidScene> for BuildError {
    fn from(error: InvalidScene) -> Self {
        BuildError::Invalid(error)
    }
}

impl From<StoreError> for BuildError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::UnknownComponent { entity, kind } => {
                BuildError::UnknownComponent { entity, kind }
            }
            StoreError::Entity(error) => BuildError::Storage(error),
        }
    }
}

/// Store one document component.
///
/// A named function rather than inlined so the `?` in `build` reads as "store
/// this", and so the one thing this pass does has one name in one file.
fn store(world: &mut World, entity: Entity, component: &Component) -> Result<(), BuildError> {
    crate::components::insert(world, entity, component)?;
    Ok(())
}
