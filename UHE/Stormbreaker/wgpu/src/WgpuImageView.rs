//! Image views.
//!
//! ## Status
//!
//! Split out of `WgpuResources`. Thin: `wgpu::TextureView` is a first-class value
//! the device makes, there is no manual descriptor to destroy, and lifetime is
//! handled by wgpu.
