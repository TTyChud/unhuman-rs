//! The cached world transform, and the dirty flag that says when it is a lie.
//!
//! ## Why it is cached
//!
//! A world transform is the product of the local transforms of a node and every
//! ancestor, in that order. Every drawable asks for it once a frame, and asking
//! costs a matrix multiply per level of the hierarchy: a Mixamo rig with a
//! hundred-deep bone chain would cost a hundred multiplies per vertex-batched
//! draw. The C++ caches exactly this in `m_WorldTransformCache` and rebuilds it
//! each frame in `UpdateWorldTransformCache`, which is O(n) with an iterative
//! DFS — a fix for the recursive version's stack depth, noted in
//! `docs/PARITY.md` under animation as a bug to carry the lesson from rather than
//! copy.
//!
//! ## Why the dirty flag is a field and not a marker component
//!
//! "Is this node stale?" is a question about the world, so the obvious ECS answer
//! is a `TransformDirty` component that exists only when the answer is yes. That
//! answer is wrong here: inserting a component moves the entity to another
//! archetype, so *marking* a node dirty would invalidate the very views the editor
//! is walking while it finds out what is dirty — the hazard
//! [`crate::deferred`] exists for, reached by asking a question. A bool inside the
//! component costs nothing and changes no shape.
//!
//! ## The contract
//!
//! * [`update`] is O(1) when nothing is dirty and O(n) when something is.
//! * [`resolve`] never returns a stale matrix. It is what a system should call.
//! * [`of`] returns whatever is cached, stale or not. It exists for walks that
//!   are *about* the cache — a profiler counting dirty nodes, a test asserting a
//!   flag — and it is the reason the type is named for the cache rather than for
//!   the answer.
//!
//! Any local-transform change marks the node and its whole subtree stale. Not
//! lazily, and not only the node: a lazy scheme is one where the first frame
//! after an edit shows a child in the wrong place, which is the bug
//! `docs/PARITY.md` calls "one frame late" about input.

use std::collections::HashSet;

use glam::{Mat4, Quat, Vec3};
use vault::doc::Transform;

use crate::components::{LocalTransform, WorldTransform as Cache};
use crate::entity::Entity;
use crate::hierarchy;
use crate::world::{Error, World};

/// A transform decomposed into the three parts a scene file stores.
fn to_parts(local: &Transform) -> (Vec3, Quat, Vec3) {
    (
        Vec3::from_array(local.translation),
        Quat::from_xyzw(
            local.rotation[0],
            local.rotation[1],
            local.rotation[2],
            local.rotation[3],
        ),
        Vec3::from_array(local.scale),
    )
}

fn from_parts(translation: Vec3, rotation: Quat, scale: Vec3) -> Transform {
    Transform {
        translation: translation.to_array(),
        rotation: rotation.to_array(),
        scale: scale.to_array(),
    }
}

/// `T * R * S`, the composition order the C++'s `TransformComponent::GetTransform`
/// builds and the one every other engine builds.
///
/// Order is load-bearing: scale applied before rotation shears the rotation, and
/// translation before scale scales the position. Getting this wrong looks like a
/// subtle scale bug for months.
fn compose(transform: &Transform) -> Mat4 {
    let (translation, rotation, scale) = to_parts(transform);
    Mat4::from_scale_rotation_translation(scale, rotation, translation)
}

/// The local transform of `entity`.
pub fn local(world: &World, entity: Entity) -> Option<Transform> {
    world
        .component::<LocalTransform>(entity)
        .map(|local| local.0)
}

/// Set the local transform, and mark the subtree's cached world transforms stale.
///
/// A write through a mutable borrow rather than an insert: the component is
/// always there, and re-inserting it would move the entity to another archetype
/// for no reason.
pub fn set_local(world: &mut World, entity: Entity, transform: Transform) -> Result<(), Error> {
    match world.component_mut::<LocalTransform>(entity) {
        Some(slot) => slot.0 = transform,
        None => return Err(Error::UnknownEntity(entity)),
    }
    mark_stale(world, entity);
    Ok(())
}

/// Queue a local-transform change for the next
/// [`World::flush_mutations`](crate::World::flush_mutations).
///
/// What a transform gizmo calls. The gizmo is drawn from inside the renderer's
/// world-transform walk, so writing the transform it is dragging cannot happen
/// there — the same reason `Scene::QueueReparent` exists for a drag in the tree.
pub fn queue_set_local(world: &mut World, entity: Entity, transform: Transform) {
    world.queue_mutation(move |world| set_local(world, entity, transform));
}

/// Recompute every stale world transform, and return how many were written.
///
/// Roots first, then depth first through the subtree, so a parent's matrix is
/// already correct when its children are visited — which is what lets a dirty
/// child be rebuilt without walking up its chain.
///
/// Returns 0 without touching the world when nothing is dirty, which is the
/// common case in a running game and the reason a per-frame call is affordable.
pub fn update(world: &mut World) -> usize {
    if stale_count(world) == 0 {
        return 0;
    }

    let roots = hierarchy::roots(world);
    let mut written = 0;
    let mut visited = HashSet::with_capacity(world.len());
    // Explicit stack, so a hierarchy deeper than the call stack cannot overflow
    // it. Pushed in reverse so they pop in order, which makes the traversal order
    // the storage order and a test's expected values readable.
    let mut stack: Vec<(Entity, Mat4)> = roots
        .into_iter()
        .rev()
        .map(|root| (root, Mat4::IDENTITY))
        .collect();

    while let Some((entity, parent_world)) = stack.pop() {
        if !visited.insert(entity.uuid()) {
            continue;
        }
        // A node with no local transform contributes identity, exactly as the
        // C++'s `UpdateWorldTransformCache` does when `HasComponent` fails.
        let local = match world.component::<LocalTransform>(entity) {
            Some(local) => compose(&local.0),
            None => Mat4::IDENTITY,
        };
        let world_matrix = parent_world * local;
        let dirty = is_stale(world, entity);
        if dirty {
            if let Some(cache) = world.component_mut::<Cache>(entity) {
                *cache = Cache::fresh(world_matrix);
            }
            written += 1;
        }
        for child in hierarchy::child_ids(world, entity).iter().rev() {
            stack.push((Entity::from_uuid(*child), world_matrix));
        }
    }
    written
}

/// The cached world transform of `entity`, **which may be stale**.
///
/// For code that is about the cache — a profiler counting dirty nodes, a test
/// asserting a flag, an editor drawing stale transforms in a warning colour.
/// Everything else wants [`resolve`], which cannot hand back a stale matrix.
pub fn of(world: &World, entity: Entity) -> Option<Mat4> {
    world.component::<Cache>(entity).map(|cache| cache.matrix())
}

/// Whether `entity`'s cached world transform is out of date.
///
/// `true` for an entity that has never been updated, which is the honest answer:
/// it has no world transform to be right about.
pub fn is_stale(world: &World, entity: Entity) -> bool {
    world
        .component::<Cache>(entity)
        .map(|cache| cache.is_stale())
        .unwrap_or(true)
}

/// How many entities are out of date.
pub fn stale_count(world: &World) -> usize {
    let mut count = 0;
    for entity in world.entities() {
        if is_stale(world, entity) {
            count += 1;
        }
    }
    count
}

/// Mark `entity` and everything below it out of date.
///
/// The subtree, not just the node: a parent's world transform is part of every
/// descendant's. Walking the subtree here is what makes the propagation eager,
/// and eager is the point — see the note about one-frame-late at the top.
pub fn mark_stale(world: &mut World, entity: Entity) {
    let mut stack = vec![entity];
    let mut seen = HashSet::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current.uuid()) {
            continue;
        }
        if let Some(cache) = world.component_mut::<Cache>(current) {
            *cache = Cache::stale(cache.matrix());
        }
        for child in hierarchy::child_ids(world, current) {
            stack.push(Entity::from_uuid(*child));
        }
    }
}

/// The world transform of `entity`, fresh whatever it takes.
///
/// Returns the cache when it is current. Otherwise it walks up the parent chain,
/// multiplies down and writes the answers back, which makes the node and every
/// node above it current. Descendants are left alone: their caches are flagged
/// only when something they depend on actually moved, and a node whose local
/// transform changed is still wrong however correct its ancestors now are.
///
/// Writing back during a read is safe here precisely because nothing changes
/// shape: [`Cache`] is always present, so assigning to it moves no rows. It is
/// also why this can take `&mut World` while [`of`] cannot.
pub fn resolve(world: &mut World, entity: Entity) -> Result<Mat4, Error> {
    if !world.contains(entity) {
        return Err(Error::UnknownEntity(entity));
    }
    if !is_stale(world, entity) {
        return of(world, entity).ok_or(Error::UnknownEntity(entity));
    }

    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = entity;
    while seen.insert(cursor.uuid()) {
        chain.push(cursor);
        let parent = hierarchy::parent(world, cursor);
        if parent.is_none() {
            break;
        }
        let parent = Entity::from_uuid(parent);
        if !world.contains(parent) {
            break;
        }
        cursor = parent;
    }

    let mut world_matrix = Mat4::IDENTITY;
    for node in chain.iter().rev() {
        if let Some(local) = world.component::<LocalTransform>(*node) {
            world_matrix *= compose(&local.0);
        }
        // A node whose ancestors were just multiplied is now correct whether or not
        // its own cache was stale, so this is where the subtree gets repaired.
        if let Some(cache) = world.component_mut::<Cache>(*node) {
            *cache = Cache::fresh(world_matrix);
        }
    }
    Ok(world_matrix)
}

/// What happened when a local transform was recovered from a world transform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetFromWorld {
    /// The local transform was rewritten, so the entity is where `world_matrix`
    /// said it should be.
    Moved,
    /// The matrix has no decomposition — a negative determinant or shear, which a
    /// user can type into a transform field with one zero scale on it.
    ///
    /// There is no honest local transform for such a matrix, so the old one stays
    /// and the cache entry is marked stale. Reporting this rather than guessing
    /// is what [`crate::hierarchy_ops::ReparentError::NotInvertible`] is built
    /// on: the reparent happened, and saying so is better than either refusing
    /// the move or quietly storing a mirrored, wrong transform.
    Undecomposable,
}

/// Set `entity`'s local transform so that its world transform is `world_matrix`.
///
/// The inverse of [`resolve`], and the operation that makes a reparent
/// world-preserving. `Err` only for a handle that is not in the world; the
/// caller decides what that means.
pub fn set_local_from_world(
    world: &mut World,
    entity: Entity,
    world_matrix: Mat4,
) -> Result<SetFromWorld, Error> {
    let parent_world = match hierarchy::parent(world, entity) {
        parent if parent.is_none() => Mat4::IDENTITY,
        parent => {
            let parent = Entity::from_uuid(parent);
            if world.contains(parent) {
                resolve(world, parent)?
            } else {
                // A dangling parent is a root.
                Mat4::IDENTITY
            }
        }
    };

    // `try_inverse` first: a parent scaled to zero on an axis has no inverse, and
    // the whole reparent is undefined rather than merely lossy.
    let Some(inverse) = parent_world.try_inverse() else {
        return Ok(SetFromWorld::Undecomposable);
    };
    let local = inverse * world_matrix;
    // `to_scale_rotation_translation` asserts rather than returning an error when
    // the determinant is zero or an axis collapses, so the check is ours to make.
    // A user typing `scale = 0` into a transform field is the reachable case.
    if local.determinant().abs() <= f32::EPSILON {
        return Ok(SetFromWorld::Undecomposable);
    }
    let (scale, rotation, translation) = local.to_scale_rotation_translation();
    if !scale.cmpne(Vec3::ZERO).any() {
        return Ok(SetFromWorld::Undecomposable);
    }

    match world.component_mut::<LocalTransform>(entity) {
        Some(slot) => slot.0 = from_parts(translation, rotation, scale),
        None => return Err(Error::UnknownEntity(entity)),
    }
    // The cache entry is being set to the matrix we were asked for, so it is
    // correct by construction and need not be invalidated: a reparent that
    // preserves the world transform should not dirty the subtree it did not move.
    if let Some(cache) = world.component_mut::<Cache>(entity) {
        *cache = Cache::fresh(world_matrix);
    }
    mark_stale(world, entity);
    Ok(SetFromWorld::Moved)
}
