//! Barriers. There are none.
//!
//! ## Status
//!
//! **This module exists to say that there is nothing here, and why that is not a
//! gap.**
//!
//! `chassis::graph` emits barriers from declared reads and writes, and the
//! Vulkan backend lowers them to `vk::ImageMemoryBarrier`. wgpu emits nothing:
//! it infers transitions from the access a render pass declares, and
//! `wgpu::RenderPass` takes no barrier parameter.
//!
//! Recording barriers here would not be a harmless extra. It would be
//! commands that cannot be expressed, and an `Encodable` trait does not exist
//! in the API -- there is nothing to encode them into. The correct wgpu
//! executor is one that *ignores* the barrier output, which is exactly what
//! `WgpuRenderGraphExecutor` does.
//!
//! So this file is a comment. If someone later adds a function here, the first
//! question should be what it would encode.
