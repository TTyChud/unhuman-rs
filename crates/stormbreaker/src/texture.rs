//! Textures and image views.
//!
//! Mirrors `VulkanTexture.cpp` and `VulkanImageView.cpp`. Note what is *not*
//! here: a layout field per texture. The C++ engine tracks layouts in the
//! render graph and separately in `ImageState`, which is why
//! `VulkanCommandBuffer.cpp:104,153` can disagree with the graph about what a
//! texture's layout is. Here the graph is the only authority — the texture knows
//! its format and extent, and transitions arrive as barriers it never has to
//! reason about.

use ash::vk;
use chassis::{Extent, Registry, Texture, TextureDesc, TextureFormat, TextureUsage};

use crate::error::{BackendError, Result};
use crate::memory;
use crate::utils;

pub struct TextureResource {
    image: vk::Image,
    device_memory: vk::DeviceMemory,
    view: vk::ImageView,
    format: TextureFormat,
    extent: Extent,
    mip_levels: u32,
    label: String,
}

impl TextureResource {
    pub fn image(&self) -> vk::Image {
        self.image
    }

    pub fn view(&self) -> vk::ImageView {
        self.view
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    pub fn mip_levels(&self) -> u32 {
        self.mip_levels
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// The view plus the subresource range every barrier for this texture uses.
    pub fn barrier_target(&self) -> crate::barrier_encoder::Target {
        crate::barrier_encoder::Target::color(self.image, self.mip_levels, 1)
    }

    pub fn destroy(&self, device: &ash::Device) {
        unsafe {
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.device_memory, None);
        }
    }
}

pub struct Textures {
    device: ash::Device,
    registry: Registry<TextureResource, chassis::TextureTag>,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
}

impl Textures {
    pub fn new(
        device: &ash::Device,
        memory_properties: vk::PhysicalDeviceMemoryProperties,
    ) -> Self {
        Self {
            device: device.clone(),
            registry: Registry::new(),
            memory_properties,
        }
    }

    pub fn create(
        &mut self,
        device: &ash::Device,
        desc: &TextureDesc,
        label: &str,
    ) -> Result<Texture> {
        if desc.format == TextureFormat::Undefined {
            return Err(BackendError::EmptyUsage { what: "texture format" });
        }
        let usage = to_vk_usage(desc.usage);
        if usage.is_empty() {
            return Err(BackendError::EmptyUsage { what: "texture usage" });
        }
        let level_count = desc.mip_levels.max(1);

        let image_info = vk::ImageCreateInfo {
            image_type: if desc.extent.depth > 1 {
                vk::ImageType::TYPE_3D
            } else {
                vk::ImageType::TYPE_2D
            },
            format: utils::vk_format(desc.format),
            extent: vk::Extent3D {
                width: desc.extent.width,
                height: desc.extent.height,
                depth: desc.extent.depth.max(1),
            },
            mip_levels: level_count,
            array_layers: 1,
            samples: utils::vk_samples(desc.samples),
            tiling: vk::ImageTiling::OPTIMAL,
            usage,
            sharing_mode: vk::SharingMode::EXCLUSIVE,
            initial_layout: vk::ImageLayout::UNDEFINED,
            ..Default::default()
        };

        let image = match unsafe { device.create_image(&image_info, None) } {
            Ok(image) => image,
            Err(e) => return Err(BackendError::vk("vkCreateImage", e)),
        };

        let requirements = unsafe { device.get_image_memory_requirements(image) };
        let device_memory = match memory::create_image_memory(
            device,
            &self.memory_properties,
            requirements.memory_type_bits,
            requirements.size,
        ) {
            Ok(m) => m,
            Err(e) => {
                unsafe { device.destroy_image(image, None) };
                return Err(e);
            }
        };

        if let Err(e) = unsafe { device.bind_image_memory(image, device_memory, 0) } {
            unsafe {
                device.free_memory(device_memory, None);
                device.destroy_image(image, None);
            }
            return Err(BackendError::vk("vkBindImageMemory", e));
        }

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            // UNDEFINED is the only legal layout for a view created alongside
            // the image; the graph transitions it later.
            .format(utils::vk_format(desc.format))
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: utils::aspect(desc.format),
                base_mip_level: 0,
                level_count,
                base_array_layer: 0,
                layer_count: 1,
            });

        let view = match unsafe { device.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(e) => {
                unsafe {
                    device.free_memory(device_memory, None);
                    device.destroy_image(image, None);
                }
                return Err(BackendError::vk("vkCreateImageView", e));
            }
        };

        Ok(self.registry.create(TextureResource {
            image,
            device_memory,
            view,
            format: desc.format,
            extent: desc.extent,
            mip_levels: level_count,
            label: label.to_string(),
        }))
    }

    pub fn get(&self, handle: Texture) -> Option<&TextureResource> {
        self.registry.get(handle)
    }

    pub fn destroy(&mut self, device: &ash::Device, handle: Texture) -> bool {
        match self.registry.remove(handle) {
            Some(resource) => {
                resource.destroy(device);
                true
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.registry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (Texture, &TextureResource)> {
        self.registry.iter()
    }
}

impl Drop for Textures {
    fn drop(&mut self) {
        for (_, resource) in self.registry.iter() {
            resource.destroy(&self.device);
        }
    }
}

pub fn to_vk_usage(usage: TextureUsage) -> vk::ImageUsageFlags {
    let mut out = vk::ImageUsageFlags::empty();
    if usage.contains(TextureUsage::SAMPLED) {
        out |= vk::ImageUsageFlags::SAMPLED;
    }
    if usage.contains(TextureUsage::STORAGE) {
        out |= vk::ImageUsageFlags::STORAGE;
    }
    if usage.contains(TextureUsage::COLOR_ATTACHMENT) {
        out |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
    }
    if usage.contains(TextureUsage::DEPTH_ATTACHMENT) {
        out |= vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT;
    }
    if usage.contains(TextureUsage::TRANSFER_SRC) {
        out |= vk::ImageUsageFlags::TRANSFER_SRC;
    }
    if usage.contains(TextureUsage::TRANSFER_DST) {
        out |= vk::ImageUsageFlags::TRANSFER_DST;
    }
    out
}