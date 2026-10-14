//! Uploads.
//!
//! ## Status
//!
//! Split out of `WgpuResources`. Thin: `queue.write_buffer` / `write_texture` are
//! the whole of it. Vulkan needs a staging buffer, a transfer region and a
//! fence, which is three files.
