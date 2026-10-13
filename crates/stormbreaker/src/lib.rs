//! The Vulkan backend.
//!
//! Everything the engine knows about graphics lives behind `chassis`, so this
//! crate is the only place a Vulkan type is allowed to appear. The two files
//! that matter architecturally are `extension_check` (which sync tier a device
//! gets) and `barrier_encoder` (the single place a graph barrier becomes a
//! Vulkan command).

pub mod barrier_encoder;
pub mod buffer;
pub mod context;
pub mod deletion_queue;
pub mod device;
pub mod error;
pub mod frame_context;
pub mod memory;
pub mod extension_check;
pub mod shader;
pub mod sync;
pub mod texture;
pub mod upload;
pub mod utils;

pub use barrier_encoder::{image_barrier2, image_barrier_legacy, BarrierEncoder, Target};
pub use extension_check::{select_sync_tier, Capabilities, QueueFamily, SyncTier};
pub use context::Context;
pub use device::{create_device, device_available, DeviceOptions};
pub use error::{BackendError, Result};
pub use frame_context::{Recorder, Recording};
pub use barrier_encoder::{buffer_barrier2, buffer_barrier_legacy};
pub use buffer::{BufferResource, Buffers};
pub use deletion_queue::{DeletionQueue, Pending};
pub use shader::{content_hash, PipelineCache, ShaderResource, Shaders};
pub use sync::{Fence, Semaphore};
pub use texture::{TextureResource, Textures};
pub use upload::UploadRing;
pub use memory::{
    create_buffer, create_image_memory, find_memory_type, read_buffer, write_buffer,
};
pub use extension_check::{version_major, version_minor};
