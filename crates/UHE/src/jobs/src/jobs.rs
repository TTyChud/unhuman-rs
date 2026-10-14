use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

pub type Job = Box<dyn FnOnce() + Send + 'static>;

type QueuedJob = (Job, Option<Arc<JobCounter>>);

thread_local! {
    /// Unique index of the calling thread. Main thread is 0, workers are 1..=N.
    static THREAD_INDEX: Cell<u32> = const { Cell::new(0) };
}

/// A lightweight counter to wait on a group of jobs.
///
/// This is the core of the task graph: jobs can depend on counters.
#[derive(Default)]
pub struct JobCounter(AtomicUsize);

impl JobCounter {
    pub fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    pub fn pending(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    pub fn is_complete(&self) -> bool {
        self.pending() == 0
    }

    fn inc(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    /// `AcqRel`, not `Release`.
    ///
    /// A waiter observes zero via a single `Acquire` load, which only
    /// synchronizes with whichever thread performed the *final* decrement.
    /// With a release-only RMW, every other worker's writes stay unordered and
    /// become visible or not at random. `AcqRel` on every decrement chains the
    /// RMWs into one release sequence, so the last one carries every prior
    /// job's writes. The C++ engine has this exact flaw.
    fn dec(&self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Shared {
    queue: Mutex<VecDeque<QueuedJob>>,
    wake: Condvar,
    running: AtomicBool,
    in_flight: AtomicI32,
}

/// Runs one dequeued job and settles both counters. Shared by `try_pop_job` and
/// the worker loop so there is exactly one place that decrements them.
///
/// Note the ordering: the counter settles after the body returns but before the
/// job closure itself is destroyed, so captured `Arc`s outlive the signal by a
/// few instructions. That matches the C++ engine, where `std::function` is also
/// destroyed after `fetch_sub`. A counter therefore means "the body finished",
/// not "the body's captures are gone".
fn run_queued(job: QueuedJob, shared: &Shared) {
    let (job, counter) = job;
    job();
    if let Some(counter) = counter {
        counter.dec();
    }
    // AcqRel for the same reason as `JobCounter::dec`.
    shared.in_flight.fetch_sub(1, Ordering::AcqRel);
}

fn pop(shared: &Shared) -> Option<QueuedJob> {
    shared.queue.lock().unwrap().pop_front()
}

/// A thread pool. Index 0 is the calling (main) thread; spawned workers are 1..=N.
///
/// `thread_count()` exists because per-thread Vulkan command pools need one
/// command list per thread, and the mapping has to be known at submit time.
pub struct Jobs {
    shared: Arc<Shared>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl Jobs {
    /// Spawns `hardware_concurrency() - 1` workers, leaving one core for the
    /// calling thread. The `- 1` matters on big.LITTLE Android targets, which
    /// the C++ engine targeted explicitly.
    pub fn new() -> Self {
        let cores = thread::available_parallelism().map_or(4, |n| n.get());
        let num_workers = cores.saturating_sub(1).max(1);

        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            running: AtomicBool::new(true),
            in_flight: AtomicI32::new(0),
        });

        let mut workers = Vec::with_capacity(num_workers);
        for index in 0..num_workers {
            let shared = Arc::clone(&shared);
            workers.push(
                thread::Builder::new()
                    .name(format!("grind-worker-{index}"))
                    .spawn(move || worker_loop(index as u32 + 1, shared))
                    .expect("failed to spawn job worker"),
            );
        }

        Self { shared, workers }
    }

    /// Kicks a single job. If a counter is attached it is incremented here, and
    /// decremented when the job finishes.
    pub fn execute(&self, job: Job, counter: Option<Arc<JobCounter>>) {
        // Count the job as in-flight BEFORE it becomes visible to other
        // threads, so is_busy()/wait_for_all() can never miss it.
        self.shared.in_flight.fetch_add(1, Ordering::AcqRel);

        if let Some(counter) = &counter {
            counter.inc();
        }

        self.shared.queue.lock().unwrap().push_back((job, counter));
        self.shared.wake.notify_one();
    }

    /// Splits `count` into contiguous batches, running `f(i)` for each, and
    /// blocks until all iterations complete. Contiguous slices beat one job per
    /// element when the body is cheap.
    pub fn parallel_for<F>(&self, count: u32, f: F, min_batch_size: u32)
    where
        F: Fn(u32) + Send + Sync + 'static,
    {
        if count == 0 {
            return;
        }
        let min_batch_size = min_batch_size.max(1);
        let worker_count = self.workers.len() as u32;

        // Single iteration or single worker: run inline, no scheduling overhead.
        if count == 1 || worker_count == 1 {
            for i in 0..count {
                f(i);
            }
            return;
        }

        let counter = Arc::new(JobCounter::new());
        let f = Arc::new(f);

        // Aim for ~2 batches per worker so stragglers can be picked up by
        // threads that finish early.
        let mut batch_count = count.div_ceil(min_batch_size);
        batch_count = batch_count.min(worker_count * 2).max(1);
        let batch_size = count.div_ceil(batch_count);

        for batch in 0..batch_count {
            let begin = batch * batch_size;
            if begin >= count {
                break;
            }
            let end = (begin + batch_size).min(count);

            let f = Arc::clone(&f);
            self.execute(
                Box::new(move || {
                    for i in begin..end {
                        f(i);
                    }
                }),
                Some(Arc::clone(&counter)),
            );
        }

        self.wait(&counter);
    }

    /// Blocks until `counter` reaches zero. While waiting the calling thread
    /// steals queued jobs and runs them, so waiting is never idle CPU time —
    /// and a counter whose jobs are all still queued still completes on a
    /// single-worker machine instead of deadlocking.
    pub fn wait(&self, counter: &JobCounter) {
        // The counter is re-checked every iteration and a successful pop does
        // NOT exit the loop. Folding the two into `while !done && !pop()` would
        // return as soon as the caller work-helped a single job, stranding the
        // rest of the batch.
        while !counter.is_complete() {
            if !self.try_pop_job() {
                // Queue is empty: the job is either running elsewhere or has
                // not been scheduled yet.
                thread::yield_now();
            }
        }
    }

    /// Blocks until every job kicked so far has completed. Unlike `Drop`, the
    /// job system keeps running afterwards.
    pub fn wait_for_all(&self) {
        while self.is_busy() {
            if !self.try_pop_job() {
                thread::yield_now();
            }
        }
    }

    /// True while any job kicked through this system is still queued or running.
    pub fn is_busy(&self) -> bool {
        self.shared.in_flight.load(Ordering::Acquire) > 0
    }

    /// Total threads: workers + the main thread. This is the per-frame command
    /// list count.
    pub fn thread_count(&self) -> u32 {
        self.workers.len() as u32 + 1
    }

    /// Index of the calling thread in `0..thread_count()`.
    pub fn current_thread_index() -> u32 {
        THREAD_INDEX.with(Cell::get)
    }

    /// Pops ONE queued job and runs it on the calling thread. Returns false if
    /// the queue was empty. Used by workers and by any thread work-helping
    /// inside `wait`/`wait_for_all`/`TaskGraph`.
    pub fn try_pop_job(&self) -> bool {
        match pop(&self.shared) {
            Some(job) => {
                run_queued(job, &self.shared);
                true
            }
            None => false,
        }
    }
}

impl Default for Jobs {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.wake.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker_loop(index: u32, shared: Arc<Shared>) {
    THREAD_INDEX.with(|cell| cell.set(index));

    while shared.running.load(Ordering::Acquire) {
        match pop(&shared) {
            Some(job) => run_queued(job, &shared),
            None => {
                // Queue is empty: sleep until new work arrives or shutdown.
                let queue = shared.queue.lock().unwrap();
                let _unused = shared
                    .wake
                    .wait_while(queue, |queue| {
                        queue.is_empty() && shared.running.load(Ordering::Acquire)
                    })
                    .unwrap();
            }
        }
    }
}
