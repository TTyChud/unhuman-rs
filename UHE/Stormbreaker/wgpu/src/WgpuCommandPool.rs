//! Command pools. There are none.
//!
//! ## Status
//!
//! **wgpu has no command pool**, and this is the single fact that makes parallel
//! recording on wgpu different from Vulkan.
//!
//! In Vulkan the pool -- not the buffer -- is the external-synchronisation object
//! for recording, which is why one pool per pass is what makes concurrent
//! recording sound. wgpu has no such object: `CommandEncoder`s are independent
//! values with no shared mutable state, so two threads may build two encoders
//! over one device with nothing to coordinate.
//!
//! That makes wgpu's parallel recording easier than Vulkan's, not harder, and it
//! is the reason the two executors do not share a body.
