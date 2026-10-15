use std::fmt;

use crate::desc::{
    BufferDesc, DescriptorSetLayoutDesc, PipelineDesc, RenderPassDesc, SamplerDesc, TextureDesc,
};
use crate::format::TextureFormat;
use crate::handle::{BindGroup, Buffer, ImageView, Pipeline, Sampler, Shader, Texture};

/// Error type for every fallible RHI operation.
///
/// One type at the boundary rather than one per operation, because callers at
/// this layer treat a failed pipeline creation and a failed submit the same way:
/// log it with context and tear the frame down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The backend could not satisfy the request on this device.
    Unsupported(String),
    /// A handle did not resolve: wrong generation, or never created.
    InvalidHandle,
    /// A descriptor that fails backend validation.
    InvalidDescriptor(String),
    /// Device lost, or a surface/out-of-date condition.
    DeviceLost,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(what) => write!(f, "unsupported: {what}"),
            Error::InvalidHandle => write!(f, "invalid handle"),
            Error::InvalidDescriptor(why) => write!(f, "invalid descriptor: {why}"),
            Error::DeviceLost => write!(f, "device lost"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Extent2d {
    pub width: u32,
    pub height: u32,
}

/// Which submit/barrier path the device supports.
///
/// Carried over from `VulkanExtensionCheck.h:12-17`. The C++ version gates on
/// this correctly: every barrier and submit goes through one lowering point
/// that picks the modern or legacy encoding from the tier.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SyncTier {
    /// Plain `vkCmdPipelineBarrier` + `vkQueueSubmit`.
    #[default]
    Legacy,
    /// `vkCmdPipelineBarrier2` + `vkQueueSubmit2` (Vulkan 1.3).
    Sync2,
    /// As above, plus timeline semaphores.
    Sync2Timeline,
}

impl SyncTier {
    /// True for the two tiers that spell barriers `vkCmdPipelineBarrier2`.
    ///
    /// The question callers actually ask. `Sync2` and `Sync2Timeline` differ in
    /// semaphore type, not in barrier spelling, so anything that lowers a
    /// barrier should ask this rather than match on a tier name -- matching the
    /// name is how the next tier gets added and one branch silently stops
    /// compiling.
    pub fn uses_sync2(self) -> bool {
        matches!(self, SyncTier::Sync2 | SyncTier::Sync2Timeline)
    }

    pub fn supports_sync2(self) -> bool {
        !matches!(self, SyncTier::Legacy)
    }

    pub fn supports_timeline_semaphores(self) -> bool {
        matches!(self, SyncTier::Sync2Timeline)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GpuInfo {
    pub name: String,
    pub sync_tier: SyncTier,
    pub max_texture_size: u32,
    /// Number of frames the backend allows in flight.
    pub frames_in_flight: u32,
}

/// The swapchain, readable without mutable access so passes can query extent.
pub trait Swapchain {
    fn extent(&self) -> Extent2d;
    fn format(&self) -> TextureFormat;
    /// Acquired for the current frame. Valid between `Gpu::frame_begin` and
    /// `Gpu::submit`.
    fn current_image_view(&self) -> ImageView;
    /// Index of the acquired image.
    fn frame_index(&self) -> u32;
    /// How many images the swapchain has.
    ///
    /// A frame loop needs this and it is not derivable: one image means the CPU
    /// waits for the GPU every frame, two means it never does. A backend with no
    /// swapchain returns `None` from `Gpu::swapchain` instead of reporting `0`,
    /// so a `0` here would mean a real swapchain with no images, which cannot
    /// happen.
    fn image_count(&self) -> u32;
    fn is_out_of_date(&self) -> bool;
}

/// A secondary command list.
///
/// **No barrier methods, by design.** Barriers come from the render graph, which
/// derives them from the declared read/write sets. The moment a pass can hand-
/// write a barrier, the graph can no longer prove the frame is correct — which
/// is precisely the state the C++ engine is stuck in, with
/// `BeginRenderPass` hand-writing `oldLayout = eUndefined`
/// (`VulkanCommandBuffer.cpp:104,153`) alongside the graph's own synthesis.
pub trait CommandList {
    fn begin_rendering(&mut self, desc: &RenderPassDesc);
    fn end_rendering(&mut self);

    fn set_viewport(&mut self, extent: Extent2d);
    fn set_scissor(&mut self, x: u32, y: u32, width: u32, height: u32);

    fn bind_pipeline(&mut self, pipeline: &Pipeline);
    fn bind_group(&mut self, set: u32, group: &BindGroup);
    fn set_push_constants(&mut self, data: &[u8]);

    fn bind_vertex_buffer(&mut self, slot: u32, buffer: &Buffer, offset: u64);
    fn bind_index_buffer(&mut self, buffer: &Buffer, offset: u64);

    fn draw(&mut self, vertices: u32, instances: u32);
    fn draw_indexed(&mut self, indices: u32, instances: u32);
    fn dispatch(&mut self, groups_x: u32, groups_y: u32, groups_z: u32);

    fn copy_buffer(&mut self, src: &Buffer, dst: &Buffer);

    /// Copy a texture's whole mip 0 into a buffer, for reading it back.
    ///
    /// Part of the RHI rather than a backend extra because a debug view, a
    /// screenshot and a GPU test all need it, and none of them should have to
    /// reach past the trait to get it. The graph orders it: declaring a pass
    /// that reads the texture and writes the buffer emits the image-to-buffer
    /// transition, which is the thing that has to be right.
    fn copy_texture_to_buffer(&mut self, src: &Texture, dst: &Buffer);
}

/// Resource creation and destruction. Separate from `Gpu` so a backend can make
/// creation thread-safe while frame recording stays single-owner.
pub trait Resources {
    fn create_buffer(&mut self, desc: BufferDesc, initial: Option<&[u8]>) -> Result<Buffer>;
    fn create_texture(&mut self, desc: TextureDesc) -> Result<Texture>;
    fn create_image_view(&mut self, texture: &Texture) -> Result<ImageView>;
    /// `spirv` is the output of `forge`; the module is cached by hash so a
    /// rebuild with identical source costs nothing.
    fn create_shader(&mut self, label: &str, spirv: &[u8]) -> Result<Shader>;
    fn create_pipeline(&mut self, desc: PipelineDesc) -> Result<Pipeline>;
    /// A bind group: a set of resources a pipeline can read, written once and
    /// bound many times.
    ///
    /// ## Why this takes a descriptor rather than handles
    ///
    /// It used to take `layout: &[u32]`, and that is why *neither backend
    /// implemented it*. A list of opaque `u32`s carries no binding types, no
    /// counts and no stages, so there was nothing to build a descriptor set
    /// layout from -- the only correct implementation was to return an error, and
    /// both did, with a comment saying a layout was needed.
    ///
    /// The layout is the same [`DescriptorSetLayoutDesc`] a pipeline declares for
    /// its set, so a bind group and the pipeline that reads it are built from one
    /// description and cannot disagree.
    ///
    /// ## Buffers, images and samplers
    ///
    /// One entry per *explicitly bound* binding. Buffers carry a byte offset;
    /// images are a view and samplers are a sampler, both keyed by binding
    /// number.
    ///
    /// Explicit numbers rather than positional, because a layout may declare
    /// bindings 0 and 7 and leave the gap for another resource kind, and
    /// "whichever index you passed" is not a rule anyone can hold in their head.
    ///
    /// ## Why images and samplers are two arguments
    ///
    /// Because Vulkan wants them as two bindings when they are two, and wgpu
    /// wants one. `DescriptorKind::CombinedImageSampler` is the case where they
    /// are one, and the backend collapses the two lists for it -- so a caller
    /// writes one binding number and gets the pairing the layout asked for.
    fn create_bind_group(
        &mut self,
        layout: &DescriptorSetLayoutDesc,
        buffers: &[(u32, Buffer, u64)],
        images: &[(u32, ImageView)],
        samplers: &[(u32, Sampler)],
    ) -> Result<BindGroup>;

    /// A sampler.
    ///
    /// Its own object rather than parameters to `create_bind_group`, because a
    /// sampler outlives the group it is written into and is shared by every
    /// group that samples the same way.
    fn create_sampler(&mut self, desc: &SamplerDesc) -> Result<Sampler>;

    fn destroy_buffer(&mut self, buffer: Buffer);
    fn destroy_texture(&mut self, texture: Texture);

    /// Puts `data` into a texture.
    ///
    /// ## Why this exists rather than `create_texture` taking initial data
    ///
    /// `create_buffer` does take it, and asymmetry is the whole problem: a
    /// sampled texture was necessarily empty, so the descriptor path -- sampler,
    /// image, bind group, draw -- could be built but never *proven*. Every way
    /// to fake the contents produced a green test for a renderer that cannot
    /// load an image.
    ///
    /// ## Row order, and why it is not configurable here
    ///
    /// Top-to-bottom. `origin` is top-left and `extent.height` counts downwards,
    /// which is the opposite of Vulkan's and of wgpu's. A per-backend or
    /// per-call option makes every caller carry the knowledge, and a test that
    /// forgets it fails in a way that looks like a wrong texture. One rule,
    /// both backends, documented here.
    ///
    /// ## Returns when submitted, not when finished
    ///
    /// The bytes are in a staging buffer and a copy has been *queued*. Destroying
    /// the texture before the queue drains is
    /// `VUID-vkDestroyImage-image-01000` -- "currently in use by VkCommandBuffer"
    /// -- which is what validation reported the first time this ran, against the
    /// test written to prove it worked.
    ///
    /// So this is not synchronous and no caller may treat it as one. The honest
    /// shape is a wait before anything the upload touched is destroyed, and a
    /// frame loop that waits once at frame end rather than once per upload.
    ///
    /// An earlier version of this comment claimed the opposite, which is the
    /// worst kind of wrong: it is reassuring, and it would have been followed.
    fn upload_texture(
        &mut self,
        texture: &Texture,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()>;
    fn destroy_pipeline(&mut self, pipeline: Pipeline);
    fn destroy_shader(&mut self, shader: Shader);
    fn destroy_bind_group(&mut self, group: BindGroup);
    fn destroy_sampler(&mut self, sampler: Sampler);
}

pub type PassId = u32;

/// The one owner of synchronization.
pub trait RenderGraph {
    /// Clears the graph for the next frame. The graph is rebuilt and recompiled
    /// every frame; there is deliberately no compile cache.
    fn reset(&mut self);

    fn add_pass(&mut self, desc: RenderPassDesc) -> PassId;
    /// `producer` must complete before `consumer` records.
    fn add_dependency(&mut self, producer: PassId, consumer: PassId);

    /// Topological sort, dead-pass cull, lifetimes, barrier synthesis.
    fn compile(&mut self) -> Result<Vec<PassId>>;

    /// Records every pass into its own command list and submits, in compiled
    /// order.
    ///
    /// **The command lists are no longer passed in.** They used to be, and that
    /// was a wart: it said the *caller* owned command-buffer lifetime, while both
    /// executors own it. Vulkan's needs one pool per list -- the pool is the
    /// external-synchronisation object for recording -- and wgpu's encoder holds a
    /// borrow the caller cannot express. Making the caller manage either is a
    /// requirement neither API actually has.
    ///
    /// `jobs` is the parallelism a backend *may* use for recording. It is a
    /// parameter rather than a promise because one backend recording on N threads
    /// and another on one is a legitimate difference, and the trait should not
    /// imply otherwise.
    fn execute(&mut self, jobs: &grind::Jobs) -> Result<()>;
}

/// The device.
///
/// `gpu` is `None` in headless mode; that is the whole of rule 6 in the
/// architecture doc, and it is why every field the GPU touches is an
/// `Option<Box<dyn Gpu>>` rather than a concrete backend type.
pub trait Gpu {
    fn info(&self) -> &GpuInfo;
    /// The swapchain, or `None` when there is not one.
    ///
    /// `Option`, not a bare reference. A headless run -- a GPU test, a
    /// server-side frame, a CI job with no window -- has a real `Gpu` and no
    /// swapchain, and a signature that says otherwise forces every caller to
    /// carry a dummy. It also makes the difference between backends honest:
    /// wgpu can create a headless device and Vulkan cannot create a window
    /// without one.
    fn swapchain(&self) -> Option<&dyn Swapchain>;
    fn graph(&mut self) -> &mut dyn RenderGraph;
    fn resources(&mut self) -> &mut dyn Resources;

    /// Record and submit one compiled frame from a renderer frontend, in the
    /// graph's compiled order.
    ///
    /// The frame was compiled by `chassis::graph`; the backend owns the executor,
    /// the slot table and the command lists, so this is the one call that turns a
    /// frontend's declaration into GPU work.
    ///
    /// **Not `present`.** The frame loop is `frame_begin` -> `record_frame` ->
    /// [`Gpu::submit`], and `submit` is where the present goes: one per frame, at
    /// the point every caller reaches through `Box<dyn Gpu>`, which is what
    /// `submit`'s own sentence ("submit every list produced this frame, then
    /// present") describes. Two presents in one frame is a present of an image
    /// nobody holds.
    ///
    /// ## Why these three arguments
    ///
    /// `CompiledGraph` is what both executors already take
    /// (`VulkanRenderGraphExecutor::execute`, `WgpuRenderGraphExecutor::execute`)
    /// and what a frontend's `RenderGraph::compile` produces -- a `CompiledFrame`
    /// is that graph plus the builder's passes and resource names, which a
    /// backend has no use for.
    ///
    /// `SlotTable` is the frontend's half of the frame: the graph names resources
    /// by slot index, the RHI names them by generational handle, and only the
    /// frontend knows which handle it put at which slot. It fills every slot it
    /// created and leaves the swapchain image's unbound -- a swapchain image has
    /// no `Texture` handle -- and binding that one is the backend's job.
    ///
    /// `Arc`, not `&`, because a backend may record passes on worker threads and
    /// a closure handed to a job pool is `'static`; a borrowed recorder cannot be
    /// one.
    ///
    /// The default refuses by name rather than recording nothing: a frame that
    /// silently did nothing is the failure this method exists to remove, and a
    /// backend that has not adopted it yet should say so.
    fn record_frame(
        &mut self,
        _frame: &crate::graph::CompiledGraph,
        _slots: &crate::graph::SlotTable,
        _recorder: std::sync::Arc<dyn crate::graph::PassRecorder>,
    ) -> Result<()> {
        Err(Error::Unsupported(
            "this gpu does not execute frontend frames".to_owned(),
        ))
    }

    /// Fence wait, swapchain acquire, deferred-deletion flush.
    fn frame_begin(&mut self) -> Result<()>;
    /// Submit whatever the backend recorded for this frame, then present.
    ///
    /// This is the backend's own graph, not a frontend's: a renderer's frame goes
    /// through [`Gpu::record_frame`], which also presents when a swapchain is
    /// attached. Which of the two is reachable differs by backend — Vulkan can
    /// only run an empty `submit`, because nothing can declare into its graph
    /// adapter — so a frame loop that draws a frontend's frame calls
    /// `record_frame` and does not need `submit` at all.
    fn submit(&mut self) -> Result<()>;
    /// Blocks until the device is idle. Test and shutdown only.
    fn wait_idle(&mut self);
}
