//! The scene document: the data a scene file holds.
//!
//! ## Identifiers are UUIDs, not indices
//!
//! Entities are identified by UUID and the parent is a UUID, **not** an index
//! into a vector. That is inherited from the C++, where `IDComponent` is a
//! `UUID` and `RelationshipComponent::m_ParentID` refers to one, and it is
//! deliberate: an index-based parent means reordering the entity list silently
//! reparents everything.
//!
//! It also means an entity can be moved between scenes without a rewrite, and
//! that a merge of two scenes cannot produce a cycle by accident -- because
//! adding an entity whose parent UUID does not exist leaves it a root rather
//! than corrupting a chain.
//!
//! ## This is a document, not the ECS
//!
//! [`crate::doc::SceneDoc`] is a plain value: no handles, no references, no
//! `Rc`, nothing that needs the ECS to exist. That is what makes it
//! serializable, diffable and testable, and it is why `meats` will *build* its
//! storage from a `SceneDoc` rather than the other way round.

use serde::{Deserialize, Serialize};

/// A 128-bit entity identifier.
///
/// Not a `u64` and not a string: the C++'s `UUID` is a random `u64`, which
/// gives 2^64 values and no way to distinguish "entity 0" from "no entity"
/// without a separate flag. `[u8; 16]` is the same size, self-describing, and
/// serializes compactly under postcard.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Uuid(pub [u8; 16]);

impl Uuid {
    /// The all-zero UUID, used as "no parent" and "unset".
    ///
    /// Chosen over `Option<Uuid>` on the wire because `Option` costs a tag byte
    /// per field and a scene is mostly `None` -- and because a zero UUID can
    /// never be minted, so it is unambiguous by construction rather than by
    /// convention.
    pub const NONE: Self = Self([0; 16]);

    /// Whether this is [`Uuid::NONE`].
    pub fn is_none(&self) -> bool {
        self.0 == Self::NONE.0
    }
}

impl std::fmt::Debug for Uuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Uuid(")?;
        for byte in self.0.iter().take(8) {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "…)")
    }
}

/// Position, rotation (quaternion) and scale.
///
/// A quaternion rather than Euler angles, for the same reason the C++ uses one:
/// Euler angles interpolate wrongly and gimbal-lock, and a scene file is full
/// of rotations.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Transform {
    /// Metres along each axis.
    pub translation: [f32; 3],
    /// Quaternion, `xyzw`.
    pub rotation: [f32; 4],
    /// Multiplier per axis. 1 = unscaled.
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

/// One entity: its identity, its place in the hierarchy, and its components.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct EntityDoc {
    /// This entity's identity, stable across saves.
    pub id: Uuid,
    /// Parent entity. [`Uuid::NONE`] for a root.
    pub parent: Uuid,
    /// Display name. Not unique, not an identifier.
    pub name: String,
    /// Free-form labels. The C++ has a single `TagComponent`; a list is what
    /// every consumer actually wanted, and it costs nothing extra here.
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Local transform relative to the parent.
    pub transform: Transform,
    /// What this entity has.
    pub components: Vec<crate::components::Component>,
}

/// A whole scene.
///
/// Deliberately flat: an array of entities with parent pointers, not a tree of
/// nested entities. Flat means a diff is local, a load does not recurse, and a
/// corrupt parent pointer costs one entity rather than the rest of the file.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct SceneDoc {
    /// Every entity, flat, with parent pointers between them.
    pub entities: Vec<EntityDoc>,
}

impl SceneDoc {
    /// Entities with no parent, in document order.
    pub fn roots(&self) -> impl Iterator<Item = &EntityDoc> {
        self.entities.iter().filter(|e| e.parent.is_none())
    }

    /// Look up an entity by id.
    pub fn get(&self, id: Uuid) -> Option<&EntityDoc> {
        self.entities.iter().find(|e| e.id == id)
    }

    /// Direct children of an entity, in document order.
    pub fn children_of(&self, id: Uuid) -> impl Iterator<Item = &EntityDoc> {
        self.entities.iter().filter(move |e| e.parent == id)
    }

    /// Every component of a given kind on an entity.
    pub fn components_of<T: crate::components::ComponentKind>(&self, id: Uuid) -> Option<&T> {
        self.get(id)?
            .components
            .iter()
            .find_map(|component| component.as_kind::<T>())
    }
}

/// Why a document was rejected before it reached the ECS.
///
/// Separate from [`crate::header::Incompatible`] because this one is about the
/// *contents* being self-inconsistent, which a header cannot catch: a file can
/// have a perfect header and still contain a parent pointer to an entity that
/// is not there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InvalidScene {
    /// Two entities share a UUID.
    DuplicateId {
        /// The repeated value.
        id: Uuid,
    },
    /// An entity's parent UUID matches no entity.
    DanglingParent {
        /// The entity with the bad pointer.
        entity: Uuid,
        /// The parent it points at.
        parent: Uuid,
    },
    /// The parent chain contains a cycle.
    ///
    /// Reachable because the format is flat and deliberately does not prevent
    /// it, and a cycle here is an infinite loop in every consumer that walks
    /// the hierarchy.
    Cycle {
        /// One entity on the cycle.
        involved: Uuid,
    },
    /// The same component appears twice on one entity.
    DuplicateComponent {
        /// The entity with the duplicate.
        entity: Uuid,
        /// Which kind is duplicated.
        kind: &'static str,
    },
}

impl std::fmt::Display for InvalidScene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvalidScene::DuplicateId { id } => {
                write!(f, "two entities share the id {id:?}")
            }
            InvalidScene::DanglingParent { entity, parent } => write!(
                f,
                "entity {entity:?} names parent {parent:?}, which is not in this scene"
            ),
            InvalidScene::Cycle { involved } => write!(
                f,
                "the parent chain containing {involved:?} is a cycle; the hierarchy would \
                 never terminate"
            ),
            InvalidScene::DuplicateComponent { entity, kind } => {
                write!(f, "entity {entity:?} has two {kind} components")
            }
        }
    }
}

impl std::error::Error for InvalidScene {}

impl SceneDoc {
    /// Check the invariants a consumer would otherwise trip over.
    ///
    /// **Not** called on decode by default. A file that fails this is still
    /// worth loading -- the editor's job is to open a broken scene, not to
    /// refuse it -- so it is a separate call the caller chooses. The loader
    /// reports it; the ECS builder is what refuses.
    pub fn validate(&self) -> Result<(), InvalidScene> {
        use std::collections::HashSet;

        let mut seen = HashSet::with_capacity(self.entities.len());
        for entity in &self.entities {
            if !seen.insert(entity.id) {
                return Err(InvalidScene::DuplicateId { id: entity.id });
            }
        }

        for entity in &self.entities {
            if !entity.parent.is_none() && !seen.contains(&entity.parent) {
                return Err(InvalidScene::DanglingParent {
                    entity: entity.id,
                    parent: entity.parent,
                });
            }
        }

        // Cycle detection: walk each entity to its root, remembering who we
        // visited on this particular walk. A shared `visited` set would report
        // a DAG as a cycle, which is the bug in the obvious implementation.
        for entity in &self.entities {
            let mut on_this_path = HashSet::new();
            let mut cursor = entity.id;
            loop {
                if !on_this_path.insert(cursor) {
                    return Err(InvalidScene::Cycle { involved: cursor });
                }
                match self.get(cursor) {
                    Some(current) if !current.parent.is_none() => cursor = current.parent,
                    _ => break,
                }
            }
        }

        Ok(())
    }
}
