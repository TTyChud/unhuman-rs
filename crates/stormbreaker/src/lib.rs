//! The Vulkan backend.
//!
//! Everything the engine knows about graphics lives behind `chassis`, so this
//! crate is the only place a Vulkan type is allowed to appear. The two files
//! that matter architecturally are `extension_check` (which sync tier a device
//! gets) and `barrier_encoder` (the single place a graph barrier becomes a
//! Vulkan command).

pub mod barrier_encoder;
pub mod extension_check;
pub mod utils;

pub use barrier_encoder::{image_barrier2, image_barrier_legacy, BarrierEncoder, Target};
pub use extension_check::{select_sync_tier, Capabilities, QueueFamily, SyncTier};
pub use extension_check::{version_major, version_minor};
