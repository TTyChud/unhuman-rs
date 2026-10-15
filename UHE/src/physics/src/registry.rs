//! The key → handle map: femur's single owner of solver state.
//!
//! # Why the runtime handle does not live in the scene component
//!
//! The C++ stores it there — `RigidBody3DComponent::RuntimeBodyID`,
//! `RigidBody2DComponent::RuntimeBody` — which makes one struct both the scene
//! data and the runtime data, and leaves a solver handle sitting in the middle
//! of something the serialiser writes to disk. It also means the free list is
//! wherever the component is, so "this body is gone" and "this component is
//! gone" are different facts that have to be kept in step by hand.
//!
//! Here the scene document holds nothing solver-specific and this map is the
//! only thing a despawn has to go through. A handle that lives in two places is
//! a handle that can be freed in one of them.
//!
//! # Why insert refuses to overwrite
//!
//! Overwriting a key looks harmless and is the most likely way to leak: the
//! entry is replaced, so the old solver body becomes unreachable *and* keeps
//! being simulated, and the only symptom is an editor whose frame time creeps
//! upward as you drag entities around. Refusing turns that into an `Err` at the
//! call site, which is the one place that can fix it.

use std::collections::HashMap;

use rapier2d::dynamics::RigidBodyHandle as Handle2;
use rapier2d::geometry::ColliderHandle as Collider2;
use rapier3d::dynamics::RigidBodyHandle as Handle3;
use rapier3d::geometry::ColliderHandle as Collider3;

use crate::bodies::{BodyId, SpawnError};

/// What one entity owns in the solver.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BodySlot {
    /// The body handle.
    pub body: Handle3,
    /// The collider handle, if the entity has one.
    pub collider: Option<Collider3>,
}

/// The 3D key → handle map.
///
/// See the module docs for why the runtime handle lives here and not in the
/// scene component.
#[derive(Clone, Debug, Default)]
pub struct BodyRegistry {
    slots: HashMap<BodyId, BodySlot>,
}

impl BodyRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a body. Fails if the key is taken; see the module docs.
    pub fn insert(
        &mut self,
        id: BodyId,
        body: Handle3,
        collider: Option<Collider3>,
    ) -> Result<(), SpawnError> {
        if self.slots.contains_key(&id) {
            return Err(SpawnError::AlreadyExists(id));
        }
        self.slots.insert(id, BodySlot { body, collider });
        Ok(())
    }

    /// Removes a body, returning what it owned so the caller can delete it
    /// from the solver. `None` if the key was not registered, which makes
    /// despawn idempotent rather than panicking.
    pub fn remove(&mut self, id: BodyId) -> Option<BodySlot> {
        self.slots.remove(&id)
    }

    /// The solver handle for a key.
    pub fn get(&self, id: BodyId) -> Option<BodySlot> {
        self.slots.get(&id).copied()
    }

    /// Whether a key is registered. The check that must exist, because a
    /// runtime handle in a freed slot is silently addressable in rapier's
    /// generational scheme only until the slot is reused.
    pub fn contains(&self, id: BodyId) -> bool {
        self.slots.contains_key(&id)
    }

    /// Moves a collider's ownership from one key to another, which is what
    /// re-parenting a collider entity means physically: the shape does not
    /// change, the body it pushes does.
    ///
    /// Returns `false` if either key is unknown, having changed nothing.
    pub fn reparent_collider(&mut self, from: BodyId, to: BodyId) -> bool {
        if from == to {
            return false;
        }
        let Some(collider) = self.slots.get(&from).and_then(|slot| slot.collider) else {
            return false;
        };
        // The source keeps its key but loses the collider; a shape has one
        // parent at a time, and leaving the old slot pointing at it would make
        // a despawn of `from` take the collider with it.
        if !self.slots.contains_key(&to) {
            return false;
        }
        if let Some(source) = self.slots.get_mut(&from) {
            source.collider = None;
        }
        if let Some(target) = self.slots.get_mut(&to) {
            target.collider = Some(collider);
        } else {
            return false;
        }
        true
    }

    /// How many bodies are registered.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Every registered key with what it owns, in unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (BodyId, BodySlot)> + '_ {
        self.slots.iter().map(|(k, v)| (*k, *v))
    }
}

/// The 2D form of [`BodyRegistry`].
///
/// A separate type rather than a `BodyRegistry` with a `#[cfg]`: the two hold
/// different handle types, and a generic registry would need a second type
/// parameter to make that work, which buys nothing for two dozen lines.
#[derive(Clone, Debug, Default)]
pub struct BodyRegistry2 {
    slots: HashMap<BodyId, BodySlot2>,
}

/// What one 2D entity owns in the solver.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BodySlot2 {
    /// The body handle.
    pub body: Handle2,
    /// The collider handle, if the entity has one.
    pub collider: Option<Collider2>,
}

impl BodyRegistry2 {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a body. Fails if the key is taken.
    pub fn insert(
        &mut self,
        id: BodyId,
        body: Handle2,
        collider: Option<Collider2>,
    ) -> Result<(), SpawnError> {
        if self.slots.contains_key(&id) {
            return Err(SpawnError::AlreadyExists(id));
        }
        self.slots.insert(id, BodySlot2 { body, collider });
        Ok(())
    }

    /// Removes a body, returning what it owned.
    pub fn remove(&mut self, id: BodyId) -> Option<BodySlot2> {
        self.slots.remove(&id)
    }

    /// The solver handles for a key.
    pub fn get(&self, id: BodyId) -> Option<BodySlot2> {
        self.slots.get(&id).copied()
    }

    /// Whether a key is registered.
    pub fn contains(&self, id: BodyId) -> bool {
        self.slots.contains_key(&id)
    }

    /// How many bodies are registered.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Every registered key with what it owns.
    pub fn iter(&self) -> impl Iterator<Item = (BodyId, BodySlot2)> + '_ {
        self.slots.iter().map(|(k, v)| (*k, *v))
    }
}
