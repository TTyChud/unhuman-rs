//! The driver's pipeline cache.
//!
//! Mirrors `VulkanPipelineCache.cpp`. Separate from `Pipelines`, which decides
//! *whether* to compile something, and from `shader.rs`, which owns SPIR-V
//! modules. Three layers, three files, matching the C++ split.
//!
//! Two caches exist and they solve different problems:
//!
//! * This one asks the driver "have you already compiled this?" — it can still
//!   cost a `vkCreateGraphicsPipelines` call.
//! * `Pipelines` asks *us* "is this the same description as last time?" — it
//!   avoids the call entirely.
//!
//! The C++ tree has the first kind only, which is why `VulkanPipelineState.cpp`
//! recompiles on every request.

use ash::vk;

use crate::vulkan_error::{BackendError, Result};

pub struct PipelineCache {
    device: ash::Device,
    cache: vk::PipelineCache,
    started_warm: bool,
}

impl PipelineCache {
    /// `initial_data` comes from a previous run's `data()`. Passing garbage is
    /// harmless: the driver checks its own header and ignores what it cannot read,
    /// so a bad blob costs a cold start and not a failed startup.
    pub fn new(device: &ash::Device, initial_data: Option<&[u8]>) -> Result<Self> {
        let info = vk::PipelineCacheCreateInfo::default().initial_data(initial_data.unwrap_or(&[]));
        let (cache, started_warm) = match unsafe { device.create_pipeline_cache(&info, None) } {
            Ok(cache) => (cache, initial_data.is_some_and(|d| !d.is_empty())),
            Err(_) => {
                let empty = vk::PipelineCacheCreateInfo::default();
                let cache = unsafe { device.create_pipeline_cache(&empty, None) }
                    .map_err(|e| BackendError::vk("vkCreatePipelineCache", e))?;
                (cache, false)
            }
        };
        Ok(Self {
            device: device.clone(),
            cache,
            started_warm,
        })
    }

    pub fn handle(&self) -> vk::PipelineCache {
        self.cache
    }

    /// False when the provided blob was rejected or absent, and the run started
    /// cold.
    pub fn started_warm(&self) -> bool {
        self.started_warm
    }

    /// Serialisable contents, in the layout the driver expects. Persist these
    /// between runs and the next one starts warm.
    pub fn data(&self) -> Result<Vec<u8>> {
        unsafe { self.device.get_pipeline_cache_data(self.cache) }
            .map_err(|e| BackendError::vk("vkGetPipelineCacheData", e))
    }

    /// Folds another cache in, e.g. one built during a shader-warmup pass.
    pub fn merge(&self, other: vk::PipelineCache) -> Result<()> {
        unsafe { self.device.merge_pipeline_caches(self.cache, &[other]) }
            .map_err(|e| BackendError::vk("vkMergePipelineCaches", e))
    }
}

impl Drop for PipelineCache {
    fn drop(&mut self) {
        unsafe { self.device.destroy_pipeline_cache(self.cache, None) };
    }
}
