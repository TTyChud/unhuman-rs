//! What the frame actually cost, in numbers rather than in a feeling.
//!
//! ## Why this file exists at all
//!
//! "The frame is slow" is not a bug report. It is a symptom with at least four
//! unrelated causes — too many draw calls, too many state changes, a GPU wait, a
//! CPU-side rebuild — and none of them announce which. Without counters the only
//! available technique is commenting things out, which finds the answer in
//! hours and leaves the engine in pieces in the meantime.
//!
//! ## What is counted, and what is deliberately not
//!
//! Counted: passes (declared, compiled, culled away), draw calls, instances,
//! GPU-visible state changes, meshes rejected by culling.
//!
//! Not counted: time. `RenderGraph::execute` takes a `&grind::Jobs` and a
//! backend *may* record passes in parallel, so wall-clock attribution per pass
//! belongs to Phase 10's timestamp queries, on both backends, where the numbers
//! are comparable. A `Duration` measured here would be the CPU time to *issue*
//! work, which correlates with nothing a GPU-bound frame is doing — and a
//! counter that lies about where the time goes is worse than no counter.
//!
//! ## Where the C++ stood
//!
//! `Renderer2D` has a `Statistics { DrawCalls, QuadCount }` and it is never
//! read: `ResetStats` and `GetStats` have no callers outside the file, and the
//! live 2D renderer never even constructs one. `Renderer3D` has no statistics at
//! all. A counter that exists and is not looked at is not instrumentation.

use chassis::{BindGroup, Buffer, Pipeline};

/// GPU-visible state changes caused by one frame's replay.
///
/// Each of these is a place a driver can do real work: a pipeline bind rebuilds
/// internal state, a descriptor bind may revalidate, a push-constant write is a
/// transfer. The number that matters is the count, and the count that matters
/// most is [`Self::pipelines`] — a pipeline change is the expensive one, and it
/// is also the one a batched draw list exists to remove.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct StateChanges {
    pub pipelines: u32,
    pub bind_groups: u32,
    pub push_constants: u32,
    pub vertex_buffers: u32,
    pub index_buffers: u32,
}

impl StateChanges {
    /// Every change, of every kind.
    pub fn total(&self) -> u32 {
        self.pipelines
            + self.bind_groups
            + self.push_constants
            + self.vertex_buffers
            + self.index_buffers
    }

    pub fn merge(&mut self, other: &Self) {
        self.pipelines += other.pipelines;
        self.bind_groups += other.bind_groups;
        self.push_constants += other.push_constants;
        self.vertex_buffers += other.vertex_buffers;
        self.index_buffers += other.index_buffers;
    }

    /// Changes minus the unavoidable ones: one draw's worth of bindings each.
    ///
    /// A frame with 400 draws and 400 state changes has batched perfectly. A
    /// frame with 400 draws and 900 has 500 avoidable ones, which is the number
    /// a batching bug shows up in.
    pub fn excess(&self, draws: u32) -> u32 {
        self.total().saturating_sub(draws.saturating_mul(2))
    }
}

/// Descriptor sets a tracked command list can hold.
///
/// Vulkan guarantees 4 sets (`maxDrawIndexedIndexValue` is unrelated, but
/// `VkPhysicalDeviceLimits::maxBoundDescriptorSets` is 4 on every implementation
/// to date); the array is larger so an out-of-range set number is counted rather
/// than dropped, which is the difference between "the counters look wrong" and
/// "the counters say a pass bound set 7".
const TRACKED_SETS: usize = 8;

/// Replay-time state cache, so a bind is only issued when it changes.
///
/// This is the frontend's half of the batching story: the *draw list* decides
/// what order to submit in, and this decides how many of those submissions the
/// driver actually has to act on. Both halves are needed and they are not the
/// same thing — a correctly sorted list that re-binds the same pipeline twice
/// still costs the driver twice, which is exactly what the C++ does.
///
/// ## Why a push-constant write is always counted
///
/// Because there is no cheap way to know it is redundant, and guessing wrong is
/// a rendering bug rather than a slow frame. Skipping a write means comparing
/// the bytes about to be written against the last bytes written — the same size
/// as the write itself, on the CPU, on the critical path — to save a transfer
/// the GPU will do in a few hundred nanoseconds. It is a pessimisation dressed as
/// an optimisation, so the counter reports the truth: one write, one change.
pub struct StateTracker {
    changes: StateChanges,
    pipeline: Option<Pipeline>,
    groups: [Option<BindGroup>; TRACKED_SETS],
    vertex_buffer: Option<(Buffer, u64)>,
    index_buffer: Option<(Buffer, u64)>,
}

impl Default for StateTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StateTracker {
    pub fn new() -> Self {
        Self {
            changes: StateChanges::default(),
            pipeline: None,
            groups: [None; TRACKED_SETS],
            vertex_buffer: None,
            index_buffer: None,
        }
    }

    /// Start a pass: everything is unbound again.
    ///
    /// Not a cosmetic reset. Each pass records into its **own** command list, so
    /// nothing a previous pass bound survives into this one — carrying the cache
    /// across would report zero changes for the first bind of every pass and
    /// under-count by exactly the number of passes. The C++ cannot hit this bug
    /// because it records everything into one list, where the cache *would* be
    /// correct; that is the price of parallel recording, and it is paid in
    /// counters rather than in correctness.
    pub fn begin_pass(&mut self) {
        self.pipeline = None;
        self.groups = [None; TRACKED_SETS];
        self.vertex_buffer = None;
        self.index_buffer = None;
    }

    /// True when the pipeline actually changed, counting the change if so.
    pub fn bind_pipeline(&mut self, pipeline: &Pipeline) -> bool {
        if self.pipeline == Some(*pipeline) {
            return false;
        }
        self.pipeline = Some(*pipeline);
        self.changes.pipelines += 1;
        true
    }

    /// True when the group at `set` actually changed.
    pub fn bind_group(&mut self, set: u32, group: &BindGroup) -> bool {
        let Some(slot) = self.groups.get_mut(set as usize) else {
            // Counted rather than ignored: a set number past the end is either a
            // bug in the layout or a driver that will reject it, and a counter
            // that silently drops it cannot tell those apart from "no bind".
            self.changes.bind_groups += 1;
            return true;
        };
        if *slot == Some(*group) {
            return false;
        }
        *slot = Some(*group);
        self.changes.bind_groups += 1;
        true
    }

    /// Always a change. See the type's docs.
    pub fn push_constants(&mut self) {
        self.changes.push_constants += 1;
    }

    /// True when the vertex buffer handle *or its offset* changed.
    ///
    /// The offset is part of the binding, not an afterthought. Two meshes in one
    /// interleaved buffer differ only by offset, so a tracker that compares
    /// handles alone would skip the second bind and the second mesh would render
    /// the first one's vertices — a wrong picture, not a slow one, and it appears
    /// only once sub-allocation lands (Phase 3), which is exactly the kind of bug
    /// that survives because nothing exercised the path when it was written.
    pub fn bind_vertex_buffer(&mut self, buffer: &Buffer, offset: u64) -> bool {
        let binding = (*buffer, offset);
        if self.vertex_buffer == Some(binding) {
            return false;
        }
        self.vertex_buffer = Some(binding);
        self.changes.vertex_buffers += 1;
        true
    }

    /// True when the index buffer handle or its offset changed.
    ///
    /// Offset in bytes, not indices, because that is what
    /// [`chassis::CommandList::bind_index_buffer`] takes and what a merged index
    /// buffer actually stores.
    pub fn bind_index_buffer(&mut self, buffer: &Buffer, offset: u64) -> bool {
        let binding = (*buffer, offset);
        if self.index_buffer == Some(binding) {
            return false;
        }
        self.index_buffer = Some(binding);
        self.changes.index_buffers += 1;
        true
    }

    pub fn changes(&self) -> StateChanges {
        self.changes
    }

    /// Fold this tracker's totals into a frame's, and clear it.
    pub fn drain_into(&mut self, frame: &mut FrameStats) {
        frame.state.merge(&self.changes);
        self.changes = StateChanges::default();
        self.begin_pass();
    }
}

/// The cost of one frame, in the units a draw list produces.
///
/// Deliberately a plain struct of numbers with no behaviour beyond arithmetic:
/// anything that decides what the counters mean belongs to the thing being
/// counted, and a report format that lives next to the data goes stale silently.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct FrameStats {
    /// Monotonic frame number. Not the swapchain's: an acquired image index wraps
    /// at two or three, which makes it useless for spotting a once-every-N-frames
    /// regression.
    pub frame: u64,
    /// Passes opened, including any that compiled away.
    pub passes_declared: u32,
    /// Passes that survived the graph's dead-pass cull.
    pub passes_compiled: u32,
    /// Draw calls issued.
    pub draws: u32,
    /// Instances across those draw calls.
    pub instances: u32,
    /// Meshes the culler rejected.
    pub draws_culled: u32,
    pub state: StateChanges,
}

impl FrameStats {
    /// Passes declared but not compiled. The graph culls a pass that cannot
    /// reach an imported resource; this is how much of the frame nobody needed.
    pub fn passes_culled(&self) -> u32 {
        self.passes_declared.saturating_sub(self.passes_compiled)
    }

    /// Instances per draw. Below 1.0 means the batcher is not batching; well
    /// above 1.0 means it is, which is only a win if the draw-call count fell
    /// with it, so this is read next to [`Self::draws`] and never alone.
    pub fn instances_per_draw(&self) -> f32 {
        if self.draws == 0 {
            0.0
        } else {
            self.instances as f32 / self.draws as f32
        }
    }

    /// Nothing was submitted.
    ///
    /// Worth testing, because a frame that is silently empty — a camera culling
    /// everything, a draw list never built, a pass set that names nothing the
    /// compiler can root — looks exactly like an engine that is working.
    pub fn is_empty(&self) -> bool {
        self.draws == 0 && self.passes_compiled == 0
    }
}

/// The result of one culling sweep.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct CullingStats {
    pub tested: u32,
    pub culled: u32,
}

impl CullingStats {
    pub fn merge(&mut self, other: &Self) {
        self.tested += other.tested;
        self.culled += other.culled;
    }

    /// Fraction rejected, or 0 when nothing was tested.
    ///
    /// A `None` would be more honest than a `0.0` that reads like "nothing was
    /// in the frustum", but it would make every call site unwrap, and the
    /// caller's own [`FrameStats::draws`] is right there to disambiguate.
    pub fn fraction_culled(&self) -> f32 {
        if self.tested == 0 {
            0.0
        } else {
            self.culled as f32 / self.tested as f32
        }
    }
}
