//! Descriptor pools.
//!
//! Mirrors `VulkanDescriptorPool.cpp`, one of the three files the C++ backend
//! splits its descriptor handling across.
//!
//! A pool is the allocation source for sets; `vulkan_descriptor_set.rs` allocates
//! from one. Sizes are derived from the layouts that have been requested so far,
//! because a pool whose sizes do not cover the types in its sets fails
//! `vkAllocateDescriptorSets` with `ERROR_OUT_OF_POOL_MEMORY` and no indication of
//! which type was short.

use std::collections::HashMap;

use ash::vk;

use crate::vulkan_error::{BackendError, Result};

/// Accumulated pool sizes, keyed by descriptor type.
#[derive(Debug, Default)]
pub struct PoolSizes {
    sizes: HashMap<vk::DescriptorType, vk::DescriptorPoolSize>,
}

impl PoolSizes {
    /// Adds a layout's worth of descriptors. Repeated calls for the same type
    /// accumulate rather than overwrite, so declaring two bind groups of the same
    /// shape does not under-count the second.
    pub fn account_for(&mut self, bindings: &[crate::vulkan_descriptor_manager::Binding]) {
        for binding in bindings {
            let size = vk::DescriptorPoolSize {
                ty: binding.kind.descriptor_type(),
                descriptor_count: binding.count,
            };
            match self.sizes.get_mut(&binding.kind.descriptor_type()) {
                Some(existing) => existing.descriptor_count += binding.count,
                None => {
                    self.sizes.insert(binding.kind.descriptor_type(), size);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }

    pub fn as_slice(&self) -> Vec<vk::DescriptorPoolSize> {
        self.sizes.values().copied().collect()
    }
}

/// One `VkDescriptorPool`.
pub struct DescriptorPool {
    device: ash::Device,
    pool: vk::DescriptorPool,
    max_sets: u32,
    allocated: u32,
}

impl DescriptorPool {
    pub fn create(
        device: &ash::Device,
        sizes: &[vk::DescriptorPoolSize],
        max_sets: u32,
    ) -> Result<Self> {
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(max_sets.max(1))
            .pool_sizes(sizes);
        let pool = unsafe { device.create_descriptor_pool(&info, None) }
            .map_err(|e| BackendError::vk("vkCreateDescriptorPool", e))?;
        Ok(Self {
            device: device.clone(),
            pool,
            max_sets: max_sets.max(1),
            allocated: 0,
        })
    }

    pub fn handle(&self) -> vk::DescriptorPool {
        self.pool
    }

    pub fn max_sets(&self) -> u32 {
        self.max_sets
    }

    pub fn allocated(&self) -> u32 {
        self.allocated
    }

    pub fn has_room(&self) -> bool {
        self.allocated < self.max_sets
    }

    /// Allocates one set against `layout`.
    pub fn allocate(&mut self, layout: vk::DescriptorSetLayout) -> Result<vk::DescriptorSet> {
        if !self.has_room() {
            return Err(BackendError::NoDescriptorPool);
        }
        let layouts = [layout];
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.pool)
            .set_layouts(&layouts);
        let mut sets = unsafe { self.device.allocate_descriptor_sets(&info) }
            .map_err(|e| BackendError::vk("vkAllocateDescriptorSets", e))?;
        self.allocated += 1;
        Ok(sets.remove(0))
    }

    /// Frees sets and resets the pool for reuse. Only safe when nothing
    /// recorded still references them, which is the frame-retirement job.
    pub fn reset(&mut self) {
        unsafe {
            // A reset that fails means the device is lost, and there is nothing
            // useful to do with a half-reset pool.
            let _ = self.device.reset_descriptor_pool(
                self.pool,
                vk::DescriptorPoolResetFlags::empty(),
            );
        }
        self.allocated = 0;
    }
}

impl Drop for DescriptorPool {
    fn drop(&mut self) {
        unsafe { self.device.destroy_descriptor_pool(self.pool, None) };
    }
}