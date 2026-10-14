//! Buffers.
//!
//! ## Status
//!
//! Split out of `WgpuResources`. Thin: `wgpu::Buffer` owns its own memory and its
//! own usage flags, so there is no allocation strategy, no sub-allocation and
//! no host-visible dance to get right. Vulkan needs all three.
