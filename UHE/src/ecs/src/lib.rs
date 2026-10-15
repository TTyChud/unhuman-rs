//! `meats` — the entity-component-system.
//!
//! ## hecs underneath, nothing of hecs outside
//!
//! Storage is [`hecs`] 0.11.2. No `hecs` type appears in this crate's public
//! interface: `World` is ours, `Entity` is ours, and a component is anything
//! that satisfies [`Storable`]. The C++ exposes `entt::registry&` from
//! `Scene::GetRegistry()` and every system then reaches into `entt` directly —
//! which is why its systems have to know that EnTT groups storage by archetype
//! and that a view is a chain of raw pointers. Every caller here goes through
//! [`World`], which is where that knowledge stops.
//!
//! ## An entity is a UUID, and that is deliberate
//!
//! [`Entity`] is a `Uuid`, not a storage slot, and a parent link is a `Uuid`.
//! This is inherited from the C++ (`IDComponent` is a `u64` UUID,
//! `RelationshipComponent::m_ParentID` refers to one) and it is deliberate for
//! the reason `docs/PARITY.md` gives for not copying a bug: an index-based
//! parent means reordering an entity list silently reparents everything. It
//! also means an entity can move between scenes without a rewrite, and that
//! `World` can be cloned for a play-mode snapshot with every parent link still
//! pointing at the right node.
//!
//! The cost is one hash lookup per access, paid through a `uuid → slot` index
//! in [`World`]. The C++ pays the same lookup (`Scene::m_UUIDIndex`, added in
//! "Issue #17 hardening" once UUID scans started dominating frame time).
//!
//! ## Every entity has the same shape
//!
//! [`World::spawn`] always attaches [`Id`], [`Name`], [`Tags`],
//! [`Sequence`], [`LocalTransform`], [`Parent`], [`Children`] and
//! [`WorldTransform`]. Nothing that walks the scene has to handle "this entity
//! has no parent component", which is the difference between a `parent()` that
//! returns a `Uuid` and one that returns an `Option` and makes every caller
//! carry a branch.
//!
//! ## The deferred queue is not optional
//!
//! hecs moves an entity to a new archetype when a component is inserted, and an
//! archetype move rewrites the entity's row. Any iterator into that row — a
//! view, a query — is invalidated. EnTT does exactly the same on `emplace`,
//! which is why the C++ has `m_PendingReparents` and friends: the editor
//! requests a hierarchy change from inside a tree walk and applies it at the
//! top of the next frame, outside every iteration. [`World::queue_mutation`]
//! is that mechanism, generalised, and [`deferred`] explains it.
//!
//! ## Not ported, on purpose
//!
//! * `Scene::Scene`'s `m_registry.storage<T>()` priming loop — a DLL workaround
//!   for EnTT asserting across module boundaries. hecs has no equivalent hazard.
//! * The 21 `OnComponentAdded` specializations — 20 of them are empty, and the
//!   21st sets a camera's viewport. A `World` can do that in the one function
//!   that inserts a camera, and 21 hooks for one is not a design.
//! * `Scene::Copy`, 220 hand-written lines copying 20 components field by field.
//!   [`World`] derives `Clone`.
//!
//! ## Layout
//!
//! | file | concern |
//! |---|---|
//! | [`entity`] | the handle, and where a fresh UUID comes from |
//! | [`world`] | storage: spawn, despawn, insert, remove, get, query |
//! | [`deferred`] | the mutation queue that makes mutation safe |
//! | [`hierarchy`] | parent/child links, roots, descendants, paths |
//! | [`hierarchy_ops`] | reparenting, detaching, with cycle refusal |
//! | [`world_transform`] | the cached world transform and its dirty flag |
//! | [`from_doc`] / [`to_doc`] | the two directions of a scene document |
//!
//! `docs/TRANSFER.md` sketches `components/` as a directory of one file per
//! component. That is not what is here, and the reason is that these components
//! are newtypes over `vault`'s payloads plus seven three-line structs: ten
//! files of twenty lines each is a directory with ten doc comments and no
//! reader. The list that matters — the mapping from a document component kind
//! to a stored component — is one list in [`components`], and it generates both
//! directions, which is the property that stops the two copies from drifting.

#![deny(missing_docs)]

pub mod components;
pub mod deferred;
pub mod entity;
pub mod from_doc;
pub mod hierarchy;
pub mod hierarchy_ops;
pub mod to_doc;
pub mod world;
pub mod world_transform;

pub use components::{Children, Id, LocalTransform, Name, Parent, Sequence, WorldTransform};
pub use deferred::Mutation;
pub use entity::Entity;
pub use from_doc::BuildError;
pub use hierarchy_ops::ReparentError;
pub use world::{Error, Storable, World};

/// Build a world from a scene document. See [`from_doc::build`].
pub use from_doc::build;

/// Write a world out as a scene document. See [`to_doc::write`].
pub use to_doc::write;

/// `vault`'s scene-document types, re-exported so a caller building a world
/// needs one `use` and not two.
///
/// Re-exported rather than required through `meats` paths, because
/// [`components::Id`] *is* a `vault` UUID and a signature that says
/// `vault::doc::Uuid` where the caller has `meats::Id` in hand is a papercut.
pub use vault::doc::{SceneDoc, Transform, Uuid};
