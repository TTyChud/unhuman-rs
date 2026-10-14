//! The wgpu backend.
//!
//! Same `chassis::Gpu` as the Vulkan backend, same render graph, same shaders.
//! The two are interchangeable behind a trait object and neither knows the other
//! exists.

#[path = "WgpuDevice.rs"]
pub mod wgpu_device;

#[path = "WgpuResources.rs"]
pub mod wgpu_resources;

#[path = "WgpuPipeline.rs"]
pub mod wgpu_pipeline;

#[path = "WgpuCommandBuffer.rs"]
pub mod wgpu_command_buffer;

// The executor lives in `RenderGraph/` to match the Vulkan backend, so a reader
// who knows where to look in one knows where to look in the other.
#[path = "RenderGraph/WgpuRenderGraphExecutor.rs"]
pub mod wgpu_render_graph_executor;

#[path = "WgpuGpu.rs"]
pub mod wgpu_gpu;

// Modules that exist to mirror the Vulkan backend's file set. Most are a
// split-out or a documented absence; two -- `WgpuSwapchain` and `WgpuError` --
// are real gaps, and each says so at the top of its own file.
#[path = "WgpuAdapter.rs"]
pub mod wgpuAdapter;
#[path = "WgpuAllocation.rs"]
pub mod wgpuAllocation;
#[path = "WgpuBarrierEncoder.rs"]
pub mod wgpuBarrierEncoder;
#[path = "WgpuBuffer.rs"]
pub mod wgpuBuffer;
#[path = "WgpuCommandPool.rs"]
pub mod wgpuCommandPool;
#[path = "WgpuContext.rs"]
pub mod wgpuContext;
#[path = "WgpuError.rs"]
pub mod wgpuError;
#[path = "WgpuFramebuffer.rs"]
pub mod wgpuFramebuffer;
#[path = "WgpuImageView.rs"]
pub mod wgpuImageView;
#[path = "WgpuSwapchain.rs"]
pub mod wgpuSwapchain;
#[path = "WgpuTexture.rs"]
pub mod wgpuTexture;
#[path = "WgpuTypes.rs"]
pub mod wgpuTypes;
#[path = "WgpuUpload.rs"]
pub mod wgpuUpload;
