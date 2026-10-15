//! Changing the hierarchy: reparenting, detaching, and refusing the changes that
//! would break it.
//!
//! ## A cycle is refused, not noticed afterwards
//!
//! Reparenting a node under its own descendant is the one operation in this
//! crate that can make the data structure *wrong* rather than merely
//! wrong-looking, because every walk in [`crate::hierarchy`] — roots,
//! descendants, paths, world transforms — is defined only for a forest. The C++
//! checks for it too (`Scene::ReparentEntity` calls `IsEntityParentOf` and warns)
//! but its check is a bounded walk with `const int maxDepth = 4096`, so a cycle
//! deeper than that passes the check and is then an infinite loop in every other
//! consumer. This one walks with a visited set and refuses.
//!
//! The refusal is an `Err`, not a warning. `Scene::ReparentEntity` is a `void`
//! method called from a UI handler that ignores it, so the editor's drag-drop has
//! no way to say "that is not allowed" — the drop silently does nothing and looks
//! broken. Here the caller gets to decide, and the editor is a caller.
//!
//! ## World transforms are preserved on a reparent, and only there
//!
//! A reparent changes what a local transform *means*, so [`reparent`] rewrites it
//! to keep the entity where it is on screen. [`attach`] does not: a fresh child
//! should appear at its parent's origin, and preserving a world transform for an
//! entity that has never been drawn means preserving a world transform of the
//! world origin — which cancels the parent's and puts it back at (0,0,0). That is
//! the distinction the C++ encodes in a bool: `m_PendingReparents`' `PreserveWorld`.
//!
//! A reparent whose local transform cannot be recovered still re-parents, and
//! reports [`ReparentError::NotInvertible`]. Refusing the move instead would leave
//! the user's drag with no effect at all, which is the "looks broken" outcome
//! this file exists to prevent — a node in the wrong place is debuggable, a node
//! that will not move is not.

use std::collections::HashSet;

use crate::components::{Children, Parent};
use crate::entity::Entity;
use crate::hierarchy;
use crate::world::World;
use crate::world_transform::{self, SetFromWorld};
use vault::doc::Uuid;

/// Why a hierarchy change did not happen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReparentError {
    /// The handle names no entity in this world.
    Unknown(Entity),
    /// An entity cannot be its own parent.
    SelfParent {
        /// The entity that was being moved.
        entity: Entity,
    },
    /// The proposed parent is below the entity being moved, which would make the
    /// chain a loop: the node would become its own ancestor, and every walk of
    /// the hierarchy would never terminate.
    DescendantOfSelf {
        /// The entity being moved.
        child: Entity,
        /// The node it would have been moved under.
        parent: Entity,
    },
    /// The hierarchy is *already* cyclic, and this operation cannot say where the
    /// loop begins.
    ///
    /// Reachable only if something wrote [`Parent`] directly — every mutation in
    /// this crate refuses to create one — so in practice it reports a bug
    /// elsewhere. It is a variant rather than an assumption because the
    /// alternative is an unbounded walk looking for a cycle nobody asked about.
    AlreadyCyclic {
        /// An entity on the loop.
        entity: Entity,
    },
    /// The new local transform could not be recovered from the world transform.
    ///
    /// Only from a parent whose world matrix is not invertible — zero scale on an
    /// axis. The move happened; only the local transform was left alone.
    NotInvertible {
        /// The entity whose transform was left alone.
        entity: Entity,
    },
}

impl std::fmt::Display for ReparentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReparentError::Unknown(entity) => write!(f, "{entity} is not in this world"),
            ReparentError::SelfParent { entity } => write!(f, "{entity} cannot be its own parent"),
            ReparentError::DescendantOfSelf { child, parent } => write!(
                f,
                "{child} cannot be moved under {parent}, which is below {child}: the chain would \
                 never end"
            ),
            ReparentError::AlreadyCyclic { entity } => write!(
                f,
                "the hierarchy already loops through {entity}; this operation did not create it \
                 and cannot repair it"
            ),
            ReparentError::NotInvertible { entity } => write!(
                f,
                "{entity} kept its local transform: its new parent's world transform cannot be \
                 undone"
            ),
        }
    }
}

impl std::error::Error for ReparentError {}

/// Move `child` under `parent`, keeping it where it is on screen.
///
/// What an editor drag-drop means. The world transform is captured before the
/// move and re-derived from the new parent's afterwards, so the node does not
/// jump — and because [`LocalTransform`](crate::components::LocalTransform) is a
/// quaternion rather than Euler angles, a reparent in the middle of a drag does
/// not accumulate the round-trip error three Euler angles make.
pub fn reparent(world: &mut World, child: Entity, parent: Entity) -> Result<(), ReparentError> {
    check(world, child, parent)?;
    let before = resolve(world, child)?;
    link(world, child, parent);
    match world_transform::set_local_from_world(world, child, before) {
        Ok(SetFromWorld::Moved) => Ok(()),
        Ok(SetFromWorld::Undecomposable) => Err(ReparentError::NotInvertible { entity: child }),
        Err(error) if error.is_unknown_entity() => Err(ReparentError::Unknown(child)),
        Err(_) => Err(ReparentError::NotInvertible { entity: child }),
    }
}

/// Move `child` under `parent` without touching its local transform.
///
/// For a child that is being created, and for an editor that means "put this
/// here" literally rather than "put this here, in the same place". The C++ calls
/// this `AttachChildEntity` and reaches it through the same queue as a reparent,
/// distinguished by a bool.
pub fn attach(world: &mut World, child: Entity, parent: Entity) -> Result<(), ReparentError> {
    check(world, child, parent)?;
    link(world, child, parent);
    Ok(())
}

/// Make `child` a root, keeping it where it is on screen.
///
/// Children stay attached. The C++ fixed this: `CollapseEntity` records that the
/// old implementation re-parented every child to the grandparent, "which
/// scattered subtrees" — a drag that detached a parent silently rearranged
/// everything under it.
pub fn detach(world: &mut World, child: Entity) -> Result<(), ReparentError> {
    if !world.contains(child) {
        return Err(ReparentError::Unknown(child));
    }
    // A stale entry can leave an entity listed under its own `Children`, so its
    // parent link resolves to itself. Repair that before detaching, or
    // "detached from itself" is what happens.
    if let Some(Children(children)) = world.component_mut::<Children>(child) {
        let own = child.uuid();
        children.retain(|sibling| *sibling != own);
    }

    let before = resolve(world, child)?;
    unlink(world, child);
    match world_transform::set_local_from_world(world, child, before) {
        Ok(SetFromWorld::Moved) => Ok(()),
        Ok(SetFromWorld::Undecomposable) => Err(ReparentError::NotInvertible { entity: child }),
        Err(error) if error.is_unknown_entity() => Err(ReparentError::Unknown(child)),
        Err(_) => Err(ReparentError::NotInvertible { entity: child }),
    }
}

/// Queue a reparent for the next [`World::flush_mutations`].
///
/// The checks run *now* and the move happens later, so the editor that gets an
/// `Err` here can refuse the drop and say why — the complaint about
/// `Scene::ReparentEntity` being `void` is that it cannot do that. A move that
/// passes the checks and then fails at flush time can only fail because the world
/// changed underneath it, and that failure is in
/// [`World::take_failures`](crate::World::take_failures).
pub fn queue_reparent(
    world: &mut World,
    child: Entity,
    parent: Entity,
) -> Result<(), ReparentError> {
    check(world, child, parent)?;
    world.queue_mutation(move |world| reparent(world, child, parent).map_err(Into::into));
    Ok(())
}

/// Queue a detach for the next [`World::flush_mutations`].
pub fn queue_detach(world: &mut World, child: Entity) -> Result<(), ReparentError> {
    if !world.contains(child) {
        return Err(ReparentError::Unknown(child));
    }
    world.queue_mutation(move |world| detach(world, child).map_err(Into::into));
    Ok(())
}

/// Despawn `entity` and everything below it. Returns how many went.
///
/// Iterative, children first, so a deep hierarchy does not recurse and a parent
/// never outlives its children's link to it. The C++'s `DestroyEntity` does the
/// same walk recursively, which is a stack depth per level.
pub fn despawn_subtree(world: &mut World, entity: Entity) -> Result<usize, ReparentError> {
    if !world.contains(entity) {
        return Err(ReparentError::Unknown(entity));
    }
    let mut seen = HashSet::new();
    let mut stack = vec![entity];
    let mut post_order = Vec::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current.uuid()) {
            // A cycle cannot be created by this crate, but `descendants` and this
            // walk must both terminate if something writes `Parent` directly.
            continue;
        }
        post_order.push(current);
        stack.extend(hierarchy::children(world, current));
    }
    post_order.reverse();

    let mut removed = 0;
    for victim in post_order {
        if world.despawn(victim).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// [`world_transform::resolve`] for a handle `check` has already proved is here.
///
/// The mapping is total in practice: every `Error` `resolve` can return for an
/// entity `check` accepted is "that entity is gone", and `check` ran on the same
/// borrow. It is a function so both call sites say that once.
fn resolve(world: &mut World, entity: Entity) -> Result<glam::Mat4, ReparentError> {
    world_transform::resolve(world, entity).map_err(|_| ReparentError::Unknown(entity))
}

/// The check every move shares, run before anything is written.
///
/// Refuses self-parenting, refuses a move under a descendant, and refuses to do
/// arithmetic on a hierarchy that is already cyclic. `Ok` means the move is
/// structurally possible; whether the local transform survives it is
/// [`world_transform`]'s problem, reported separately.
fn check(world: &World, child: Entity, parent: Entity) -> Result<(), ReparentError> {
    if !world.contains(child) {
        return Err(ReparentError::Unknown(child));
    }
    if !world.contains(parent) {
        return Err(ReparentError::Unknown(parent));
    }
    if child == parent {
        return Err(ReparentError::SelfParent { entity: child });
    }

    // Walk up from the proposed parent. Reaching the child means the new parent is
    // below it, and the move would close a loop.
    let mut seen = HashSet::new();
    let mut cursor = parent;
    loop {
        if !seen.insert(cursor.uuid()) {
            // The chain being walked is already a loop, so there is no root to
            // find and no way to say where it starts. Refuse rather than guess.
            return Err(ReparentError::AlreadyCyclic { entity: cursor });
        }
        if cursor == child {
            return Err(ReparentError::DescendantOfSelf { child, parent });
        }
        let above = hierarchy::parent(world, cursor);
        if above.is_none() {
            return Ok(());
        }
        let above = Entity::from_uuid(above);
        if !world.contains(above) {
            // A dangling parent is a root: the chain ends here.
            return Ok(());
        }
        cursor = above;
    }
}

/// Write the link: unhook from the old parent, hook onto the new one.
fn link(world: &mut World, child: Entity, parent: Entity) {
    if let Some(Children(existing)) = world.component::<Children>(parent) {
        if existing.contains(&child.uuid()) {
            // Already linked. `Scene::ReparentEntity` guards this because an
            // editor drag-drop can fire twice for one gesture — both the source
            // and the target accept it — and doing the work twice would list the
            // child twice and draw it twice.
            return;
        }
    }
    unlink(world, child);
    if let Some(link) = world.component_mut::<Parent>(child) {
        *link = Parent(parent.uuid());
    }
    if let Some(Children(children)) = world.component_mut::<Children>(parent) {
        children.push(child.uuid());
    }
    // The node and everything under it are somewhere else now, whatever happened
    // to their local transforms.
    world_transform::mark_stale(world, child);
}

/// Remove `child` from whatever parent it has, making it a root.
fn unlink(world: &mut World, child: Entity) {
    let previous = hierarchy::parent(world, child);
    if let Some(Children(siblings)) = world.component_mut::<Children>(Entity::from_uuid(previous)) {
        siblings.retain(|sibling| *sibling != child.uuid());
    }
    if let Some(link) = world.component_mut::<Parent>(child) {
        *link = Parent(Uuid::NONE);
    }
}
