//! The components a scene is made of.
//!
//! ## Two sets, one list
//!
//! There are two kinds of thing here and they are not interchangeable:
//!
//! * the eight components every entity has ([`Id`], [`Name`], [`Tags`],
//!   [`Sequence`], [`LocalTransform`], [`Parent`], [`Children`],
//!   [`WorldTransform`]), which are the ECS's own bookkeeping, and
//! * the document components, which are [`vault::components`] payloads stored
//!   one per type — a `Sprite` is a `Sprite`, not a `SpriteComponent` holding
//!   one.
//!
//! The second kind is a *list of pairs*, and the pair is generated from the one
//! [`declare_document_components!`] invocation below. That is the same move
//! `vault::components` makes with its own list, and it is the point: a
//! hand-written second copy of a component list is a list that will drift, and
//! drift here is silent — a component that inserts but cannot be read back
//! vanishes from the next save with no error anywhere.
//!
//! ## Not `Tag`, not `Relationship`, not `Transform`
//!
//! `docs/TRANSFER.md` says the ECS holds 20 components, and the C++ gets there
//! by making `TransformComponent`, `IDComponent`, `TagComponent` and
//! `RelationshipComponent` four of the twenty. That is what EnTT forces when a
//! component is the only way to store anything. Here [`LocalTransform`],
//! [`Id`], [`Tags`] and [`Parent`]/[`Children`] are still components — they have
//! to be, a renderer iterates transforms — but they are *ours*, always present,
//! and never optional. That is the difference between a hierarchy walk that
//! cannot fail and one that handles four missing-component cases.

use glam::Mat4;
use vault::components::{
    Animator, BoxCollider2d, BoxCollider3d, Camera2d, CapsuleCollider3d, DirectionalLight, Model3d,
    NativeScript, PointLight, RigidBody, RigidBody2d, SphereCollider3d, SpotLight, Sprite,
    SpriteAnimation, Text,
};
use vault::doc::{Transform, Uuid};

/// Identity: the UUID an entity is known by.
///
/// Redundant with [`crate::Entity`], and deliberately so. A handle is what code
/// holds; `Id` is what a *query* can see. [`crate::to_doc::write`] reads
/// identity out of an iteration, and an ECS with no identity component is an
/// ECS that cannot be enumerated without consulting something outside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Id(pub Uuid);

/// The display name. Not unique, not an identifier.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Name(pub String);

/// Free-form labels.
///
/// A `Vec<String>` rather than the C++'s single `Tag`: every consumer of a tag
/// ends up wanting more than one, and `EntityDoc` already stores them this way.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Tags(pub Vec<String>);

/// Position in the world's canonical order.
///
/// The reason is [`World::entities`](crate::World::entities): hecs groups
/// entities by archetype, so the order the storage iterates in depends on which
/// components an entity has and reshuffles when one is added. A scene file
/// saved in that order produces a diff on every save, which trains everyone to
/// ignore diffs.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Sequence(pub u32);

/// Where the entity sits relative to its parent.
///
/// Stored as the document's quaternion-and-scale triple rather than the C++'s
/// Euler angles, for the reason `vault::doc::Transform` gives: Euler angles
/// interpolate wrongly, gimbal-lock at 90°, and a scene file is full of
/// rotations. This is the one place the port is not a translation — the C++'s
/// `SetLocalTransformFromWorld` converts back to Euler after every reparent, so
/// a reparent during a drag loses accuracy and can visibly shear.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct LocalTransform(pub Transform);

/// The parent entity, or [`Uuid::NONE`] for a root.
///
/// A UUID and not a handle for the reason the whole hierarchy is: an
/// index-based parent means reordering the entity list silently reparents
/// everything. A *dangling* parent UUID is legal and means "root" — that is
/// what makes merging two scene files safe, since the loser of a merge ends up
/// with one orphan rather than a corrupted chain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Parent(pub Uuid);

impl Default for Parent {
    /// A root.
    ///
    /// Hand-written because `vault::doc::Uuid` has no `Default`, and its absence
    /// is deliberate there: `Uuid::NONE` is a named constant, and a `default()`
    /// that silently means "no parent" would let a forgotten field in a builder
    /// become a root rather than a compile error.
    fn default() -> Self {
        Parent(Uuid::NONE)
    }
}

/// Direct children, in attachment order.
///
/// Authoritative for enumeration and redundant with every child's [`Parent`] by
/// design: without it, "the children of X" is a scan of every entity, and the
/// editor's tree, the renderer's culling and the transform walk all ask it once
/// a frame. [`crate::hierarchy`] reads this; [`crate::hierarchy_ops`] is the
/// only thing that writes it, so the two can never disagree.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Children(pub Vec<Uuid>);

/// The cached world transform, and whether it is still true.
///
/// Cached because it is asked once per drawable per frame and the answer costs
/// a matrix multiply per ancestor. The `stale` flag is a field rather than a
/// separate marker component: adding a component moves the entity to another
/// archetype, which is exactly the invalidation
/// [`crate::deferred`] exists to avoid, and a "is this node dirty" question
/// should not be answered by mutating the world to ask it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WorldTransform {
    matrix: Mat4,
    stale: bool,
}

impl WorldTransform {
    /// An entry that is known to be current.
    pub(crate) fn fresh(matrix: Mat4) -> Self {
        Self {
            matrix,
            stale: false,
        }
    }

    /// An entry that is known to be out of date, keeping the matrix it had.
    ///
    /// The matrix is kept rather than zeroed so a debugger looking at a stale
    /// node shows the last position it was drawn at, which is the number needed
    /// to work out *when* it went wrong.
    pub(crate) fn stale(matrix: Mat4) -> Self {
        Self {
            matrix,
            stale: true,
        }
    }

    /// The cached matrix. **Stale unless [`WorldTransform::is_fresh`]**.
    ///
    /// Named to make the hazard impossible to miss: every call site in this
    /// crate that wants a guaranteed-fresh value calls
    /// [`crate::world_transform::resolve`] instead.
    pub fn matrix(self) -> Mat4 {
        self.matrix
    }

    /// Whether the cached matrix is known to be current.
    pub fn is_fresh(self) -> bool {
        !self.stale
    }

    /// Whether the cached matrix is known to be out of date.
    pub fn is_stale(self) -> bool {
        self.stale
    }
}

impl Default for WorldTransform {
    fn default() -> Self {
        Self::stale(Mat4::IDENTITY)
    }
}

/// The document components, and the two directions between a document and
/// storage.
///
/// One list, three outputs, exactly as `vault::components` does it:
/// [`insert`] stores a [`vault::components::Component`] as its own component
/// type, [`read`] reads an entity's components back out, and [`KINDS`] names
/// the kinds in the order [`read`] emits them.
///
/// ## The order is fixed, and the round trip is therefore conditional
///
/// hecs has no notion of the order components were added in — it groups them by
/// archetype — so an entity's component list cannot come back in the order it
/// went in. [`read`] emits [`KINDS`] order instead, which is the declaration
/// order in `vault::components`. A document that is already in that order round
/// trips byte for byte; a document that is not round trips to the same document
/// with its components reordered, which is the same scene. This is stated
/// rather than hidden because the alternative — storing each entity's component
/// order as data — is a second copy of every component list on disk, to preserve
/// a distinction no consumer reads.
macro_rules! declare_document_components {
    ($( $variant:ident($ty:ty), )*) => {
        /// The component kinds, in the order [`read`] emits them.
        ///
        /// These are `vault`'s variant names, listed here rather than read out
        /// of `vault` because Rust offers no way to enumerate another crate's
        /// enum. `tests/component_set.rs` is what keeps the two lists honest: it
        /// stores one of every kind and reads them all back, so a kind added to
        /// `vault` and forgotten here fails a test rather than quietly
        /// vanishing from a saved scene.
        pub const KINDS: &[&str] = &[$( stringify!($variant) ),*];

        /// Store a document component as its own component type.
        ///
        /// Fails only for a kind `vault` knows and this build does not, which is
        /// reachable the moment `vault` gains a component and `meats` has not
        /// been rebuilt against it. It is an error rather than a silent skip
        /// because the silent version drops data on save: the scene loads, it
        /// renders, and the component is gone with nothing to say so.
        pub fn insert(
            world: &mut crate::World,
            entity: crate::Entity,
            component: &vault::components::Component,
        ) -> Result<(), StoreError> {
            match component {
                $(
                    vault::components::Component::$variant(payload) => world
                        .insert(entity, payload.clone())
                        .map_err(StoreError::Entity),
                )*
                // `vault::components::Component` is `#[non_exhaustive]`, so this
                // arm is both required and reachable. It is how a kind this build
                // does not know reaches the caller instead of vanishing -- and
                // `kind_name` still works on a variant we cannot destructure,
                // which is the whole reason `vault` grew that method.
                _ => Err(StoreError::UnknownComponent {
                    entity: entity.uuid(),
                    kind: component.kind_name(),
                }),
            }
        }

        /// Read an entity's document components back out, in [`KINDS`] order.
        ///
        /// One storage lookup per kind rather than one per component held: a
        /// lookup is a `query_one`, which scans the archetype table for the
        /// component, so asking "what is on this entity" costs a fixed fifteen
        /// instead of growing with the answer. Fifteen kinds times a few hundred
        /// entities is a few thousand of these per save, which is part of why
        /// saving a scene is not a per-frame operation.
        pub fn read(
            world: &crate::World,
            entity: crate::Entity,
        ) -> Vec<vault::components::Component> {
            let mut found = Vec::new();
            $(
                if let Some(payload) = world.component::<$ty>(entity) {
                    found.push(vault::components::Component::$variant(payload.clone()));
                }
            )*
            found
        }
    };
}

declare_document_components! {
    Sprite(Sprite),
    Camera2d(Camera2d),
    Model3d(Model3d),
    DirectionalLight(DirectionalLight),
    PointLight(PointLight),
    SpotLight(SpotLight),
    RigidBody3d(RigidBody),
    BoxCollider3d(BoxCollider3d),
    SphereCollider3d(SphereCollider3d),
    CapsuleCollider3d(CapsuleCollider3d),
    RigidBody2d(RigidBody2d),
    BoxCollider2d(BoxCollider2d),
    SpriteAnimation(SpriteAnimation),
    Text(Text),
    NativeScript(NativeScript),
    Animator(Animator),
}

/// Why a document component could not be stored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StoreError {
    /// The entity is not in this world, or the move to another archetype was
    /// refused. Never expected from [`crate::from_doc`], which spawns the entity
    /// immediately before storing on it — which is exactly why it is checked
    /// rather than unwrapped.
    Entity(crate::Error),
    /// `vault` has a component kind this build of `meats` does not store.
    UnknownComponent {
        /// The entity the component was on.
        entity: Uuid,
        /// The kind's name, as `vault` spells it.
        kind: &'static str,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Entity(error) => write!(f, "{error}"),
            StoreError::UnknownComponent { entity, kind } => write!(
                f,
                "{entity:?} has a {kind} component, which this build of meats cannot store"
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Entity(error) => Some(error),
            StoreError::UnknownComponent { .. } => None,
        }
    }
}
