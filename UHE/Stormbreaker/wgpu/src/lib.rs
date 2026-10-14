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
pub mod wgpu_adapter;
#[path = "WgpuAllocation.rs"]
pub mod wgpu_allocation;
#[path = "WgpuBarrierEncoder.rs"]
pub mod wgpu_barrier_encoder;
#[path = "WgpuBuffer.rs"]
pub mod wgpu_buffer;
#[path = "WgpuCommandPool.rs"]
pub mod wgpu_command_pool;
#[path = "WgpuContext.rs"]
pub mod wgpu_context;
#[path = "WgpuError.rs"]
pub mod wgpu_error;
#[path = "WgpuFramebuffer.rs"]
pub mod wgpu_framebuffer;
#[path = "WgpuImageView.rs"]
pub mod wgpu_image_view;
#[path = "WgpuSwapchain.rs"]
pub mod wgpu_swapchain;
#[path = "WgpuTexture.rs"]
pub mod wgpu_texture;
#[path = "WgpuTypes.rs"]
pub mod wgpu_types;
#[path = "WgpuUpload.rs"]
pub mod wgpu_upload;
