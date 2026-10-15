//! The hierarchy: who is under whom, and how to ask.
//!
//! ## Reads only. Writes are [`crate::hierarchy_ops`]'s job.
//!
//! Every function here takes `&World` and returns a value. That split is the
//! point: the queries in this file are the ones a renderer, an editor tree and
//! the transform walk all call, and if they could also change the hierarchy then
//! "read the tree" and "edit the tree" would need the same borrow and the same
//! review. Reparenting is in [`crate::hierarchy_ops`] because it has to decide
//! things — refuse a cycle, preserve a world transform, repair a stale link —
//! and a query should never have to.
//!
//! ## Links are UUIDs, and a dangling one means root
//!
//! [`Parent`] holds a UUID, [`Children`] holds UUIDs, and a [`Parent`] naming an
//! entity that is not here reads as "no parent". That is not a fallback bolted
//! on to make a broken document load; it is what makes a *merge* of two scene
//! files safe. Two people editing one scene produce a document where one
//! entity's parent UUID does not exist, and the correct answer for a merge
//! conflict is "that node is a root now", not "the loader failed" and not "the
//! chain through it is corrupt".
//!
//! ## Every walk here is iterative, and bounded
//!
//! The C++ walks the parent chain recursively, with `const int maxDepth = 4096`
//! as a cycle guard in `Scene::GetWorldSpaceTransformMatrix` and again in
//! `Scene::IsEntityParentOf` — two copies of the same guard. Recursion here
//! would be a stack-depth bug on a deep glTF hierarchy, so these walks use an
//! explicit stack and a visited set instead. The visited set is what makes a
//! pre-existing cycle terminate; the C++'s depth counter only makes it stop
//! early, and returns "not a descendant" from a loop it did not recognise.

use std::collections::HashSet;

use crate::components::{Children, Parent};
use crate::entity::Entity;
use crate::world::World;
use vault::doc::Uuid;

/// The parent entity's id, or [`Uuid::NONE`] for a root.
///
/// [`Uuid::NONE`] rather than `Option` because the document has no
/// `Option<Uuid>` either — a scene is mostly parentless entities, and a tag byte
/// per field to say so is a byte per field for the whole file. The C++ uses `0`
/// for the same reason.
pub fn parent(world: &World, entity: Entity) -> Uuid {
    world
        .component::<Parent>(entity)
        .map(|parent| parent.0)
        .unwrap_or(Uuid::NONE)
}

/// Direct children, in attachment order.
///
/// Empty for an entity that is not here, because a missing entity has no
/// children and an `Option` here would put a branch in every caller's loop.
pub fn children(world: &World, entity: Entity) -> Vec<Entity> {
    child_ids(world, entity)
        .iter()
        .copied()
        .map(Entity::from_uuid)
        .collect()
}

/// Direct children as ids, borrowed.
///
/// The allocating [`children`] is what a caller usually wants; this is what the
/// walks in this crate use, because a per-frame traversal that allocates a
/// `Vec` per node is the difference between a transform cache that is free and
/// one that allocates more than it computes.
pub fn child_ids(world: &World, entity: Entity) -> &[Uuid] {
    world
        .component::<Children>(entity)
        .map(|children| children.0.as_slice())
        .unwrap_or(&[])
}

/// Whether `entity` has no parent, or a parent that is not here.
pub fn is_root(world: &World, entity: Entity) -> bool {
    let parent = parent(world, entity);
    parent.is_none() || !world.contains(Entity::from_uuid(parent))
}

/// Every entity that is a root, in [`crate::Sequence`] order.
///
/// Computed from [`Parent`] rather than by subtracting the child list from the
/// entity list. The subtraction is the bug the C++ already fixed once — its
/// `Scene::GetRootEntities` comment says stale child entries "prevent ghost
/// duplicates when a stale child entry exists on two parents" — and the way it
/// fixed it was to make `Parent` authoritative and never trust `Children`.
pub fn roots(world: &World) -> Vec<Entity> {
    world
        .entities()
        .into_iter()
        .filter(|entity| is_root(world, *entity))
        .collect()
}

/// Every entity below `entity`, at any depth, excluding `entity` itself.
///
/// Depth first, in attachment order, so a caller that draws a tree draws it in
/// the order it is stored. Stops at a node it has already visited, which is what
/// a cycle needs and is why this cannot spin.
pub fn descendants(world: &World, entity: Entity) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(entity.uuid());
    let mut stack: Vec<Entity> = children(world, entity).into_iter().rev().collect();
    while let Some(current) = stack.pop() {
        if !seen.insert(current.uuid()) {
            continue;
        }
        found.push(current);
        for child in children(world, current).into_iter().rev() {
            stack.push(child);
        }
    }
    found
}

/// The chain from the root down to `entity`, inclusive.
///
/// `None` if `entity` is not here or the chain does not terminate — a cycle, or
/// a parent that is not here part-way up. A partial path would be worse than
/// none: a tree widget that draws half a chain hides the part it got wrong.
pub fn path_from_root(world: &World, entity: Entity) -> Option<Vec<Entity>> {
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = entity;
    loop {
        if !seen.insert(cursor.uuid()) {
            return None;
        }
        path.push(cursor);
        let parent = parent(world, cursor);
        if parent.is_none() {
            return Some(path);
        }
        let parent = Entity::from_uuid(parent);
        if !world.contains(parent) {
            // A dangling parent is a root, so the chain above here is the whole
            // chain -- but the entity that was asked for is genuinely absent.
            return if cursor == entity { Some(path) } else { None };
        }
        cursor = parent;
    }
}

/// How deep `entity` is below a root. `None` if it is not here.
pub fn depth(world: &World, entity: Entity) -> Option<usize> {
    path_from_root(world, entity).map(|path| path.len() - 1)
}

/// Whether `ancestor` is strictly above `entity`.
///
/// False for an entity against itself, which is what "strictly above" means and
/// is the answer the cycle check in [`crate::hierarchy_ops`] needs.
pub fn is_ancestor_of(world: &World, ancestor: Entity, entity: Entity) -> bool {
    if ancestor == entity {
        return false;
    }
    let mut seen = HashSet::new();
    let mut cursor = entity;
    while seen.insert(cursor.uuid()) {
        let parent = parent(world, cursor);
        if parent.is_none() {
            return false;
        }
        let parent = Entity::from_uuid(parent);
        if parent == ancestor {
            return true;
        }
        if !world.contains(parent) {
            return false;
        }
        cursor = parent;
    }
    false
}
