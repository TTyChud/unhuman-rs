//! The Vulkan backend.
//!
//! Everything the engine knows about graphics lives behind `chassis`, so this
//! crate is the only place a Vulkan type is allowed to appear. The two files
//! that matter architecturally are `extension_check` (which sync tier a device
//! gets) and `barrier_encoder` (the single place a graph barrier becomes a
//! Vulkan command).
//!
//! Every file here is named after the C++ file it came from, so a file can be
//! put next to its original: `VulkanBuffer.rs` beside `VulkanBuffer.cpp`. The
//! module identifier stays snake_case through `#[path]`, which keeps `use
//! crate::vulkan_buffer::...` idiomatic and avoids a non_snake_case warning on
//! every declaration.


#[path = "VulkanTypes.rs"]
pub mod vulkan_types;
#[path = "VulkanUtils.rs"]
pub mod vulkan_utils;
#[path = "VulkanError.rs"]
pub mod vulkan_error;
#[path = "VulkanExtensionCheck.rs"]
pub mod vulkan_extension_check;
#[path = "VulkanAllocation.rs"]
pub mod vulkan_allocation;
#[path = "VulkanDeletionQueue.rs"]
pub mod vulkan_deletion_queue;
#[path = "VulkanInstance.rs"]
pub mod vulkan_instance;
#[path = "VulkanPhysicalDevice.rs"]
pub mod vulkan_physical_device;
#[path = "VulkanLogicalDevice.rs"]
pub mod vulkan_logical_device;
#[path = "VulkanDevice.rs"]
pub mod vulkan_device;
#[path = "VulkanContext.rs"]
pub mod vulkan_context;
#[path = "VulkanBuffer.rs"]
pub mod vulkan_buffer;
#[path = "VulkanFramebuffer.rs"]
pub mod vulkan_framebuffer;
#[path = "VulkanImageView.rs"]
pub mod vulkan_image_view;
#[path = "VulkanScopes.rs"]
pub mod vulkan_scopes;
#[path = "VulkanSwapchain.rs"]
pub mod vulkan_swapchain;
#[path = "VulkanTexture2D.rs"]
pub mod vulkan_texture_2d;
#[path = "VulkanTexture.rs"]
pub mod vulkan_texture;
#[path = "VulkanFence.rs"]
pub mod vulkan_fence;
#[path = "VulkanSemaphore.rs"]
pub mod vulkan_semaphore;
#[path = "VulkanCommandPool.rs"]
pub mod vulkan_command_pool;
#[path = "VulkanCommandBuffer.rs"]
pub mod vulkan_command_buffer;
#[path = "VulkanFrameContext.rs"]
pub mod vulkan_frame_context;
#[path = "VulkanBarrierEncoder.rs"]
pub mod vulkan_barrier_encoder;
#[path = "VulkanSubmitEncoder.rs"]
pub mod vulkan_submit_encoder;
#[path = "VulkanShader.rs"]
pub mod vulkan_shader;
#[path = "VulkanComputePipeline.rs"]
pub mod vulkan_compute_pipeline;
#[path = "VulkanGraphicPipeline.rs"]
pub mod vulkan_graphic_pipeline;
#[path = "VulkanPipelineCache.rs"]
pub mod vulkan_pipeline_cache;
#[path = "VulkanPipelineState.rs"]
pub mod vulkan_pipeline_state;
#[path = "VulkanDescriptorPool.rs"]
pub mod vulkan_descriptor_pool;
#[path = "VulkanDescriptorSet.rs"]
pub mod vulkan_descriptor_set;
#[path = "VulkanDescriptorManager.rs"]
pub mod vulkan_descriptor_manager;
#[path = "VulkanUpload.rs"]
pub mod vulkan_upload;
pub use vulkan_barrier_encoder::{image_barrier2, image_barrier_legacy, BarrierEncoder, Target};
pub use vulkan_extension_check::{select_sync_tier, Capabilities, QueueFamily, SyncTier};
pub use vulkan_context::Context;
pub use vulkan_descriptor_manager::{BindGroupLayoutDesc, Binding, Descriptors};
pub use vulkan_descriptor_pool::{DescriptorPool, PoolSizes};
pub use vulkan_descriptor_set::{descriptor_type_of, DescriptorSet};
pub use vulkan_device::{create_device, device_available, DeviceOptions};
pub use vulkan_compute_pipeline::ComputePipelineDesc;
pub use vulkan_graphic_pipeline as graphic;
pub use vulkan_pipeline_state::{PipelineHash, PipelineResource, Pipelines};
pub use vulkan_error::{BackendError, Result};
pub use vulkan_command_buffer::Recording;
pub use vulkan_frame_context::Recorder;
pub use vulkan_barrier_encoder::{buffer_barrier2, buffer_barrier_legacy};
pub use vulkan_buffer::{BufferResource, Buffers};
pub use vulkan_deletion_queue::{DeletionQueue, Pending};
pub use vulkan_shader::{content_hash, ShaderResource, Shaders};
pub use vulkan_fence::Fence;
pub use vulkan_pipeline_cache::PipelineCache;
pub use vulkan_semaphore::Semaphore;
pub use vulkan_texture::{TextureResource, Textures};
pub use vulkan_upload::UploadRing;
pub use vulkan_allocation::{
    create_buffer, create_image_memory, find_memory_type, read_buffer, write_buffer,
};
pub use vulkan_extension_check::{version_major, version_minor};
#[path = "VulkanRenderGraphTypes.rs"]
pub mod vulkan_render_graph_types;
#[path = "VulkanRenderGraphResources.rs"]
pub mod vulkan_render_graph_resources;
#[path = "VulkanRenderGraphBuilder.rs"]
pub mod vulkan_render_graph_builder;
#[path = "VulkanRenderGraphCompiler.rs"]
pub mod vulkan_render_graph_compiler;
#[path = "VulkanRenderGraph.rs"]
pub mod vulkan_render_graph;
#[path = "VulkanRenderGraphExecutor.rs"]
pub mod vulkan_render_graph_executor;
