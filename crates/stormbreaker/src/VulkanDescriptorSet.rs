//! Descriptor sets.
//!
//! Mirrors `VulkanDescriptorSet.cpp`.
//!
//! The C++ engine binds a **global** descriptor set on every `BindPipeline`
//! (`VulkanCommandBuffer.cpp:370-375`), which its own
//! `docs/framegraph-and-rendergraph.md:57` lists as a known problem. The fix is
//! structural: sets live here, are bound at an explicit set index by
//! `vulkan_command_buffer`, and there is no global. Nothing to port, because there
//! is nothing to carry over.

use ash::vk;

use crate::vulkan_error::{BackendError, Result};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum DescriptorKind {
    UniformBuffer,
    StorageBuffer,
    SampledImage,
    StorageImage,
    CombinedImageSampler,
    Sampler,
    InputAttachment,
}

impl DescriptorKind {
    pub fn descriptor_type(self) -> vk::DescriptorType {
        match self {
            DescriptorKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
            DescriptorKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
            DescriptorKind::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
            DescriptorKind::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
            DescriptorKind::CombinedImageSampler => {
                vk::DescriptorType::COMBINED_IMAGE_SAMPLER
            }
            DescriptorKind::Sampler => vk::DescriptorType::SAMPLER,
            DescriptorKind::InputAttachment => vk::DescriptorType::INPUT_ATTACHMENT,
        }
    }
}

impl From<DescriptorKind> for vk::DescriptorType {
    fn from(kind: DescriptorKind) -> Self {
        kind.descriptor_type()
    }
}

/// A live `VkDescriptorSet` with the layout it was allocated against.
#[derive(Clone, Copy, Debug)]
pub struct DescriptorSet {
    set: vk::DescriptorSet,
    layout: vk::DescriptorSetLayout,
}

impl DescriptorSet {
    pub fn new(set: vk::DescriptorSet, layout: vk::DescriptorSetLayout) -> Self {
        Self { set, layout }
    }

    pub fn handle(&self) -> vk::DescriptorSet {
        self.set
    }

    pub fn layout(&self) -> vk::DescriptorSetLayout {
        self.layout
    }
}

/// Applies writes to a set.
///
/// The writes borrow their descriptors, so this is a borrow-scoped call rather
/// than a stored value: the caller has to keep the buffers and images alive
/// until it returns, which is exactly the constraint Vulkan imposes.
pub fn update(
    device: &ash::Device,
    set: DescriptorSet,
    buffer_writes: &[vk::WriteDescriptorSet<'_>],
    image_writes: &[vk::WriteDescriptorSet<'_>],
) -> Result<()> {
    let mut writes: Vec<vk::WriteDescriptorSet<'_>> =
        Vec::with_capacity(buffer_writes.len() + image_writes.len());
    writes.extend_from_slice(buffer_writes);
    writes.extend_from_slice(image_writes);

    for write in &mut writes {
        write.dst_set = set.handle();
    }

    if writes.is_empty() {
        return Ok(());
    }

    unsafe { device.update_descriptor_sets(&writes, &[]) };
    Ok(())
}

/// Vulkan guarantees at least this many bindings per set.
pub const MAX_BINDINGS: u32 = 128;

pub fn check_binding_count(count: usize) -> Result<()> {
    if count as u32 > MAX_BINDINGS {
        return Err(BackendError::TooManyBindings { count });
    }
    Ok(())
}