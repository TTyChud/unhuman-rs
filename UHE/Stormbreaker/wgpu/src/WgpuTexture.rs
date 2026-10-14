//! Textures.
//!
//! ## Status
//!
//! Split out of `WgpuResources`. Thin: `wgpu::Texture` owns its memory and its
//! layout. Vulkan needs `VulkanTexture` and `VulkanTexture2D` separately because
//! it has no concept of a texture array.
