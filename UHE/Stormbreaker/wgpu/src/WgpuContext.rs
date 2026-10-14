//! Device + queue + capabilities.
//!
//! ## Status
//!
//! Not yet split out. `WgpuGpu` holds device, queue, adapter index and resource
//! registries directly. Vulkan needed `VulkanContext` because device creation
//! spans instance, physical device and logical device and the handle has to
//! outlive them in the right destruction order; wgpu's `Device` is one value and
//! there is no such ordering to get wrong -- yet. Split it when something needs
//! to hold a device without owning the resources.
