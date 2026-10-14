use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use crate::jobs::Jobs;

pub type TaskId = u32;

/// A task body. `Fn`, not `FnOnce`, because a graph is executed every frame:
/// the node must be callable again after `deps_remaining` is re-armed.
type Entry = Arc<dyn Fn() + Send + Sync>;

struct TaskNode {
    entry: Entry,
    /// Prerequisites remaining before this task may run.
    deps_remaining: AtomicUsize,
    /// Total incoming edges. Immutable after the build; re-arms `deps_remaining`
    /// on every `execute`.
    dep_count: usize,
    /// Tasks that depend on this one.
    dependents: Vec<TaskId>,
}

/// Shared access to the frozen graph for the duration of `execute`.
///
/// # Safety
/// `execute` blocks until every task has completed, so the nodes outlive all
/// tasks, and `TaskNode` is `Sync` (an `Arc` to a `Send + Sync` closure, an
/// atomic, and plain immutable data). Nothing is mutated during execution
/// except `deps_remaining`. The C++ version gets this for free by capturing
/// `this`; the invariant is identical.
#[derive(Clone, Copy)]
struct GraphPtr(*const TaskNode);

unsafe impl Send for GraphPtr {}

impl GraphPtr {
    /// # Safety
    /// `id` must be in bounds, and the graph must not be mutated or dropped
    /// while the returned reference is alive.
    unsafe fn get(&self, id: TaskId) -> &'static TaskNode {
        &*self.0.add(id as usize)
    }
}

/// A DAG of tasks executed across the job system.
///
/// Dispatch ownership is exclusive: a task is scheduled either by the root scan
/// (it has no dependencies) or by whichever predecessor finishes last, never
/// both.
pub struct TaskGraph {
    nodes: Vec<TaskNode>,
    /// Signed, so a dispatch-ownership bug shows up as a negative value in a
    /// test or debugger instead of wrapping to a huge positive and hanging.
    total_remaining: Arc<AtomicI32>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            total_remaining: Arc::new(AtomicI32::new(0)),
        }
    }

    /// Allocates a node in a flat, cache-friendly array.
    ///
    /// `add_dependency` is only valid between building and `execute`.
    pub fn create_task(&mut self, job: impl Fn() + Send + Sync + 'static) -> TaskId {
        let id = self.nodes.len() as TaskId;
        self.nodes.push(TaskNode {
            entry: Arc::new(job),
            deps_remaining: AtomicUsize::new(0),
            dep_count: 0,
            dependents: Vec::new(),
        });
        id
    }

    /// Task `a` must finish before task `b` can start.
    ///
    /// # Panics
    /// On a self-dependency or an out-of-range id, both graph construction
    /// bugs.
    pub fn add_dependency(&mut self, a: TaskId, b: TaskId) {
        assert!(a != b, "task {a} cannot depend on itself");
        let len = self.nodes.len() as TaskId;
        assert!(
            a < len && b < len,
            "dependency {a}->{b} out of range ({len} tasks)"
        );

        self.nodes[a as usize].dependents.push(b);
        let node = &mut self.nodes[b as usize];
        node.dep_count += 1;
        node.deps_remaining.fetch_add(1, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Executes the graph. The calling thread helps by running already-dispatched
    /// tasks instead of spinning idle.
    pub fn execute(&mut self, jobs: Arc<Jobs>) {
        let total = self.nodes.len() as i32;
        self.total_remaining.store(total, Ordering::Relaxed);

        // Re-arm each task's counter from its immutable edge count.
        for node in &self.nodes {
            node.deps_remaining.store(node.dep_count, Ordering::Relaxed);
        }

        let ptr = GraphPtr(self.nodes.as_ptr());

        // The root scan MUST read `dep_count`, never `deps_remaining`. Workers
        // resolve dependents while this scan is still running, so a non-root's
        // runtime counter can hit 0 mid-scan; checking that would dispatch the
        // task twice and run it twice. Nodes with `dep_count == 0` can never
        // appear in a dependents list, so root dispatch is exclusive.
        for id in 0..self.nodes.len() as TaskId {
            if unsafe { ptr.get(id) }.dep_count == 0 {
                dispatch(
                    ptr,
                    id,
                    Arc::clone(&jobs),
                    Arc::clone(&self.total_remaining),
                );
            }
        }

        while self.total_remaining.load(Ordering::Acquire) > 0 {
            if !jobs.try_pop_job() {
                // Queue is empty: the rest are running on workers, or waiting on
                // dependencies that have not resolved yet.
                thread::yield_now();
            }
        }
    }

    pub fn reset(&mut self) {
        self.nodes.clear();
    }
}

impl Default for TaskGraph {
    fn default() -> Self {
        Self::new()
    }
}

fn dispatch(ptr: GraphPtr, id: TaskId, jobs: Arc<Jobs>, total_remaining: Arc<AtomicI32>) {
    let entry = Arc::clone(&unsafe { ptr.get(id) }.entry);

    // Clone before the call: the closure owns its handles, `execute` borrows.
    let next_jobs = Arc::clone(&jobs);
    let next_total = Arc::clone(&total_remaining);

    jobs.execute(
        Box::new(move || {
            entry();
            resolve(ptr, id, next_jobs, next_total);
        }),
        None,
    );
}

fn resolve(ptr: GraphPtr, id: TaskId, jobs: Arc<Jobs>, total_remaining: Arc<AtomicI32>) {
    // `node` borrows node `id`; dispatch below only reads other nodes, which are
    // disjoint indices of the same array.
    let node = unsafe { ptr.get(id) };
    for &dependent in &node.dependents {
        // `fetch_sub` returns the value before the subtraction. Seeing 1 means
        // this was the last arrival, so we own the dispatch.
        let previous = unsafe { ptr.get(dependent) }
            .deps_remaining
            .fetch_sub(1, Ordering::AcqRel);
        if previous == 1 {
            dispatch(
                ptr,
                dependent,
                Arc::clone(&jobs),
                Arc::clone(&total_remaining),
            );
        }
    }

    // MUST be the last access to graph state. A waiter that observes 0 has the
    // right to drop the graph, so any access after this line would race with
    // that destruction. Found by ThreadSanitizer in the C++ engine.
    // AcqRel: the waiter in `execute` observes zero with a single Acquire
    // load, so every decrement must chain the release sequence.
    total_remaining.fetch_sub(1, Ordering::AcqRel);
}
