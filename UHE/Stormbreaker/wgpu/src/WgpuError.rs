//! Error type.
//!
//! ## Status
//!
//! **Not yet.** wgpu errors are `wgpu::Error` plus panics, and both leak straight
//! into `chassis::Error` at the call sites.
//!
//! This is a real gap and it is the one that makes wgpu harder to reason about
//! than Vulkan: an infallible wgpu call (shader modules, bind groups) has no
//! error to convert, so the check has to happen *before* the call or the caller
//! gets a panic. Two such checks exist today, found the hard way -- see
//! `WgpuResources::bind_group_layout` and the `count: Some(1)` note.
