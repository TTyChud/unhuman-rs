//! `bifrost::Frame` — one frame, from an empty graph to a validated submission.
//!
//! ## What a frame is
//!
//! A frame is a render graph, declared from scratch and thrown away. That is not a
//! shortcut: it is what `chassis::graph::RenderGraph` documents (`builder.rs:9-11`
//! — no compile cache, at realistic pass counts the compile is microseconds), and
//! it means a resource dropped from the frontend cannot linger in a pool waiting
//! for an owner that no longer exists.
//!
//! ## Nothing here is a port
//!
//! Upstream cancelled the frame-graph frontend (decision D6; `docs/PARITY.md` §5).
//! What survives in the C++ is the backend half — `VulkanRenderGraph`, a barrier
//! synthesiser — with no declaration layer above it at all: no pass builder, no
//! read/write set, no closure registry. So `Frame` and [`Pass`] are new work, not
//! a translation, and every C++ reference below is cited for something being
//! *absent*.
//!
//! ## The seam that is missing today
//!
//! [`chassis::Gpu`] has no way to install a [`chassis::graph::PassRecorder`]. The
//! Vulkan backend has an inherent `VulkanGpu::set_pass_recorder`, and the
//! wgpu backend has nothing equivalent, so a backend-neutral frontend cannot
//! currently hand its passes to a backend through the trait. [`Frame::recorder`]
//! returns the whole dispatch table in one object, which is the shape that needs
//! receiving; `Gpu::submit` is where recording happens on both backends today.
//! Until that method exists on the trait, `Frame` can *validate* a frame
//! backend-neutrally and *record* it only on Vulkan, in one inherent call.
//!
//! This is called out in [`Frame::submit`] rather than papered over, because a
//! missing trait method discovered by the second backend is a wasted afternoon.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

// `ValidationError` reached through `graph::types` rather than re-exported from
// `graph`, which is where it belongs: `graph/mod.rs` re-exports `ErrorKind` and
// every other name in `types` and misses this one, so a caller who wants to
// report a frame error has to know a module path the facade was supposed to
// hide. Not bifrost's file to fix; worth naming in the report.
use chassis::graph::types::ValidationError;
use chassis::graph::{self, RenderGraph};

use crate::pass::Pass;
use crate::replay::DrawCount;
use crate::stats::{FrameStats, StateTracker};

/// Why a frame did not compile.
#[derive(Debug)]
pub enum FrameError {
    /// The graph rejected the declaration. **Every** error, not just the first:
    /// `chassis::graph::validate` collects them all so one run reports every
    /// mistake, and a `FrameError` that kept only the first would throw away the
    /// other seven.
    Invalid(Vec<ValidationError>),
    /// The backend refused the frame.
    Backend(chassis::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Invalid(errors) => {
                write!(f, "render graph rejected the frame:")?;
                for error in errors {
                    write!(f, "\n  {error}")?;
                }
                Ok(())
            }
            FrameError::Backend(error) => write!(f, "{error}"),
        }
    }
}

impl Error for FrameError {}

impl From<chassis::Error> for FrameError {
    fn from(error: chassis::Error) -> Self {
        FrameError::Backend(error)
    }
}

/// What a compiled frame produced.
#[derive(Clone, Debug, Default)]
pub struct FrameOutput {
    /// Passes in execution order, with barriers resolved.
    ///
    /// Pure data: no closures, no backend types. An executor works from this and
    /// never sees the frontend again, which is what lets it record passes in
    /// parallel while the next frame is being declared.
    pub compiled: graph::CompiledGraph,
    pub stats: FrameStats,
}

/// One frame.
pub struct Frame {
    graph: RenderGraph,
    extent: chassis::Extent2d,
    number: u64,
    stats: FrameStats,
    /// One recorder per declared pass, in declaration order. The executor looks a
    /// pass's body up by declaration index (`PassScope::index` is
    /// `CompiledPass::index`), so this list and the graph's pass list must stay in
    /// the same order. They cannot drift: both are appended to together in
    /// [`Frame::add_pass`].
    recorders: Vec<Arc<dyn graph::PassRecorder>>,
}

impl Frame {
    /// A new, empty frame.
    ///
    /// Reusing a frame across frames is not offered on purpose. `RenderGraph::reset`
    /// exists and is free, but a `Frame` that is both "the current frame" and "the
    /// frame being submitted" is two states in one value, and the mistake is
    /// declaring into a frame that has already been submitted.
    pub fn begin(extent: chassis::Extent2d) -> Self {
        Self {
            graph: RenderGraph::new(),
            extent,
            number: 0,
            stats: FrameStats::default(),
            recorders: Vec::new(),
        }
    }

    /// Set the frame number. Purely for [`FrameStats`]: a frame count that resets
    /// makes a regression that only shows up every 40th frame impossible to find.
    pub fn set_number(&mut self, number: u64) {
        self.number = number;
        self.stats.frame = number;
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn extent(&self) -> chassis::Extent2d {
        self.extent
    }

    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }

    pub fn stats_mut(&mut self) -> &mut FrameStats {
        &mut self.stats
    }

    /// The declaration, for a dump or a test.
    pub fn graph(&self) -> &RenderGraph {
        &self.graph
    }

    /// Import an external texture: a swapchain image, or anything the graph does
    /// not own.
    ///
    /// **Imports are the culling roots.** A pass that cannot reach one contributes
    /// nothing to the frame and is compiled away (`compiler.rs:250-256`) — correct,
    /// because nothing observable is produced, and the reason a frame with no
    /// imports is an empty frame rather than a broken one. Using
    /// [`Frame::texture`] where an import was meant is the mistake that produces a
    /// frame that validates, compiles to nothing, and draws nothing.
    pub fn import_texture(
        &mut self,
        name: impl Into<String>,
        initial: graph::ImageState,
        final_state: graph::ImageState,
    ) -> graph::TextureId {
        self.graph.import_texture(name, initial, final_state)
    }

    /// A transient texture: created for this frame and gone at its end.
    pub fn texture(
        &mut self,
        name: impl Into<String>,
        initial: graph::ImageState,
        final_state: graph::ImageState,
    ) -> graph::TextureId {
        self.graph.create_texture(name, initial, final_state)
    }

    pub fn import_buffer(&mut self, name: impl Into<String>) -> graph::BufferId {
        self.graph.import_buffer(name)
    }

    pub fn buffer(&mut self, name: impl Into<String>) -> graph::BufferId {
        self.graph.create_buffer(name)
    }

    /// Declare a pass.
    ///
    /// The pass's declared set is replayed onto the graph in the order the builder
    /// collected it, and the graph assigns versions as it goes — which is why
    /// declaration order is execution order.
    ///
    /// Returns the index the body will be found at, which is also
    /// [`chassis::graph::PassScope::index`] at record time.
    pub fn add_pass(&mut self, pass: Pass) -> usize {
        let index = self.graph.passes().len();
        {
            let mut builder = self
                .graph
                .pass(pass.name.clone())
                .kind(pass.kind)
                .queue(pass.queue);
            for color in pass.colors() {
                builder = if color.load {
                    builder.color_load(color.texture)
                } else {
                    builder.color(color.texture, color.clear)
                };
                if let Some(depth) = pass.depth() {
                    if depth.resolve != graph::NO_RESOURCE && color.texture == depth.resolve {
                        // A resolve target is read as an attachment by the driver,
                        // which the graph cannot see; declaring it here is what
                        // keeps it from being culled as unused.
                        builder = builder.read_texture(depth.resolve);
                    }
                }
            }
            if let Some(depth) = pass.depth() {
                builder = builder.depth(depth.texture, depth.clear_depth);
            }
            for read in pass.reads() {
                builder = match read.slot {
                    crate::pass::Slot::Texture(id) => builder.read_texture(id),
                    crate::pass::Slot::Buffer(id) => builder.read_buffer(id),
                };
            }
            for write in pass.writes() {
                builder = match write.slot {
                    crate::pass::Slot::Texture(id) => builder.write_texture(id, write.state),
                    crate::pass::Slot::Buffer(id) => builder.write_buffer(id),
                };
            }
        }
        self.recorders.push(pass.recorder());
        self.stats.passes_declared += 1;
        index
    }

    /// How many passes are declared, including any the graph will cull.
    pub fn pass_count(&self) -> usize {
        self.graph.passes().len()
    }

    /// Every pass's body, as one recorder.
    ///
    /// Dispatches on [`chassis::graph::PassScope::index`]. A pass with no body
    /// records nothing, which is what [`crate::pass::PassBuilder::build`] gives,
    /// and a pass index the frame never declared is an error rather than a silent
    /// skip: a silent skip is how a frame renders without its post-processing and
    /// nobody knows why.
    pub fn recorder(&self) -> Arc<dyn graph::PassRecorder> {
        Arc::new(FrameRecorder {
            bodies: self.recorders.clone(),
        })
    }

    /// Validate without compiling. What a debug layer calls every frame, so an
    /// error names its pass instead of surfacing as a barrier bug hundreds of lines
    /// later.
    pub fn validate(&self) -> Vec<ValidationError> {
        self.graph.validate()
    }

    /// Compile the frame.
    ///
    /// Compiles; does not execute. Execution is the executor's, and the trait's
    /// [`chassis::RenderGraph::execute`] runs the *backend's* graph, which is not
    /// this one — see the module docs on the missing install point.
    pub fn end(&mut self) -> Result<FrameOutput, FrameError> {
        let compiled = self.graph.compile().map_err(FrameError::Invalid)?;
        self.stats.passes_compiled = compiled.passes.len() as u32;
        Ok(FrameOutput {
            compiled,
            stats: self.stats,
        })
    }

    /// Add one replay's counts to the frame's.
    pub fn record_draws(&mut self, count: DrawCount) {
        self.stats.draws += count.draws;
        self.stats.instances += count.instances;
    }

    /// Add the meshes a cull removed.
    pub fn record_culled(&mut self, culled: u32) {
        self.stats.draws_culled += culled;
    }

    /// Fold a tracker's state changes into the frame's and clear the tracker.
    pub fn drain_state(&mut self, tracker: &mut StateTracker) {
        tracker.drain_into(&mut self.stats);
    }

    /// Submit through a backend.
    ///
    /// Thin on purpose. Recording happens inside `Gpu::submit`, on the backend's own
    /// graph; there is no trait method that accepts *this* frame, so anything more
    /// substantial here would either be a no-op with a confident name or would
    /// reach past the trait — the exact mistake `session.md` records for `eyes`
    /// putting a `wgpu::Surface` behind a `dyn Window`.
    pub fn submit(&self, gpu: &mut dyn chassis::Gpu) -> Result<(), FrameError> {
        gpu.submit().map_err(FrameError::Backend)
    }

    /// The human-readable declaration dump.
    pub fn dump(&self) -> String {
        self.graph.dump()
    }
}

/// Dispatches a pass body by declaration index.
struct FrameRecorder {
    bodies: Vec<Arc<dyn graph::PassRecorder>>,
}

impl graph::PassRecorder for FrameRecorder {
    fn record(
        &self,
        scope: &graph::PassScope<'_>,
        list: &mut dyn chassis::CommandList,
    ) -> chassis::Result<()> {
        let body = self.bodies.get(scope.index as usize).ok_or_else(|| {
            chassis::Error::InvalidDescriptor(format!(
                "pass index {} ('{}') has no declared body",
                scope.index, scope.name
            ))
        })?;
        body.record(scope, list)
    }
}
