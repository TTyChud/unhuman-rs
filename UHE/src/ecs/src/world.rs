//! Storage. Spawning, despawning, and getting components in and out of hecs.
//!
//! ## What this wrapper is for
//!
//! hecs is a library, not a framework, and it is deliberately small: a `World`
//! plus `query::<Q>()` and a borrow that is checked in debug builds. That is
//! exactly what the C++ wanted from EnTT and no more, and it is also why the
//! C++ leaks `entt::registry&` out of `Scene::GetRegistry()` — with nothing in
//! between, every system learns the storage's rules for itself.
//!
//! Three rules are enforced here instead, once:
//!
//! * **A handle is a UUID.** [`World`] owns the `uuid → slot` index, so
//!   nothing outside this file knows that storage is indexed at all.
//! * **An entity has a fixed component set.** Every spawn attaches identity,
//!   name, tags, order, local transform, parent, children and the cached world
//!   transform, so a walk over the scene never has to ask whether a link
//!   exists.
//! * **Mutation goes through the queue** ([`crate::deferred`]). [`World::insert`]
//!   takes `&mut self`, which is what makes [`World::each`] safe to iterate: a
//!   view cannot be alive while someone inserts, because the borrow checker
//!   will not let it. hecs's runtime borrow flags are the backstop, not the
//!   mechanism.
//!
//! ## Order
//!
//! [`World::entities`] returns entities in [`Sequence`] order, which is the
//! order the document listed them in. hecs yields entities grouped by
//! archetype, so "the order the world iterates" is an artefact of which
//! components exist and would reshuffle a scene file on every save. Order is
//! therefore stored, not assumed.

use std::collections::HashMap;

use crate::components::{
    Children, Id, LocalTransform, Name, Parent, Sequence, Tags, WorldTransform,
};
use crate::entity::{Entity, UuidMinter};
use glam::Mat4;
use vault::doc::Uuid;

/// A value that can be stored on an entity.
///
/// Blanket-implemented for anything that is `Clone + Send + Sync + 'static`,
/// which is also what makes `World` itself `Send` and `Sync`: hecs marks its
/// `World` `Send`/`Sync` unconditionally, on the assumption that components
/// are, so the assumption is made explicit here where it can be a bound rather
/// than a comment.
///
/// `Clone` is required because [`World`] is clonable, and that is what buys the
/// replacement for the C++'s 220-line `Scene::Copy`: a play-mode snapshot is
/// `self.clone()`. `Send + Sync` because the deferred queue holds
/// `FnOnce(&mut World) + Send + Sync` — a closure that had to be `!Send` could
/// not be handed to the job system that drains the queue.
pub trait Storable: Clone + Send + Sync + 'static {}

impl<T: Clone + Send + Sync + 'static> Storable for T {}

/// Why an operation on an entity did not happen.
///
/// One type for both failures because callers handle them the same way — "that
/// handle is not usable" — and because a second error enum per operation is how
/// a caller ends up matching `Err(_)` and forgetting the case that mattered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// The handle names no entity in this world.
    ///
    /// Not a panic. A handle outlives its entity on purpose: undo keeps one for
    /// something the user deleted, and a script can hold one across a scene
    /// swap. The C++ asserts instead, which means its editor crashes on an undo
    /// it can describe.
    UnknownEntity(Entity),
    /// The entity is here but does not have that component.
    MissingComponent {
        /// The entity that was asked.
        entity: Entity,
        /// `type_name` of the component, which is what a caller needs to read a
        /// failure out of a log.
        kind: &'static str,
    },
    /// A hierarchy change was refused or could not be completed.
    ///
    /// Here because [`crate::deferred`]'s queue carries one error type and this
    /// is what a reparent returns — the alternative is a queue that can only ever
    /// report storage failures, and a refused reparent is the failure a caller
    /// most needs to hear about.
    Hierarchy(crate::hierarchy_ops::ReparentError),
}

impl Error {
    /// Whether this is an entity that is not in the world.
    ///
    /// For a caller that has already established an entity exists and wants to
    /// treat anything else as a different kind of failure.
    pub fn is_unknown_entity(&self) -> bool {
        matches!(self, Error::UnknownEntity(_))
    }
}

impl From<crate::hierarchy_ops::ReparentError> for Error {
    fn from(error: crate::hierarchy_ops::ReparentError) -> Self {
        Error::Hierarchy(error)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnknownEntity(entity) => write!(f, "{entity} is not in this world"),
            Error::MissingComponent { entity, kind } => write!(f, "{entity} has no {kind}"),
            Error::Hierarchy(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Hierarchy(error) => Some(error),
            Error::UnknownEntity(_) | Error::MissingComponent { .. } => None,
        }
    }
}

/// The world: every entity, and where its components live.
///
/// Cloned by `Clone`, and that is the whole of `Scene::Copy`'s replacement. The
/// C++ writes out twenty components field by field and, in doing so, has to
/// remember that `IDComponent` needs its index entry rebuilt and that
/// `Model3DComponent`'s `Ref<RD3d::Model>` must *not* be copied — which is a
/// bug that a derive makes unrepresentable rather than merely avoided.
#[derive(Default)]
pub struct World {
    storage: hecs::World,
    /// `uuid → storage slot`. The only reason [`Entity`] can be a UUID.
    by_uuid: HashMap<Uuid, hecs::Entity>,
    /// The same association backwards, so an iteration can name what it found.
    ///
    /// Kept in step with `by_uuid` rather than read out of the [`Id`]
    /// component: fetching `Id` inside a query that already fetches `Id` makes
    /// hecs' debug borrow checker panic, and a walk over [`Id`] is a thing a
    /// caller will do.
    by_slot: HashMap<hecs::Entity, Uuid>,
    /// Monotonic insertion counter, the source of [`Sequence`].
    next_sequence: u32,
    /// Source of fresh UUIDs for [`World::spawn`].
    minter: UuidMinter,
    /// Crate-private so [`crate::deferred`] can hold the queue's representation
    /// without it appearing in this type's public surface.
    pub(crate) pending: Vec<crate::deferred::Mutation>,
    /// Failures from the last flushes. See [`World::take_failures`].
    pub(crate) failures: Vec<Error>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The storage has no `Debug` worth printing — it is a type-erased
        // archetype table, and a derived one would be thousands of lines.
        f.debug_struct("World")
            .field("entities", &self.len())
            .field("pending_mutations", &self.pending.len())
            .finish()
    }
}

impl Clone for World {
    /// A clone carries **no** pending mutations.
    ///
    /// A queued mutation is a request from a UI about the world it is looking
    /// at, not part of the world's state; copying one into a play-mode snapshot
    /// would apply an editor drag-drop to a scene the editor is not editing. The
    /// C++ has the same shape by accident — `Scene::Copy` builds a fresh `Scene`
    /// and its pending vectors are empty.
    fn clone(&self) -> Self {
        use crate::components::{
            Children, LocalTransform, Name, Parent, Sequence, Tags, WorldTransform,
        };
        let mut new = Self::new();
        new.next_sequence = self.next_sequence;
        new.minter = self.minter.clone();
        new.pending = Vec::new();
        new.failures = self.failures.clone();
        for entity in self.entities() {
            let id = entity.uuid();
            let name = self
                .component::<Name>(entity)
                .cloned()
                .unwrap_or_else(|| Name(String::new()));
            let transform = self
                .component::<LocalTransform>(entity)
                .copied()
                .unwrap_or_default();
            let parent = self
                .component::<Parent>(entity)
                .copied()
                .unwrap_or_default();
            let handle = new.spawn_bundled(id, name.0, transform, parent);
            if let Some(tags) = self.component::<Tags>(entity) {
                if let Some(slot) = new.component_mut::<Tags>(handle) {
                    *slot = tags.clone();
                }
            }
            if let Some(sequence) = self.component::<Sequence>(entity) {
                let _ = new.insert(handle, *sequence);
            }
            if let Some(children) = self.component::<Children>(entity) {
                if let Some(slot) = new.component_mut::<Children>(handle) {
                    *slot = children.clone();
                }
            }
            if let Some(world_transform) = self.component::<WorldTransform>(entity) {
                let _ = new.insert(handle, *world_transform);
            }
            for component in crate::components::read(self, entity) {
                let _ = crate::components::insert(&mut new, handle, &component);
            }
        }
        new
    }
}

impl World {
    /// An empty world.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn an entity with a fresh UUID and an empty name.
    pub fn spawn(&mut self) -> Entity {
        let id = self.minter.mint();
        self.spawn_with(id, String::new())
    }

    /// Spawn an entity with an identity the caller already has.
    ///
    /// For [`crate::from_doc`], which must keep the ids a file stores, and for
    /// undo/redo restoring one. Nothing is minted: the caller's UUID is the
    /// entity's.
    pub fn spawn_with(&mut self, id: Uuid, name: impl Into<String>) -> Entity {
        self.spawn_bundled(id, name, LocalTransform::default(), Parent::default())
    }

    /// Spawn an entity with an identity, a name, a local transform and a parent.
    ///
    /// The one constructor [`crate::from_doc`] uses, so a loaded scene does not
    /// spawn at the origin and then have its transform written — one archetype
    /// transition instead of two, and no moment in which half the entities in a
    /// freshly loaded scene are at the world origin.
    pub(crate) fn spawn_bundled(
        &mut self,
        id: Uuid,
        name: impl Into<String>,
        transform: LocalTransform,
        parent: Parent,
    ) -> Entity {
        assert!(
            !self.by_uuid.contains_key(&id),
            "two entities with the same UUID: {id:?}"
        );
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        // One bundle, so the entity is written once rather than moved between
        // archetypes eight times on the way in.
        let slot = self.storage.spawn((
            Id(id),
            Name(name.into()),
            Tags::default(),
            Sequence(sequence),
            transform,
            parent,
            Children::default(),
            WorldTransform::stale(Mat4::IDENTITY),
        ));
        self.by_uuid.insert(id, slot);
        self.by_slot.insert(slot, id);
        Entity::from_uuid(id)
    }

    /// How many entities there are.
    pub fn len(&self) -> usize {
        self.by_uuid.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.by_uuid.is_empty()
    }

    /// Reserve room for `count` more entities.
    ///
    /// [`crate::from_doc`] knows the entity count before it spawns anything, and
    /// one up-front reserve beats a grow step per archetype transition.
    pub fn reserve(&mut self, count: usize) {
        self.storage.reserve_entities(count as u32);
    }

    /// Whether `entity` is still here.
    pub fn contains(&self, entity: Entity) -> bool {
        self.by_uuid.contains_key(&entity.uuid())
    }

    /// Every entity, in [`Sequence`] order.
    ///
    /// Allocates, because the storage cannot produce this order directly: hecs
    /// groups by archetype, so its own iteration order depends on which
    /// components an entity happens to have. That is fine for a system and
    /// wrong for a file, and this function is where the difference is known.
    pub fn entities(&self) -> Vec<Entity> {
        let mut ordered: Vec<(u32, Entity)> = self
            .by_uuid
            .iter()
            .map(|(id, slot)| {
                let sequence = self
                    .storage
                    .get::<&Sequence>(*slot)
                    .map(|s| s.0)
                    .unwrap_or(u32::MAX);
                (sequence, Entity::from_uuid(*id))
            })
            .collect();
        ordered.sort_unstable_by_key(|(sequence, _)| *sequence);
        ordered.into_iter().map(|(_, entity)| entity).collect()
    }

    /// Despawn one entity.
    ///
    /// Detaches it from its parent's [`Children`] first, so the link does not
    /// outlive the node. Children are *not* despawned: a subtree is a
    /// deliberate operation and lives in
    /// [`crate::hierarchy_ops::despawn_subtree`], where the caller has said so
    /// out loud. A queued mutation naming this entity fails with
    /// [`Error::UnknownEntity`] when it runs, which is why the queue does not
    /// have to be rewritten here.
    pub fn despawn(&mut self, entity: Entity) -> Result<(), Error> {
        let id = entity.uuid();
        let slot = *self.by_uuid.get(&id).ok_or(Error::UnknownEntity(entity))?;
        if let Some(&Parent(parent)) = self.component::<Parent>(entity) {
            if let Some(Children(children)) =
                self.component_mut::<Children>(Entity::from_uuid(parent))
            {
                children.retain(|child| *child != id);
            }
        }
        self.storage
            .despawn(slot)
            .map_err(|_| Error::UnknownEntity(entity))?;
        self.by_uuid.remove(&id);
        self.by_slot.remove(&slot);
        Ok(())
    }

    /// Add or replace a component.
    ///
    /// Replaces rather than refusing if one is already there, which is what a
    /// doc load and an undo both want; [`World::has`] tells the difference.
    ///
    /// Moves the entity to another archetype. **Never call this while a view is
    /// alive** — queue it with [`World::queue_mutation`] instead.
    pub fn insert<T: Storable>(&mut self, entity: Entity, component: T) -> Result<(), Error> {
        let slot = *self
            .by_uuid
            .get(&entity.uuid())
            .ok_or(Error::UnknownEntity(entity))?;
        self.storage
            .insert_one(slot, component)
            .map_err(|_| Error::UnknownEntity(entity))
    }

    /// Remove a component, handing it back.
    ///
    /// Returns the old value because undo needs it, and because a caller who
    /// removes a component has almost always meant to look at it first.
    pub fn remove<T: Storable>(&mut self, entity: Entity) -> Result<T, Error> {
        let slot = *self
            .by_uuid
            .get(&entity.uuid())
            .ok_or(Error::UnknownEntity(entity))?;
        self.storage
            .remove_one::<T>(slot)
            .map_err(|_| Error::MissingComponent {
                entity,
                kind: std::any::type_name::<T>(),
            })
    }

    /// Whether `entity` has a `T`.
    pub fn has<T: Storable>(&self, entity: Entity) -> bool {
        self.component::<T>(entity).is_some()
    }

    /// Read one component.
    pub fn component<T: Storable>(&self, entity: Entity) -> Option<&T> {
        let slot = *self.by_uuid.get(&entity.uuid())?;
        let reference = self.storage.get::<&T>(slot).ok()?;
        // hecs hands out a borrow token rather than a raw reference; the token's
        // pointer is valid for as long as this `&self` borrow is, so extend
        // the reference's lifetime to match and let the token drop here.
        unsafe { Some(std::mem::transmute::<&T, &T>(&*reference)) }
    }

    /// Borrow one component mutably.
    pub fn component_mut<T: Storable>(&mut self, entity: Entity) -> Option<&mut T> {
        let slot = *self.by_uuid.get(&entity.uuid())?;
        let mut reference = self.storage.get::<&mut T>(slot).ok()?;
        unsafe { Some(std::mem::transmute::<&mut T, &mut T>(&mut *reference)) }
    }

    /// Walk every entity that has a `T`.
    ///
    /// Takes `&mut self` for a read-only walk because the only archetype
    /// iteration hecs offers without a per-access borrow check needs a unique
    /// borrow. It is also what makes this safe: the callback cannot insert,
    /// because `visit` does not have the `&mut World` that would be needed. The
    /// only way to mutate during a walk is to queue the mutation — see
    /// [`World::queue_mutation`].
    ///
    /// Order is unspecified; [`World::entities`] is the ordered one.
    pub fn each<T: Storable>(&mut self, mut visit: impl FnMut(Entity, &T)) {
        let by_slot = &self.by_slot;
        for (slot, component) in self.storage.query_mut::<(hecs::Entity, &T)>() {
            let handle = Entity::from_uuid(by_slot[&slot]);
            visit(handle, component);
        }
    }

    /// Walk every entity that has a `T`, mutating it.
    pub fn each_mut<T: Storable>(&mut self, mut visit: impl FnMut(Entity, &mut T)) {
        let by_slot = &self.by_slot;
        for (slot, component) in self.storage.query_mut::<(hecs::Entity, &mut T)>() {
            let handle = Entity::from_uuid(by_slot[&slot]);
            visit(handle, component);
        }
    }

    /// Walk every entity that has both an `A` and a `B`.
    ///
    /// Two is the only arity, deliberately: the walks that need this are "every
    /// `LocalTransform` that also has a `Model3d`" and "every `LocalTransform`
    /// that also has a `DirectionalLight`" — the renderer submitting models and
    /// the renderer extracting lights. A three-way version is the next thing to
    /// arrive and the next thing to be wrong.
    pub fn each_pair<A: Storable, B: Storable>(&self, mut visit: impl FnMut(Entity, &A, &B)) {
        let by_slot = &self.by_slot;
        for (slot, a, b) in self.storage.query::<(hecs::Entity, &A, &B)>().iter() {
            let handle = Entity::from_uuid(by_slot[&slot]);
            visit(handle, a, b);
        }
    }

    /// Walk every entity that has an `A` and a `B`, mutating the `A`.
    pub fn each_pair_mut<A: Storable, B: Storable>(
        &mut self,
        mut visit: impl FnMut(Entity, &mut A, &B),
    ) {
        let by_slot = &self.by_slot;
        for (slot, a, b) in self.storage.query_mut::<(hecs::Entity, &mut A, &mut B)>() {
            let handle = Entity::from_uuid(by_slot[&slot]);
            visit(handle, a, b);
        }
    }
}
