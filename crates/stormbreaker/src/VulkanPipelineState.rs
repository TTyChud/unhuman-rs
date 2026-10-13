//! Pipeline description, hashing and the description cache.
//!
//! Mirrors `VulkanPipelineState.cpp`, which in the C++ tree holds both the state
//! objects and the creation path. Split here:
//!
//! ```text
//!   vulkan_pipeline_state.rs    this file: description, hash, cache, lifetime
//!   vulkan_graphic_pipeline.rs  VulkanGraphicPipeline.cpp
//!   vulkan_compute_pipeline.rs  VulkanComputePipeline.cpp
//!   vulkan_pipeline_cache.rs    VulkanPipelineCache.cpp, the driver's cache
//! ```
//!
//! Two caches exist and they solve different problems, which is why they are two
//! files:
//!
//! * **This one** asks *us* "is this the same description as last time?" — it
//!   avoids the `vkCreateGraphicsPipelines` call entirely.
//! * `vulkan_pipeline_cache.rs` asks the *driver* "have you already compiled
//!   this?" — it can still cost the call.
//!
//! The C++ tree has only the second kind, which is why
//! `VulkanPipelineState.cpp` recompiles on every request.

use std::collections::HashMap;

use ash::vk;
use chassis::{Pipeline, Registry};

use crate::vulkan_error::Result;
use crate::vulkan_shader::content_hash;

/// Hashes everything that affects the compiled pipeline.
///
/// Attribute offsets and formats change generated code, so they are in. The
/// label is not, so renaming a pass does not invalidate its pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineHash(u64);

impl PipelineHash {
    /// From raw bytes. Used by the compute path, whose description shares no
    /// fields with the graphics one.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(content_hash(bytes))
    }

    pub fn of(desc: &chassis::PipelineDesc) -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&desc.shader.index().to_le_bytes());
        bytes.extend_from_slice(&desc.vertex.stride.to_le_bytes());
        for attribute in &desc.vertex.attributes {
            bytes.extend_from_slice(&attribute.location.to_le_bytes());
            bytes.extend_from_slice(
                &attribute
                    .format
                    .bytes_per_texel()
                    .unwrap_or(0)
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(&attribute.offset.to_le_bytes());
        }
        bytes.extend_from_slice(
            &desc
                .index_format
                .map(|f| f as u32)
                .unwrap_or(0xff)
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&desc.primitive.topology.to_le_bytes());
        bytes.extend_from_slice(&desc.primitive.cull_mode.to_le_bytes());
        bytes.push(desc.depth.write_enabled as u8);
        bytes.push(desc.depth.format.is_some() as u8);
        bytes.push(desc.blend.enabled as u8);
        bytes.extend_from_slice(&(desc.color_targets.len() as u32).to_le_bytes());
        for target in &desc.color_targets {
            bytes.extend_from_slice(&(target.format as u32).to_le_bytes());
            bytes.push(target.blend as u8);
            bytes.push(target.write_mask);
        }
        Self(content_hash(&bytes))
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

pub struct PipelineResource {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    label: String,
}

impl PipelineResource {
    pub fn new(pipeline: vk::Pipeline, layout: vk::PipelineLayout, label: &str) -> Self {
        Self {
            pipeline,
            layout,
            label: label.to_string(),
        }
    }

    pub fn handle(&self) -> vk::Pipeline {
        self.pipeline
    }

    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Owns every pipeline and maps a description hash to the handle for it.
pub struct Pipelines {
    device: ash::Device,
    registry: Registry<PipelineResource, chassis::PipelineTag>,
    by_description: HashMap<PipelineHash, Pipeline>,
    driver_cache: vk::PipelineCache,
    created: u32,
    cache_hits: u32,
}

impl Pipelines {
    pub fn new(device: &ash::Device, driver_cache: vk::PipelineCache) -> Self {
        Self {
            device: device.clone(),
            registry: Registry::new(),
            by_description: HashMap::new(),
            driver_cache,
            created: 0,
            cache_hits: 0,
        }
    }

    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    pub fn driver_cache(&self) -> vk::PipelineCache {
        self.driver_cache
    }

    pub fn by_description(&self, hash: PipelineHash) -> Option<Pipeline> {
        self.by_description.get(&hash).copied()
    }

    pub fn mark_cache_hit(&mut self, handle: Pipeline) {
        self.cache_hits += 1;
        let _ = handle;
    }

    /// Records a newly compiled pipeline against the description hash that
    /// missed.
    pub fn insert(&mut self, hash: PipelineHash, resource: PipelineResource) -> Pipeline {
        let handle = self.registry.create(resource);
        self.by_description.insert(hash, handle);
        self.created += 1;
        handle
    }

    /// A pipeline layout with no descriptor sets and no push constants.
    ///
    /// Real layouts come once `bifrost` declares its bind groups; an empty one is
    /// correct for the current state and fails loudly at draw time rather than
    /// silently binding nothing.
    ///
    /// The description is not read yet. It is taken so the call sites read the
    /// same way for graphics and compute, and so adding set layouts later does not
    /// change either signature.
    pub fn create_layout(&mut self, _desc: &() ) -> Result<vk::PipelineLayout> {
        let set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();
        let info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&[]);
        unsafe { self.device.create_pipeline_layout(&info, None) }
            .map_err(|e| crate::vulkan_error::BackendError::vk("vkCreatePipelineLayout", e))
    }

    pub fn get(&self, handle: Pipeline) -> Option<&PipelineResource> {
        self.registry.get(handle)
    }

    /// Pipelines actually compiled, versus requests served from the description
    /// cache. `hits` at zero while a frame asks for the same pipeline repeatedly
    /// means the hash is missing a field.
    pub fn stats(&self) -> (u32, u32) {
        (self.created, self.cache_hits)
    }

    pub fn len(&self) -> usize {
        self.registry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}

impl Drop for Pipelines {
    fn drop(&mut self) {
        unsafe {
            for (_, resource) in self.registry.iter() {
                self.device.destroy_pipeline(resource.pipeline, None);
                self.device
                    .destroy_pipeline_layout(resource.layout, None);
            }
        }
    }
}