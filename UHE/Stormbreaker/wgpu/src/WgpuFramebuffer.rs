//! Framebuffers.
//!
//! ## Status
//!
//! No code. `chassis::Gpu` renders with dynamic rendering on Vulkan and wgpu
//! equivalents the attachments directly to the pass, so neither backend has a
//! framebuffer object. The file exists in C++ because C++ uses a render pass
//! object; the Rust port uses dynamic rendering / implicit attachments on both
//! sides and has no equivalent.
