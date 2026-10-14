//! Sub-allocation inside a wgpu buffer.
//!
//! ## Status
//!
//! No code. Vulkan has one because it allocates its own memory; wgpu's
//! `wgpu::Buffer` already owns its allocation and gives no way to alias one.
//! !
//! This is a *port difference*, not an omission. It is listed because the C++
//! file exists and a reader comparing the two backends will look for it, and the
//! honest answer is "wgpu does not expose the primitive this would be built
//! from" -- not silence.
