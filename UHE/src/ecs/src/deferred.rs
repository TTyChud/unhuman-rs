//! The deferred mutation queue.
//!
//! ## Why this exists
//!
//! hecs stores an entity's components in a row of one *archetype*, and an
//! archetype is a table indexed by archetype, not by entity. Insert a component
//! and the entity no longer belongs in that archetype, so it moves: the row is
//! copied into the target archetype's table and the old slot becomes a hole.
//! Any iterator walking that table is now pointing at memory that has been
//! reused for something else.
//!
//! That is not a hecs quirk to be documented around — it is the same archetype
//! swap EnTT does on `emplace`, and it is the entire reason the C++ scene has
//! `m_PendingReparents`, `m_PendingCollapses` and `m_PendingModelExpands`, under
//! a comment saying "component pools are only touched here, outside any registry
//! view iteration, so no entt iterator gets invalidated". The editor asks for a
//! reparent from inside a tree walk, and the request is applied at the top of the
//! next frame.
//!
//! So the queue is kept, and generalised: [`World::queue_mutation`] takes any
//! mutation. What is *not* kept is the C++'s habit of letting a system call
//! `emplace` directly, which is what makes the hazard reachable in the first
//! place.
//!
//! ## What the borrow checker prevents, and what it does not
//!
//! In safe Rust the panic cannot happen: [`World::each`] holds a borrow, so a
//! closure that tried to insert would not compile. What the borrow checker cannot
//! see is the *timing* of a change the user has already made — a drag-drop
//! decided in frame N, wanted in frame N, with frame N's loop already iterating.
//! That is what this queue is for.
//!
//! ## Draining
//!
//! [`World::flush_mutations`] takes the queue out before running it, so a
//! mutation that queues another lands in a fresh queue and is applied on the
//! *next* flush, not recursively inside this one. Two reasons: a queue that can
//! grow while it is being drained has no obvious end, and a change that cascades
//! should be visible to the frame that follows it. The C++ has the same
//! `auto pending = m_PendingReparents; m_PendingReparents.clear();` pair, and a
//! comment claiming the opposite — "the vector can be appended to while we work"
//! — which is not what the copy-then-clear does. The copy-then-clear is right.

use crate::world::{Error, World};

/// A queued change to the world.
///
/// A closure rather than an enum because the set of mutations is open: the editor
/// will want one this crate has never heard of, and an enum would make every new
/// operation a change here. It is given **our** [`World`], not hecs's, so a
/// mutation cannot reach past the wrapper either.
///
/// It returns a [`Result`] because a queue with no error channel is a queue that
/// loses failures silently, and the C++'s is exactly that: `FlushPendingReparents`
/// drops a rejected reparent with nothing but a `UHE_CORE_WARN` inside the call
/// that the caller cannot see. Failures are collected by
/// [`World::flush_mutations`] rather than raised, because one impossible reparent
/// must not abandon the other forty edits made in the same frame.
///
/// `Send + Sync` because the queue outlives the call that filled it: the job
/// system that drains it may be on another thread, and a closure that could not
/// cross would mean the queue can only be flushed from the thread that wrote it,
/// which is the assumption that makes the C++'s editor queue racy.
pub type Mutation = Box<dyn FnOnce(&mut World) -> Result<(), Error> + Send + Sync + 'static>;

impl World {
    /// Queue a mutation for the next [`World::flush_mutations`].
    ///
    /// ```
    /// use meats::{Error, Name, World};
    ///
    /// let mut world = World::new();
    /// let entity = world.spawn();
    ///
    /// // The walk holds the world, so it cannot mutate: it queues instead.
    /// let mut queued = Vec::new();
    /// world.each::<Name>(|_, _| queued.push(()));
    /// assert_eq!(queued.len(), 1);
    ///
    /// world.queue_mutation(move |world| world.insert(entity, Name("renamed".into())));
    /// assert_eq!(world.queued_mutation_count(), 1);
    /// assert_eq!(world.flush_mutations(), 1);
    /// assert!(world.take_failures().is_empty());
    ///
    /// // A mutation naming an entity that is gone fails, and does not stop the
    /// // next one.
    /// let gone = entity;
    /// world.despawn(entity).unwrap();
    /// world.queue_mutation(move |world| world.insert(gone, Name("x".into())));
    /// world.queue_mutation(move |world| Err(Error::UnknownEntity(gone)));
    /// assert_eq!(world.flush_mutations(), 2);
    /// assert_eq!(world.take_failures().len(), 2);
    /// ```
    pub fn queue_mutation(
        &mut self,
        mutation: impl FnOnce(&mut World) -> Result<(), Error> + Send + Sync + 'static,
    ) {
        self.pending.push(Box::new(mutation));
    }

    /// How many mutations are waiting.
    ///
    /// For a diagnostics panel, and for a test that wants to assert a request was
    /// *made*. Nothing in the engine branches on it: whether a frame applies its
    /// queued changes is not a decision downstream gets to make.
    pub fn queued_mutation_count(&self) -> usize {
        self.pending.len()
    }

    /// Apply every queued mutation, and return how many ran.
    ///
    /// Call at the top of a frame, before anything iterates. Returns the count so
    /// a test can assert that a request was *applied* rather than merely made —
    /// the alternative is a queue that quietly stops draining, with no test
    /// noticing.
    ///
    /// Failures do not stop the drain and are not returned here; they land in
    /// [`World::take_failures`], because a caller that wants to show an editor
    /// toast wants all of them, not the first.
    pub fn flush_mutations(&mut self) -> usize {
        let pending = std::mem::take(&mut self.pending);
        let applied = pending.len();
        for mutation in pending {
            if let Err(error) = mutation(self) {
                self.failures.push(error);
            }
        }
        applied
    }

    /// The failures from the flushes so far, and clear them.
    ///
    /// A buffer rather than a log line because the caller of a queued mutation is
    /// usually a UI that has to tell the user *which* edit did not happen.
    pub fn take_failures(&mut self) -> Vec<Error> {
        std::mem::take(&mut self.failures)
    }

    /// How many failures are waiting to be read.
    pub fn failure_count(&self) -> usize {
        self.failures.len()
    }
}
