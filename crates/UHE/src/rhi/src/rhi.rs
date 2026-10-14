use std::fmt;

use crate::desc::{BufferDesc, PipelineDesc, RenderPassDesc, TextureDesc};
use crate::format::TextureFormat;
use crate::handle::{BindGroup, Buffer, ImageView, Pipeline, Shader, Texture};

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
    /// One entry per binding: the buffer plus the byte offset into it.
    fn create_bind_group(&mut self, layout: &[u32], buffers: &[(Buffer, u64)]) -> Result<BindGroup>;

    fn destroy_buffer(&mut self, buffer: Buffer);
    fn destroy_texture(&mut self, texture: Texture);
    fn destroy_pipeline(&mut self, pipeline: Pipeline);
    fn destroy_shader(&mut self, shader: Shader);
    fn destroy_bind_group(&mut self, group: BindGroup);
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

    /// Records every pass into its own command list, then hands back the
    /// submission order. Recording runs across `jobs`; submission stays on the
    /// calling thread because Vulkan requires it.
    fn execute(&mut self, jobs: &grind::Jobs, lists: &mut [Box<dyn CommandList>]) -> Result<()>;
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

    /// Fence wait, swapchain acquire, deferred-deletion flush.
    fn frame_begin(&mut self) -> Result<()>;
    /// Submit every list produced this frame, then present.
    fn submit(&mut self) -> Result<()>;
    /// Blocks until the device is idle. Test and shutdown only.
    fn wait_idle(&mut self);
}