//! The Vulkan backend.
//!
//! Everything the engine knows about graphics lives behind `chassis`, so this
//! crate is the only place a Vulkan type is allowed to appear. The two files
//! that matter architecturally are `extension_check` (which sync tier a device
//! gets) and `barrier_encoder` (the single place a graph barrier becomes a
//! Vulkan command).


pub mod vulkan_types;
pub mod vulkan_utils;
pub mod vulkan_error;
pub mod vulkan_extension_check;
pub mod vulkan_allocation;
pub mod vulkan_deletion_queue;
pub mod vulkan_device;
pub mod vulkan_context;
pub mod vulkan_buffer;
pub mod vulkan_texture;
pub mod vulkan_fence;
pub mod vulkan_semaphore;
pub mod vulkan_command_pool;
pub mod vulkan_command_buffer;
pub mod vulkan_frame_context;
pub mod vulkan_barrier_encoder;
pub mod vulkan_submit_encoder;
pub mod vulkan_shader;
pub mod vulkan_pipeline_cache;
pub mod vulkan_pipeline_state;
pub mod vulkan_descriptor_manager;
pub mod vulkan_upload;
pub use vulkan_barrier_encoder::{image_barrier2, image_barrier_legacy, BarrierEncoder, Target};
pub use vulkan_extension_check::{select_sync_tier, Capabilities, QueueFamily, SyncTier};
pub use vulkan_context::Context;
pub use vulkan_descriptor_manager::{BindGroupLayoutDesc, Binding, Descriptors};
pub use vulkan_device::{create_device, device_available, DeviceOptions};
pub use vulkan_pipeline_state::{PipelineResource, Pipelines};
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
