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
