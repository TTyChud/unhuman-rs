//! Image views.
//!
//! Mirrors `VulkanImageView.cpp`, split out of `vulkan_texture.rs` because the C++
//! tree has both files and because the split is meaningful: an image is memory,
//! a view is a *window onto* it, and the two have different lifetimes and
//! different reasons to be recreated.
//!
//! `VulkanTexture.cpp` creates one view per image eagerly. So does this, through
//! `create_default`, but a view can also be made separately for a subresource
//! range or a cube face, which is what a shadow map and its comparison sampler
//! need.

use ash::vk;

use crate::vulkan_error::{BackendError, Result};

/// A `VkImageView` plus the range it covers.
#[derive(Clone, Copy, Debug)]
pub struct ImageView {
    view: vk::ImageView,
    subresource: vk::ImageSubresourceRange,
}

impl ImageView {
    pub fn new(view: vk::ImageView, subresource: vk::ImageSubresourceRange) -> Self {
        Self { view, subresource }
    }

    pub fn handle(&self) -> vk::ImageView {
        self.view
    }

    pub fn subresource(&self) -> vk::ImageSubresourceRange {
        self.subresource
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewKind {
    Dim1D,
    Dim2D,
    Dim3D,
    Cube,
    /// Dim2D array, one layer per array element.
    Array2D,
}

/// The one view covering a whole 2D image, used for colour and depth targets and
/// for a sampled 2D texture.
///
/// `UNDEFINED` is the only legal layout for a view created alongside its image;
/// the render graph transitions the image afterwards.
pub fn create_default(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
    aspect: vk::ImageAspectFlags,
    mip_levels: u32,
    layers: u32,
) -> Result<ImageView> {
    let subresource = vk::ImageSubresourceRange {
        aspect_mask: aspect,
        base_mip_level: 0,
        level_count: mip_levels,
        base_array_layer: 0,
        layer_count: layers,
    };
    let info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(subresource);

    let view = unsafe { device.create_image_view(&info, None) }
        .map_err(|e| BackendError::vk("vkCreateImageView", e))?;
    Ok(ImageView::new(view, subresource))
}

/// A view over a single mip level. Shadow maps sample exactly one level, and
/// giving them a range would let a shader read a level it was never compiled
/// against.
pub fn create_mip(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
    aspect: vk::ImageAspectFlags,
    mip: u32,
    layers: u32,
) -> Result<ImageView> {
    let subresource = vk::ImageSubresourceRange {
        aspect_mask: aspect,
        base_mip_level: mip,
        level_count: 1,
        base_array_layer: 0,
        layer_count: layers,
    };
    let info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(subresource);

    let view = unsafe { device.create_image_view(&info, None) }
        .map_err(|e| BackendError::vk("vkCreateImageView", e))?;
    Ok(ImageView::new(view, subresource))
}

/// A single cube face. `VK_IMAGE_CREATE_CUBE_COMPATIBLE` has to have been set on
/// the image; the driver will reject this otherwise with a validation error rather
/// than anything actionable.
pub fn create_cube_face(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
    aspect: vk::ImageAspectFlags,
    mip: u32,
    face: u32,
) -> Result<ImageView> {
    if face >= 6 {
        return Err(BackendError::InvalidHandle);
    }
    let subresource = vk::ImageSubresourceRange {
        aspect_mask: aspect,
        base_mip_level: mip,
        level_count: 1,
        base_array_layer: face,
        layer_count: 1,
    };
    let info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::CUBE)
        .format(format)
        .subresource_range(subresource);

    let view = unsafe { device.create_image_view(&info, None) }
        .map_err(|e| BackendError::vk("vkCreateImageView", e))?;
    Ok(ImageView::new(view, subresource))
}

pub fn destroy(device: &ash::Device, view: vk::ImageView) {
    unsafe { device.destroy_image_view(view, None) };
}