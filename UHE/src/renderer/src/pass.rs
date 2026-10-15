//! A pass: a closure, and the read/write set that declares what it touches.
//!
//! # The set is the contract
//!
//! Everything in this file exists so that one sentence is true:
//!
//! > **A pass may only touch what it declared.**
//!
//! That is what makes a hazard unrepresentable rather than merely unlikely. The
//! render graph derives every ordering edge, every image transition and every
//! barrier from the declared sets alone — there is no barrier API on
//! [`chassis::CommandList`] and no way for a pass to write one — so a pass that
//! samples a texture it did not declare has told the graph nothing about a
//! dependency that exists, and the graph will run two passes that race on it.
//!
//! The failure is not hypothetical. It is the C++'s state: a graph that
//! synthesises barriers, plus `BeginRenderPass` hand-writing
//! `oldLayout = eUndefined` on entry (`VulkanCommandBuffer.cpp:104,153`), which
//! discards the graph's own answer about what was there before. Two sources of
//! truth about the same transition, and the one that loses is whichever the
//! compiler cannot see.
//!
//! So: **a helper that declares less than the body uses is the bug to look for**,
//! and it is a silent one. The frame is correct on the machine the author tested,
//! on the driver that is lenient, and wrong under validation or on a tiler. Add
//! the declaration. Never work around it by adding a barrier.
//!
//! ## Declaring *more* is also wrong, in the other direction
//!
//! A barrier that claims an access the pass does not take is not conservative,
//! it is a stall. Declaring a whole mip chain because the shader samples one
//! level makes the driver wait on writes it did not need, and — worse — it makes
//! the pass a writer of the whole resource, so a later pass that reads a
//! different level of it gets ordered behind this one for no reason. The set is a
//! description, not a wish.
//!
//! ## Ordering inside a frame is declaration order
//!
//! A write is bound to the version current when it is declared, so declaring
//! shadow → depth → colour is already the order they will run in. Nothing has to
//! ask for it, and nothing can disagree with it.

use std::fmt;
use std::sync::Arc;

use chassis::graph::{ImageState, PassType, Queue};

/// A graph slot, tagged. The set is stored as slots, not as versions: versions
/// are assigned by the graph at declaration time, so a pass built once and
/// replayed into a fresh graph each frame must not carry them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Slot {
    Texture(chassis::graph::TextureId),
    Buffer(chassis::graph::BufferId),
}

/// A colour attachment: a slot, and whether what was there has to survive.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColorTarget {
    pub texture: chassis::graph::TextureId,
    /// `true` is `LoadOp::Load`, `false` is `LoadOp::Clear`.
    ///
    /// A bool rather than the graph's `LoadOp` because the other two variants
    /// (`DontCare`, and a clear colour) belong to the *executor*, which is the only
    /// place that knows the extent. A pass that could say `DontCare` would be
    /// asking for a transition it does not control.
    pub load: bool,
    pub clear: [f32; 4],
}

/// A depth attachment.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DepthTarget {
    pub texture: chassis::graph::TextureId,
    pub clear_depth: f32,
    /// MSAA resolve target, or [`chassis::graph::NO_RESOURCE`].
    pub resolve: u32,
}

/// A declared read: the slot, and the state it is read in.
///
/// The state is what makes read-only depth expressible. [`chassis::graph`] has no
/// separate "depth read" state, so a depth-prepass's output is read in
/// `DepthAttachment` and the graph derives the transition from the producing
/// version — which is the correct and only available spelling, and is why this is
/// a struct rather than a bare `Slot`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ReadAccess {
    pub slot: Slot,
    pub state: ImageState,
}

/// A declared write, in an explicit state.
///
/// Attachments are writes too. A pass that binds a colour target and does not
/// list it as a write is the single most common way to produce a validation
/// error the graph can name precisely (`ErrorKind::UndeclaredAttachment`), and
/// that error exists because the declaration, not the attachment list, is the
/// thing the compiler trusts.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WriteAccess {
    pub slot: Slot,
    pub state: ImageState,
}

/// Records one pass into one command list.
///
/// Already a `chassis` trait. Re-declaring it here would be a second definition of
/// the same shape, and the gotchas in `session.md` are explicit about that class:
/// three copies of a format table, two `SyncTier` enums, an RHI signature no
/// backend could implement. A closure bifrost accepts is a closure the backend
/// already knows how to call.
pub type RecordFn = dyn Fn(&chassis::graph::PassScope<'_>, &mut dyn chassis::CommandList) -> chassis::Result<()>
    + Send
    + Sync;

/// A pass, declared: what it is, what it touches, and how it records.
///
/// `Send + Sync` because the executors record passes in parallel. A body that
/// captured a `&mut` anything could not be shared, which would rule out the
/// thread-per-pass path that `RenderGraph::execute`'s `&grind::Jobs` exists for.
/// Per-instance data therefore travels in buffers, not in captures.
pub struct Pass {
    pub name: String,
    pub kind: PassType,
    pub queue: Queue,
    colors: Vec<ColorTarget>,
    depth: Option<DepthTarget>,
    reads: Vec<ReadAccess>,
    writes: Vec<WriteAccess>,
    body: Arc<RecordFn>,
}

impl fmt::Debug for Pass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The body is not a closure's problem. A derived `Debug` would print a
        // function pointer and no information; this prints the declared set,
        // which is the half that is ever wrong.
        f.debug_struct("Pass")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("queue", &self.queue)
            .field("colors", &self.colors.len())
            .field("depth", &self.depth.is_some())
            .field("reads", &self.reads)
            .field("writes", &self.writes)
            .finish()
    }
}

/// Builder for a [`Pass`].
///
/// Deliberately a free-standing builder rather than methods that mutate a `Pass`
/// directly: the declaration is assembled, then handed to a frame once. A half-built
/// pass has no frame to be in.
pub struct PassBuilder {
    pass: Pass,
}

impl PassBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            pass: Pass {
                name: name.into(),
                kind: PassType::Graphics,
                queue: Queue::Graphics,
                colors: Vec::new(),
                depth: None,
                reads: Vec::new(),
                writes: Vec::new(),
                body: Arc::new(|_, _| Ok(())),
            },
        }
    }

    pub fn kind(mut self, kind: PassType) -> Self {
        self.pass.kind = kind;
        self
    }

    pub fn queue(mut self, queue: Queue) -> Self {
        self.pass.queue = queue;
        self
    }

    /// Read-write colour target, cleared to `clear`.
    ///
    /// Declares the write as well. A helper that took only the texture would leave
    /// the caller to restate it, and the two spellings would drift.
    pub fn color(mut self, texture: chassis::graph::TextureId, clear: [f32; 4]) -> Self {
        self.pass.writes.push(WriteAccess {
            slot: Slot::Texture(texture),
            state: ImageState::ColorAttachment,
        });
        self.pass.colors.push(ColorTarget {
            texture,
            load: false,
            clear,
        });
        self
    }

    /// Read-write colour target whose previous contents must survive.
    ///
    /// The post-processing path: a pass that accumulates into last frame's
    /// luminance cannot clear the target it is accumulating into. `DontCare` would
    /// be the mistake here — it is a legal load op and it discards the history
    /// while looking correct for one frame.
    pub fn color_load(mut self, texture: chassis::graph::TextureId) -> Self {
        self.pass.writes.push(WriteAccess {
            slot: Slot::Texture(texture),
            state: ImageState::ColorAttachment,
        });
        self.pass.colors.push(ColorTarget {
            texture,
            load: true,
            clear: [0.0; 4],
        });
        self
    }

    /// Read-write depth target.
    pub fn depth(mut self, texture: chassis::graph::TextureId, clear_depth: f32) -> Self {
        self.pass.writes.push(WriteAccess {
            slot: Slot::Texture(texture),
            state: ImageState::DepthAttachment,
        });
        self.pass.depth = Some(DepthTarget {
            texture,
            clear_depth,
            resolve: chassis::graph::NO_RESOURCE,
        });
        self
    }

    /// Read-only depth, for a depth pre-pass or a shadow lookup.
    ///
    /// A **read**, not a write. That is the entire difference and it is what stops
    /// the pass from being ordered as a writer of the depth buffer — declaring it
    /// through [`Self::depth`] would put it after every other pass that writes
    /// depth, which is a pass with nothing to do.
    pub fn read_only_depth(self, texture: chassis::graph::TextureId) -> Self {
        self.read(Slot::Texture(texture), ImageState::DepthAttachment)
    }

    /// Reads a texture as a sampled image.
    pub fn shader_read(self, texture: chassis::graph::TextureId) -> Self {
        self.read(Slot::Texture(texture), ImageState::ShaderRead)
    }

    /// Reads a buffer. Buffers have no image state; the graph decides the access
    /// from the pass kind and the role.
    pub fn read_buffer(self, buffer: chassis::graph::BufferId) -> Self {
        self.read(Slot::Buffer(buffer), ImageState::Undefined)
    }

    /// Writes a texture as a storage image — a compute pass's output.
    pub fn storage(mut self, texture: chassis::graph::TextureId) -> Self {
        self.pass.writes.push(WriteAccess {
            slot: Slot::Texture(texture),
            state: ImageState::Storage,
        });
        self
    }

    /// Writes a buffer.
    pub fn write_buffer(mut self, buffer: chassis::graph::BufferId) -> Self {
        self.pass.writes.push(WriteAccess {
            slot: Slot::Buffer(buffer),
            state: ImageState::Storage,
        });
        self
    }

    fn read(mut self, slot: Slot, state: ImageState) -> Self {
        self.pass.reads.push(ReadAccess { slot, state });
        self
    }

    /// The recording body.
    ///
    /// Takes it last so the declaration is complete before the closure exists,
    /// which reads in order: what the pass is, what it touches, what it does.
    pub fn record(
        mut self,
        body: impl Fn(&chassis::graph::PassScope<'_>, &mut dyn chassis::CommandList) -> chassis::Result<()>
            + Send
            + Sync
            + 'static,
    ) -> Pass {
        self.pass.body = Arc::new(body);
        self.pass
    }

    /// The default body: record nothing.
    ///
    /// For a pass that only exists to force an ordering, which is a real use (a
    /// compute pass whose output the graph must order before a copy) and is
    /// otherwise impossible to express, because a pass with no body and no
    /// declaration is `UnsubmittedPass`.
    pub fn build(self) -> Pass {
        self.pass
    }
}

impl Pass {
    pub fn colors(&self) -> &[ColorTarget] {
        &self.colors
    }

    pub fn depth(&self) -> Option<&DepthTarget> {
        self.depth.as_ref()
    }

    pub fn reads(&self) -> &[ReadAccess] {
        &self.reads
    }

    pub fn writes(&self) -> &[WriteAccess] {
        &self.writes
    }

    /// Total declared touches, for the frame's declaration stats.
    pub fn access_count(&self) -> u32 {
        self.reads.len() as u32 + self.writes.len() as u32
    }

    /// Shareable recorder. `Arc` rather than a bare trait object because the
    /// executor may hold it while the frame goes on being declared.
    pub fn recorder(&self) -> Arc<dyn chassis::graph::PassRecorder> {
        Arc::new(BodyRecorder {
            body: Arc::clone(&self.body),
        })
    }
}

/// Adapts a closure to [`chassis::graph::PassRecorder`].
struct BodyRecorder {
    body: Arc<RecordFn>,
}

impl chassis::graph::PassRecorder for BodyRecorder {
    fn record(
        &self,
        scope: &chassis::graph::PassScope<'_>,
        list: &mut dyn chassis::CommandList,
    ) -> chassis::Result<()> {
        (self.body)(scope, list)
    }
}
