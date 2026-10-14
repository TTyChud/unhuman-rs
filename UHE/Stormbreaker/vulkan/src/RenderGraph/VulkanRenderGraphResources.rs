//! Per-frame registration of live Vulkan objects behind graph handles.
//!
//! ## Why this is a separate file
//!
//! The graph speaks in handles: `chassis::Texture`, `chassis::Buffer`. Vulkan
//! wants `vk::Image`, `vk::ImageView`, `vk::Buffer`. Something has to hold the
//! pairing for the duration of a frame, and the C++ engine puts it in
//! `VulkanRenderGraphResources.{h,cpp}` with `RegisterTexture` / `RegisterBuffer`
//! / `ClearRegistrations` / `ResolveView` / `ResolveBuffer` / `ResolveExtent`.
//!
//! ## `ClearRegistrations` is not an optimisation
//!
//! A texture destroyed after resolve but before submit leaves a dangling
//! `vk::Image` in the table, and the next frame's barrier names an image the
//! driver has freed. Clearing every frame is what makes the table's lifetime
//! *exactly* one frame rather than "since it was registered".
//!
//! ## Why the executor does not just ask the registries
//!
//! It could, and once did. But a swapchain image is not in `Textures` at all --
//! the swapchain owns it and hands it out -- so "resolve every handle through
//! one registry" was never true, only nearly true, and the nearly-true version
//! had no path for the one handle that matters most.

use ash::vk;
use chassis::graph::pass::SlotTable;

use crate::vulkan_error::{BackendError, Result};

/// Handle-to-driver-object pairings for one frame.
///
/// Not `Sync`-shared: the executor owns one and reads it while resolving, before
/// any worker thread exists for this frame. Resolution happens on the submitting
/// thread; recording happens afterwards and only sees `Textures` / `Buffers`
/// directly.
#[derive(Default)]
pub struct FrameRegistrations {
    images: Vec<(chassis::Texture, vk::Image)>,
    views: Vec<(chassis::Texture, vk::ImageView)>,
    buffers: Vec<(chassis::Buffer, vk::Buffer)>,
    extents: Vec<(chassis::Texture, (u32, u32))>,
}

impl FrameRegistrations {
    pub fn new() -> Self {
        Self::default()
    }

    /// A texture and the driver object behind it.
    ///
    /// `view` may be `VK_NULL_HANDLE` for a texture that is only ever a
    /// transfer source or destination and is never sampled. Storing the null
    /// view is better than not storing it: `resolve_view` can then say "this
    /// texture has no view" rather than "this texture is unknown", which are
    /// different bugs.
    pub fn register_texture(
        &mut self,
        texture: chassis::Texture,
        image: vk::Image,
        view: vk::ImageView,
        extent: (u32, u32),
    ) {
        self.images.push((texture, image));
        self.views.push((texture, view));
        self.extents.push((texture, extent));
    }

    pub fn register_buffer(&mut self, buffer: chassis::Buffer, vk_buffer: vk::Buffer) {
        self.buffers.push((buffer, vk_buffer));
    }

    /// Drops every registration.
    ///
    /// Called at the start of every frame, so the table's lifetime is one frame
    /// and a destroyed texture cannot still be named by tomorrow's barriers.
    pub fn clear(&mut self) {
        self.images.clear();
        self.views.clear();
        self.buffers.clear();
        self.extents.clear();
    }

    pub fn image(&self, texture: chassis::Texture) -> Option<vk::Image> {
        self.images
            .iter()
            .find(|(handle, _)| *handle == texture)
            .map(|(_, image)| *image)
    }

    pub fn view(&self, texture: chassis::Texture) -> Option<vk::ImageView> {
        self.views
            .iter()
            .find(|(handle, _)| *handle == texture)
            .map(|(_, view)| *view)
    }

    pub fn buffer(&self, buffer: chassis::Buffer) -> Option<vk::Buffer> {
        self.buffers
            .iter()
            .find(|(handle, _)| *handle == buffer)
            .map(|(_, vk_buffer)| *vk_buffer)
    }

    pub fn extent(&self, texture: chassis::Texture) -> Option<(u32, u32)> {
        self.extents
            .iter()
            .find(|(handle, _)| *handle == texture)
            .map(|(_, extent)| *extent)
    }

    /// Every texture a compiled frame touches, resolved.
    ///
    /// ## Resolve-then-record, and why
    ///
    /// The whole frame is resolved *before* any command buffer is recorded, so a
    /// slot pointing at nothing is one error reported before a single
    /// `vkBeginCommandBuffer` -- rather than a failure on whichever worker
    /// happened to reach that pass, which would name a thread and not a handle.
    ///
    /// The C++ engine does this too, and calls the result an `RGResolvedFrame`.
    pub fn resolve_frame(
        &self,
        slots: &SlotTable,
        textures: &crate::vulkan_texture::Textures,
        buffers: &crate::vulkan_buffer::Buffers,
        compiled: &chassis::graph::CompiledGraph,
    ) -> Result<()> {
        for pass in &compiled.passes {
            for color in &pass.colors {
                let handle = slots
                    .texture(color.texture)
                    .ok_or(BackendError::InvalidHandle)?;
                textures.get(handle).ok_or(BackendError::InvalidHandle)?;
            }
            if let Some(depth) = &pass.depth {
                let handle = slots
                    .texture(depth.texture)
                    .ok_or(BackendError::InvalidHandle)?;
                textures.get(handle).ok_or(BackendError::InvalidHandle)?;
            }
            for barrier in pass.pre_barriers.iter().chain(&pass.post_barriers) {
                let handle = slots
                    .texture(barrier.texture)
                    .ok_or(BackendError::InvalidHandle)?;
                textures.get(handle).ok_or(BackendError::InvalidHandle)?;
            }
            // Only `pre_buffer_barriers` exists: the graph has no post buffer
            // barriers, because a buffer's ending state is what the next pass
            // reads and the next pass's own pre-barrier says so. Adding a post
            // list later needs this loop to change with it.
            for barrier in &pass.pre_buffer_barriers {
                let handle = slots
                    .buffer(barrier.buffer)
                    .ok_or(BackendError::InvalidHandle)?;
                buffers.get(handle).ok_or(BackendError::InvalidHandle)?;
            }
        }
        Ok(())
    }

    /// How many textures are registered. A count that does not return to zero
    /// between frames means `clear` is not being called.
    pub fn len(&self) -> usize {
        self.images.len()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}
