//! Resource states, accesses and stages.
//!
//! Backend-neutral on purpose. `ImageState` maps 1:1 onto the C++ enum
//! (`VulkanTypes.h`), but nothing here is a Vulkan type, so the compiler can be
//! exercised with no device and no driver.

/// Where a texture's memory is expected to be at a given point in the frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum ImageState {
    /// Contents undefined. Also the valid source state on first use of a
    /// transient resource, which is what lets the first barrier be free.
    #[default]
    Undefined,
    ColorAttachment,
    DepthAttachment,
    ShaderRead,
    TransferSrc,
    TransferDst,
    Present,
    Storage,
}

impl ImageState {
    pub fn is_defined(self) -> bool {
        self != ImageState::Undefined
    }
}

/// What a pass does with memory, for barrier scope selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum Access {
    #[default]
    None,
    TransferRead,
    TransferWrite,
    ShaderRead,
    ShaderWrite,
    ColorWrite,
    DepthWrite,
    MemoryRead,
    MemoryWrite,
}

/// Pipeline stage, for `srcStageMask`/`dstStageMask`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum Stage {
    #[default]
    None,
    TopOfPipe,
    Transfer,
    ComputeShader,
    VertexShader,
    FragmentShader,
    ColorAttachmentOutput,
    EarlyFragmentTests,
    LateFragmentTests,
    BottomOfPipe,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum PassType {
    #[default]
    Graphics,
    Compute,
    Transfer,
}

/// Which queue a pass records into.
///
/// Carried through compilation but not yet a split point. The C++ carries
/// `RGQueue` for the same reason and nothing branches on it until a second
/// queue family exists; splitting later is a compiler change, not a graph
/// rewrite.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum Queue {
    #[default]
    Graphics,
    Compute,
    Transfer,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LoadOp {
    Load,
    #[default]
    Clear,
    /// Valid as a source state for the first use of a transient resource, and
    /// the honest replacement for the C++ habit of hardcoding
    /// `oldLayout = eUndefined` on every entry
    /// (`VulkanCommandBuffer.cpp:104,153`).
    DontCare,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ColorAttachment {
    pub texture: u32,
    pub load: LoadOp,
    pub clear: [f32; 4],
    /// MSAA resolve target, or `u32::MAX` for none.
    pub resolve: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DepthAttachment {
    pub texture: u32,
    pub load: LoadOp,
    pub clear_depth: f32,
}

pub const NO_RESOURCE: u32 = u32::MAX;

/// "The rest of the buffer", for a barrier whose range the graph cannot know.
///
/// Spelled here rather than reusing `vk::WHOLE_SIZE`, because the graph is
/// backend-neutral by construction and must not name a Vulkan constant. The
/// lowering maps this to the driver's value.
pub const WHOLE_SIZE: u64 = u64::MAX;

/// A texture slot index.
pub type TextureId = u32;
/// A buffer slot index.
pub type BufferId = u32;

/// One declared read or write, bound to the version current at declaration.
///
/// A write declares the version it *produces*; a read declares the latest
/// version that existed. Those `(resource, version)` pairs are what the
/// compiler turns into edges and barriers, so the declaration order itself is
/// already a topological order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ResourceAccess {
    Texture(TextureId, u32),
    Buffer(BufferId, u32),
}

impl ResourceAccess {
    pub fn slot(self) -> u32 {
        match self {
            ResourceAccess::Texture(id, _) => id,
            ResourceAccess::Buffer(id, _) => id,
        }
    }

    pub fn version(self) -> u32 {
        match self {
            ResourceAccess::Texture(_, v) | ResourceAccess::Buffer(_, v) => v,
        }
    }

    pub fn is_texture(self) -> bool {
        matches!(self, ResourceAccess::Texture(..))
    }

    /// `Name.vN`, for dumps and error messages.
    pub fn label(self, name: &str) -> String {
        format!("{name}.v{}", self.version())
    }
}

/// A pass as declared. No closures here: the compile step produces pure data,
/// and the backend looks up the recording function by declaration index.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PassDesc {
    pub name: String,
    pub kind: PassType,
    pub queue: Queue,
    pub colors: Vec<ColorAttachment>,
    pub depth: Option<DepthAttachment>,
    pub reads: Vec<ResourceAccess>,
    pub writes: Vec<ResourceAccess>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    /// Handle or slot index that was never created, or was freed.
    UnknownResource,
    /// An attachment's texture was not also declared with a write.
    UndeclaredAttachment,
    /// One pass wrote the same resource twice.
    DoubleWrite,
    /// A resource was read at a version that no pass produces.
    ReadBeforeWrite,
    /// `add_pass` was never given a body — a placeholder left in the graph.
    UnsubmittedPass,
    /// One pass needed the same resource in two different states.
    PassStateConflict,
    /// Kahn's queue emptied early.
    Cycle,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValidationError {
    pub kind: ErrorKind,
    pub pass: String,
    pub resource: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            pass: String::new(),
            resource: String::new(),
            message: message.into(),
        }
    }

    pub fn in_pass(mut self, pass: &str) -> Self {
        self.pass = pass.to_string();
        self
    }

    pub fn on_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = resource.into();
        self
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.kind)?;
        if !self.pass.is_empty() {
            write!(f, " in pass '{}'", self.pass)?;
        }
        if !self.resource.is_empty() {
            write!(f, " (resource '{}')", self.resource)?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A texture state transition, in backend-neutral terms.
///
/// The C++ compiler writes `VkImageMemoryBarrier` here
/// (`VulkanRenderGraphCompiler.cpp:406-476`). Emitting this instead keeps the
/// compiler testable without a device, and leaves exactly one place in
/// stormbreaker that turns a transition into `vkCmdPipelineBarrier2` or
/// `vkCmdPipelineBarrier` depending on `SyncTier`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageBarrier {
    pub texture: TextureId,
    pub src_state: ImageState,
    pub dst_state: ImageState,
    pub src_access: Access,
    pub dst_access: Access,
    pub src_stage: Stage,
    pub dst_stage: Stage,
}

impl ImageBarrier {
    /// True when the transition is a no-op, so the backend can drop it.
    pub fn is_empty(&self) -> bool {
        self.src_state == self.dst_state && self.src_access == self.dst_access
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BufferBarrier {
    pub buffer: BufferId,
    pub src_access: Access,
    pub dst_access: Access,
    pub src_stage: Stage,
    pub dst_stage: Stage,
    pub offset: u64,
    pub size: u64,
}

/// Access scope implied by a state, for the *destination* side of a barrier.
pub fn access_for_state(state: ImageState) -> Access {
    match state {
        ImageState::Undefined => Access::None,
        ImageState::ColorAttachment => Access::ColorWrite,
        ImageState::DepthAttachment => Access::DepthWrite,
        ImageState::ShaderRead => Access::ShaderRead,
        ImageState::Storage => Access::ShaderWrite,
        ImageState::TransferSrc => Access::TransferRead,
        ImageState::TransferDst => Access::TransferWrite,
        ImageState::Present => Access::MemoryRead,
    }
}

/// Pipeline stage implied by a state, for the destination side.
pub fn stage_for_state(state: ImageState) -> Stage {
    match state {
        ImageState::Undefined => Stage::TopOfPipe,
        ImageState::ColorAttachment => Stage::ColorAttachmentOutput,
        ImageState::DepthAttachment => Stage::EarlyFragmentTests,
        ImageState::ShaderRead => Stage::FragmentShader,
        ImageState::Storage => Stage::ComputeShader,
        ImageState::TransferSrc | ImageState::TransferDst => Stage::Transfer,
        ImageState::Present => Stage::BottomOfPipe,
    }
}

/// Stages a pass must wait on for everything it reads.
///
/// Unlike the destination side, this is a *union* over the pass's reads: a pass
/// cannot start drawing until every texture it samples is available. Getting
/// this wrong is the classic missing-barrier bug, so it is computed in one
/// place rather than left to each pass author.
pub fn src_stage_for_state(state: ImageState) -> Stage {
    stage_for_state(state)
}

/// A resource's transition between two states, or `None` when nothing is needed.
pub fn transition(texture: TextureId, src: ImageState, dst: ImageState) -> Option<ImageBarrier> {
    // Leaving Undefined never needs a real source scope: there is nothing to
    // wait for, and content is undefined by definition.
    let (src_access, src_stage) = if src.is_defined() {
        (access_for_state(src), src_stage_for_state(src))
    } else {
        (Access::None, Stage::TopOfPipe)
    };

    let barrier = ImageBarrier {
        texture,
        src_state: src,
        dst_state: dst,
        src_access,
        dst_access: access_for_state(dst),
        src_stage,
        dst_stage: stage_for_state(dst),
    };

    (!barrier.is_empty()).then_some(barrier)
}
