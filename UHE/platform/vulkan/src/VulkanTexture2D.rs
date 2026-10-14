//! 2D textures.
//!
//! Mirrors `VulkanTexture2D.cpp`, which the C++ keeps separate from
//! `VulkanTexture.cpp` because the engine's 2D renderer only ever wants a 2D
//! image with one mip and one layer, and the C++ type papers over that.
//!
//! Here the split earns its keep differently: `vulkan_texture.rs` owns the image
//! and its memory, and this file is the 2D-shaped convenience over it — the
//! defaults `TextureDesc` needs spelled out, plus a clear color. A `VulkanTexture2D`
//! equivalent that has to be constructed with a full `TextureDesc` is not saving
//! anyone anything.

use ash::vk;
use chassis::{Extent, Texture, TextureFormat, TextureUsage};

use crate::vulkan_texture::Textures;

/// The 2D defaults: one mip, one layer, one sample.
pub const DEFAULT_2D_MIPS: u32 = 1;
pub const DEFAULT_2D_LAYERS: u32 = 1;

/// A 2D texture description, pre-filled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Texture2DDesc {
    pub extent: Extent,
    pub format: TextureFormat,
    pub usage: TextureUsage,
    pub mip_levels: u32,
    pub clear: Option<[u8; 4]>,
}

impl Texture2DDesc {
    pub fn new(extent: Extent, format: TextureFormat, usage: TextureUsage) -> Self {
        Self {
            extent,
            format,
            usage,
            mip_levels: DEFAULT_2D_MIPS,
            clear: None,
        }
    }

    pub fn with_mips(mut self, mip_levels: u32) -> Self {
        self.mip_levels = mip_levels.max(1);
        self
    }

    /// Initial contents. `None` leaves the image undefined, which is right for a
    /// render target the first pass will clear.
    pub fn with_clear(mut self, clear: [u8; 4]) -> Self {
        self.clear = Some(clear);
        self
    }

    pub fn to_texture_desc(self) -> chassis::TextureDesc {
        chassis::TextureDesc {
            extent: Extent::new(self.extent.width, self.extent.height),
            format: self.format,
            usage: self.usage,
            mip_levels: self.mip_levels,
            samples: chassis::SampleCount::X1,
            initial_data: self.clear.map(|c| c.to_vec()),
        }
    }
}

/// Creates a 2D texture. A thin wrapper, so the 2D shape stays in one place.
pub fn create(
    textures: &mut Textures,
    device: &ash::Device,
    desc: Texture2DDesc,
    label: &str,
) -> crate::vulkan_error::Result<Texture> {
    textures.create(device, &desc.to_texture_desc(), label)
}

/// A common case worth naming: a colour target the renderer clears each frame.
pub fn create_color_target(
    textures: &mut Textures,
    device: &ash::Device,
    extent: Extent,
    format: TextureFormat,
    label: &str,
) -> crate::vulkan_error::Result<Texture> {
    create(
        textures,
        device,
        Texture2DDesc::new(
            extent,
            format,
            TextureUsage::COLOR_ATTACHMENT | TextureUsage::SAMPLED,
        ),
        label,
    )
}

/// A common case worth naming: something sampled by a shader.
pub fn create_sampled(
    textures: &mut Textures,
    device: &ash::Device,
    extent: Extent,
    format: TextureFormat,
    mip_levels: u32,
    label: &str,
) -> crate::vulkan_error::Result<Texture> {
    create(
        textures,
        device,
        Texture2DDesc::new(extent, format, TextureUsage::SAMPLED).with_mips(mip_levels),
        label,
    )
}

/// A depth target. Depth has no colour clear, so `LoadOp::Clear` is paired with
/// whatever clear depth the pass wants.
pub fn create_depth_target(
    textures: &mut Textures,
    device: &ash::Device,
    extent: Extent,
    format: TextureFormat,
    label: &str,
) -> crate::vulkan_error::Result<Texture> {
    create(
        textures,
        device,
        Texture2DDesc::new(
            extent,
            format,
            TextureUsage::DEPTH_ATTACHMENT | TextureUsage::SAMPLED,
        ),
        label,
    )
}

/// Whether an extent is worth allocating at all. A zero-sized swapchain image
/// means the window is minimised, and creating one is a driver error.
pub fn is_allocatable(extent: Extent) -> bool {
    extent.width > 0 && extent.height > 0
}

/// The swapchain format's natural extent, for deciding whether a recreate is
/// needed.
pub fn same_extent(a: Extent, b: Extent) -> bool {
    a.width == b.width && a.height == b.height
}

/// The layout a 2D colour target is left in after a pass that renders to it.
pub fn default_layout(format: TextureFormat) -> vk::ImageLayout {
    if format.is_depth() {
        vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
    } else {
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_one_mip_and_one_layer() {
        let desc = Texture2DDesc::new(
            Extent::new(64, 64),
            TextureFormat::Rgba8Unorm,
            TextureUsage::SAMPLED,
        );
        assert_eq!(desc.mip_levels, 1);
        assert_eq!(desc.clear, None);
    }

    #[test]
    fn clear_bytes_become_initial_data() {
        let desc = Texture2DDesc::new(
            Extent::new(4, 4),
            TextureFormat::Rgba8Unorm,
            TextureUsage::COLOR_ATTACHMENT,
        )
        .with_clear([1, 2, 3, 4]);
        let data = desc
            .to_texture_desc()
            .initial_data
            .expect("clear was dropped");
        assert_eq!(data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_minimised_extent_is_not_allocatable() {
        assert!(is_allocatable(Extent::new(1, 1)));
        assert!(!is_allocatable(Extent::new(0, 100)));
        assert!(!is_allocatable(Extent::new(100, 0)));
    }

    #[test]
    fn depth_targets_get_the_depth_layout() {
        assert_eq!(
            default_layout(TextureFormat::Depth32Float),
            vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        );
        assert_eq!(
            default_layout(TextureFormat::Rgba8Unorm),
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
        );
    }

    #[test]
    fn extent_comparison_catches_a_resize() {
        assert!(same_extent(Extent::new(8, 8), Extent::new(8, 8)));
        assert!(!same_extent(Extent::new(8, 8), Extent::new(8, 9)));
    }
}
