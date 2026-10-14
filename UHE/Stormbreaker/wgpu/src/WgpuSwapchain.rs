//! The swapchain. **Not implemented.**
//!
//! ## Status
//!
//! **This is the largest single gap between the two backends.** Vulkan has a
//! working, tested windowed path; wgpu has none, and `WgpuGpu::swapchain()`
//! returns `None` unconditionally.
//!
//! Why it is not simply written yet: Vulkan's swapchain is tested against
//! `VK_EXT_headless_surface`, which lets a machine with no display build a real
//! `vk::SwapchainKHR`, acquire, present and tear down. wgpu has no equivalent --
//! a wgpu `Surface` comes from a window system, and there is no window system.
//!
//! So the blocker is upstream: `UHE/platform/` is a README. Write `eyes` first,
//! then this is about two hundred lines.
