//! Descriptor management.
//!
//! Mirrors `VulkanDescriptorManager.cpp`. Owns the layouts, the pools and the
//! sets, and is the only entry point the rest of the backend uses:
//!
//! ```text
//!   vulkan_descriptor_set.rs    DescriptorKind, DescriptorSet, writes
//!   vulkan_descriptor_pool.rs   pool sizes and allocation
//!   this file                   caching, lifetime, and the mapping to chassis
//! ```
//!
//! Two things the C++ version does not have:
//!
//! * **Layouts are cached by a structural hash**, so two identical requests share
//!   one `VkDescriptorSetLayout`. The C++ allocates a layout per bind group, and
//!   `VulkanDescriptorManager.cpp:90,94,113` is reached through the
//!   `g_VulkanContext` global besides.
//! * **Sets are per-pass and bound at an index.** The C++ binds a global set on
//!   every `BindPipeline` (`VulkanCommandBuffer.cpp:370-375`), which its own
//!   `docs/framegraph-and-rendergraph.md:57` flags. Here there is no global to
//!   bind, so the failure mode has no representation.

use std::collections::HashMap;

use ash::vk;
use chassis::{BindGroup, Registry};

use crate::vulkan_descriptor_pool::{DescriptorPool, PoolSizes};
use crate::vulkan_descriptor_set::{self, DescriptorKind, DescriptorSet};
use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_shader::content_hash;

pub use crate::vulkan_descriptor_set::DescriptorKind as Kind;

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
    /// Layouts may not be freed while a pipeline referencing them is alive, so
    /// they are counted rather than destroyed on last use.
    ref_count: u32,
}

pub struct Descriptors {
    device: ash::Device,
    layouts: HashMap<u64, LayoutEntry>,
    pool_sizes: PoolSizes,
    pool: Option<DescriptorPool>,
    sets: Registry<DescriptorSet, chassis::BindGroupTag>,
    layout_hits: u32,
    layout_misses: u32,
}

impl Descriptors {
    pub fn new(device: &ash::Device) -> Self {
        Self {
            device: device.clone(),
            layouts: HashMap::new(),
            pool_sizes: PoolSizes::default(),
            pool: None,
            sets: Registry::new(),
            layout_hits: 0,
            layout_misses: 0,
        }
    }

    /// A layout for this binding set, created on first sight.
    pub fn layout(&mut self, desc: &BindGroupLayoutDesc) -> Result<vk::DescriptorSetLayout> {
        let hash = desc.hash();
        if let Some(entry) = self.layouts.get_mut(&hash) {
            entry.ref_count += 1;
            self.layout_hits += 1;
            return Ok(entry.layout);
        }
        self.layout_misses += 1;

        let bindings: Vec<vk::DescriptorSetLayoutBinding> = desc
            .bindings
            .iter()
            .map(|binding| vk::DescriptorSetLayoutBinding {
                binding: binding.binding,
                descriptor_type: vulkan_descriptor_set::descriptor_type_of(binding.kind),
                descriptor_count: binding.count,
                stage_flags: vk::ShaderStageFlags::ALL,
                p_immutable_samplers: std::ptr::null(),
                ..Default::default()
            })
            .collect();

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let layout = unsafe { self.device.create_descriptor_set_layout(&layout_info, None) }
            .map_err(|e| BackendError::vk("vkCreateDescriptorSetLayout", e))?;

        self.pool_sizes.account_for(&desc.bindings);
        self.pool = None; // sizes changed; rebuild on next allocation

        self.layouts.insert(
            hash,
            LayoutEntry {
                layout,
                ref_count: 1,
            },
        );
        Ok(layout)
    }

    /// Allocates a set. Rebuilds the pool when the accumulated sizes changed or
    /// the current one is full.
    fn pool_mut(&mut self) -> Result<&mut DescriptorPool> {
        let needs_new = match &self.pool {
            None => true,
            Some(pool) => !pool.has_room(),
        };
        if needs_new {
            // Dropping the old pool frees its sets, which is why the registry
            // below is cleared with it.
            self.pool = Some(DescriptorPool::create(
                &self.device,
                &self.pool_sizes.as_slice(),
                64,
            )?);
            self.sets = Registry::new();
        }
        self.pool.as_mut().ok_or(BackendError::NoDescriptorPool)
    }

    /// Allocates a set from `layout`.
    pub fn allocate(&mut self, layout: vk::DescriptorSetLayout) -> Result<BindGroup> {
        let pool = self.pool_mut()?;
        let set = pool.allocate(layout)?;
        Ok(self.sets.create(DescriptorSet::new(set, layout)))
    }

    /// Applies writes to an allocated set.
    pub fn update(
        &mut self,
        bind_group: BindGroup,
        buffer_writes: &[vk::WriteDescriptorSet<'_>],
        image_writes: &[vk::WriteDescriptorSet<'_>],
    ) -> Result<()> {
        let Some(set) = self.sets.get(bind_group).copied() else {
            return Err(BackendError::InvalidHandle);
        };
        vulkan_descriptor_set::update(&self.device, set, buffer_writes, image_writes)
    }

    pub fn set(&self, bind_group: BindGroup) -> Option<DescriptorSet> {
        self.sets.get(bind_group).copied()
    }

    /// Distinct layouts created. A count that climbs every frame means the hash
    /// is missing a field and each pass is allocating its own layout.
    pub fn layout_count(&self) -> usize {
        self.layouts.len()
    }

    /// Layout requests served from cache versus created. `hits` staying at zero
    /// across frames means the cache is not being consulted.
    pub fn layout_stats(&self) -> (u32, u32) {
        (self.layout_hits, self.layout_misses)
    }

    pub fn pool(&self) -> Option<vk::DescriptorPool> {
        self.pool.as_ref().map(DescriptorPool::handle)
    }

    pub fn set_count(&self) -> usize {
        self.sets.len()
    }
}

impl Drop for Descriptors {
    fn drop(&mut self) {
        // Pool first: it owns the sets, and freeing layouts while a pool
        // references them is what the destruction order rules forbid.
        self.pool = None;
        unsafe {
            for entry in self.layouts.values() {
                self.device
                    .destroy_descriptor_set_layout(entry.layout, None);
            }
        }
    }
}
