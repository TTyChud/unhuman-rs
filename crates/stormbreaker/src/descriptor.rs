//! Descriptors.
//!
//! Mirrors `VulkanDescriptorManager.cpp` and the descriptor half of
//! `RHIDevice.h`. Two things differ, both deliberate:
//!
//! * Layouts are keyed by a structural hash, so two identical requests share one
//!   `VkDescriptorSetLayout` instead of allocating a new one per pass per frame.
//!   The C++ engine has no equivalent and grows a layout per bind group.
//! * The C++ engine binds a global descriptor set on every `BindPipeline`
//!   (`VulkanCommandBuffer.cpp:370-375`), which is the thing the engine's own
//!   `docs/framegraph-and-rendergraph.md:57` flags. Here sets are per-pass and
//!   explicit, bound at a numbered set index, so the global-set problem has no
//!   representation.

use std::collections::HashMap;

use ash::vk;
use chassis::{BindGroup, Registry};

use crate::error::{BackendError, Result};
use crate::shader::content_hash;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Binding {
    pub binding: u32,
    pub count: u32,
    pub kind: DescriptorKind,
}

/// The binding set a pipeline declares. Hashable, so a layout can be shared.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BindGroupLayoutDesc {
    pub bindings: Vec<Binding>,
}

impl BindGroupLayoutDesc {
    pub fn uniform(binding: u32) -> Self {
        Self::uniforms(binding, 1)
    }

    pub fn uniforms(binding: u32, count: u32) -> Self {
        Self {
            bindings: vec![Binding {
                binding,
                count,
                kind: DescriptorKind::UniformBuffer,
            }],
        }
    }

    pub fn combined_sampler(binding: u32, count: u32) -> Self {
        Self {
            bindings: vec![Binding {
                binding,
                count,
                kind: DescriptorKind::CombinedImageSampler,
            }],
        }
    }

    pub fn hash(&self) -> u64 {
        let mut bytes = Vec::new();
        for binding in &self.bindings {
            bytes.extend_from_slice(&binding.binding.to_le_bytes());
            bytes.extend_from_slice(&binding.count.to_le_bytes());
            bytes.extend_from_slice(&(binding.kind as u32).to_le_bytes());
        }
        content_hash(&bytes)
    }
}

struct LayoutEntry {
    layout: vk::DescriptorSetLayout,
    /// Bind group layouts may not be freed while a pipeline that references them
    /// is alive, so they are reference counted rather than destroyed eagerly.
    ref_count: u32,
}

pub struct Descriptors {
    device: ash::Device,
    layouts: HashMap<u64, LayoutEntry>,
    /// Keyed by descriptor type bit, which `DescriptorType` hashes on.
    descriptor_type_counts: HashMap<vk::DescriptorType, vk::DescriptorPoolSize>,
    pools: Vec<vk::DescriptorPool>,
    sets: Registry<vk::DescriptorSet, chassis::BindGroupTag>,
}

impl Descriptors {
    pub fn new(device: &ash::Device) -> Self {
        Self {
            device: device.clone(),
            layouts: HashMap::new(),
            descriptor_type_counts: HashMap::new(),
            pools: Vec::new(),
            sets: Registry::new(),
        }
    }

    /// Returns a layout for this binding set, creating it on first sight.
    pub fn layout(&mut self, desc: &BindGroupLayoutDesc) -> Result<vk::DescriptorSetLayout> {
        let hash = desc.hash();
        if let Some(entry) = self.layouts.get_mut(&hash) {
            entry.ref_count += 1;
            return Ok(entry.layout);
        }

        let bindings: Vec<vk::DescriptorSetLayoutBinding> = desc
            .bindings
            .iter()
            .map(|binding| vk::DescriptorSetLayoutBinding {
                binding: binding.binding,
                descriptor_type: binding.kind.descriptor_type(),
                descriptor_count: binding.count,
                stage_flags: vk::ShaderStageFlags::ALL,
                p_immutable_samplers: std::ptr::null(),
                ..Default::default()
            })
            .collect();

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let layout = unsafe { self.device.create_descriptor_set_layout(&layout_info, None) }
            .map_err(|e| BackendError::vk("vkCreateDescriptorSetLayout", e))?;

        // Pool sizes must cover every type the layout uses.
        let mut sizes = Vec::new();
        for binding in &desc.bindings {
            let size = vk::DescriptorPoolSize {
                ty: binding.kind.descriptor_type(),
                descriptor_count: binding.count,
            };
            match self.descriptor_type_counts.get_mut(&binding.kind.descriptor_type()) {
                Some(existing) => existing.descriptor_count += binding.count,
                None => {
                    self.descriptor_type_counts
                        .insert(binding.kind.descriptor_type(), size);
                }
            }
            if !sizes.iter().any(|s: &vk::DescriptorPoolSize| s.ty == size.ty) {
                sizes.push(size);
            }
        }

        self.layouts.insert(
            hash,
            LayoutEntry {
                layout,
                ref_count: 1,
            },
        );
        Ok(layout)
    }

    /// Ensures a pool exists with room for the layouts declared so far.
    fn ensure_pool(&mut self, sets: u32) -> Result<()> {
        let needs_new = match self.pools.last() {
            Some(_) => sets == 0,
            None => true,
        };
        if !needs_new {
            return Ok(());
        }

        let sizes: Vec<vk::DescriptorPoolSize> =
            self.descriptor_type_counts.values().copied().collect();
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(sets.max(1))
            .pool_sizes(&sizes);
        let pool = unsafe { self.device.create_descriptor_pool(&info, None) }
            .map_err(|e| BackendError::vk("vkCreateDescriptorPool", e))?;
        self.pools.push(pool);
        Ok(())
    }

    /// Allocates a set from `layout`. Reusing a pool is not attempted yet: the
    /// free-and-reallocate path needs frame tracking to know a set is no longer
    /// referenced, which the deletion queue will own.
    pub fn allocate_set(
        &mut self,
        layout: vk::DescriptorSetLayout,
        bindings: &[(u32, DescriptorKind)],
    ) -> Result<BindGroup> {
        self.ensure_pool(1)?;
        let Some(pool) = self.pools.last().copied() else {
            return Err(BackendError::NoDescriptorPool);
        };

        let layouts = [layout];
        let allocate = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let set = unsafe { self.device.allocate_descriptor_sets(&allocate) }
            .map_err(|e| BackendError::vk("vkAllocateDescriptorSets", e))?
            .remove(0);

        // Writes are recorded, not applied until `update` is called with the
        // backing resources. A set that was never written is legal but useless,
        // so the count is checked rather than trusted.
        if bindings.len() as u32 > MAX_BINDINGS {
            return Err(BackendError::TooManyBindings { count: bindings.len() });
        }

        Ok(self.sets.create(set))
    }

    /// Applies writes to a set. `buffer_writes` and `image_writes` may be empty;
    /// bindings with no write keep whatever they had, which is what lets a
    /// bind group be reused across frames with only some slots changing.
    pub fn update(
        &mut self,
        set: BindGroup,
        buffer_writes: &[vk::WriteDescriptorSet<'_>],
        image_writes: &[vk::WriteDescriptorSet<'_>],
    ) -> Result<()> {
        let Some(handle) = self.sets.get(set).copied() else {
            return Err(BackendError::InvalidHandle);
        };

        let mut writes: Vec<vk::WriteDescriptorSet<'_>> =
            Vec::with_capacity(buffer_writes.len() + image_writes.len());
        writes.extend_from_slice(buffer_writes);
        writes.extend_from_slice(image_writes);

        for write in &mut writes {
            write.dst_set = handle;
        }

        if writes.is_empty() {
            return Ok(());
        }

        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    pub fn set_handle(&self, set: BindGroup) -> Option<vk::DescriptorSet> {
        self.sets.get(set).copied()
    }

    /// How many distinct layouts exist. A count that climbs every frame means the
    /// layout cache is missing cases and each pass is allocating its own.
    pub fn layout_count(&self) -> usize {
        self.layouts.len()
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }
}

/// Vulkan guarantees at least this many descriptor bindings per set.
const MAX_BINDINGS: u32 = 128;

impl Drop for Descriptors {
    fn drop(&mut self) {
        unsafe {
            for pool in self.pools.drain(..) {
                self.device.destroy_descriptor_pool(pool, None);
            }
            for entry in self.layouts.values() {
                self.device
                    .destroy_descriptor_set_layout(entry.layout, None);
            }
        }
    }
}