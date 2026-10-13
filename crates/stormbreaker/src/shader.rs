//! Shader modules.
//!
//! Mirrors `VulkanShader.cpp`. Modules are keyed by a content hash, so a shader
//! whose source did not change is never recompiled or re-uploaded — an addition
//! the C++ tree wanted and did not have. `forge` produces the SPIR-V and the
//! hash; this file never parses shader source.
//!
//! The driver's own cache is `pipeline_cache.rs`, and the "have we compiled this
//! description before" cache is in `pipeline.rs`. Three layers, three files.

use std::collections::HashMap;

use ash::vk;
use chassis::{Registry, Shader};

use crate::error::{BackendError, Result};

/// Reinterprets SPIR-V bytes as the `u32` words Vulkan wants, rejecting anything
/// that is not a module.
///
/// Every SPIR-V module starts with the magic word `0x07230203`. A buffer without
/// it is not SPIR-V, and letting that reach the driver produces a much worse
/// error message than refusing it here.
fn spirv_to_words(spirv: &[u8]) -> Result<&[u32]> {
    if spirv.len() < 4 || !spirv.len().is_multiple_of(4) {
        return Err(BackendError::MalformedSpirv { len: spirv.len() });
    }
    if u32::from_le_bytes([spirv[0], spirv[1], spirv[2], spirv[3]]) != 0x0723_0203 {
        return Err(BackendError::MalformedSpirv { len: spirv.len() });
    }
    // A byte buffer that is 4-byte aligned and a multiple of 4 long can be read
    // as u32 on every target Vulkan runs on.
    let (prefix, words, suffix) =
        unsafe { spirv.align_to::<u32>() };
    if !prefix.is_empty() || !suffix.is_empty() {
        return Err(BackendError::MalformedSpirv { len: spirv.len() });
    }
    Ok(words)
}

/// Cheap content hash (FNV-1a). Not cryptographic: it only has to notice that
/// the bytes changed.
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub struct ShaderResource {
    module: vk::ShaderModule,
    hash: u64,
    stage: vk::ShaderStageFlags,
}

impl ShaderResource {
    pub fn module(&self) -> vk::ShaderModule {
        self.module
    }

    pub fn hash(&self) -> u64 {
        self.hash
    }

    pub fn stage(&self) -> vk::ShaderStageFlags {
        self.stage
    }
}

/// Owns shader modules, deduplicated by content hash.
pub struct Shaders {
    registry: Registry<ShaderResource, chassis::ShaderTag>,
    by_hash: HashMap<(u64, vk::ShaderStageFlags), Shader>,
    compiled: u32,
    reused: u32,
}

impl Shaders {
    pub fn new() -> Self {
        Self {
            registry: Registry::new(),
            by_hash: HashMap::new(),
            compiled: 0,
            reused: 0,
        }
    }

    /// The cached module for this SPIR-V and stage, if one was loaded.
    ///
    /// Split out from `create` so the dedup rule is testable without a device,
    /// which is the part most likely to be wrong.
    pub fn lookup(&self, spirv: &[u8], stage: vk::ShaderStageFlags) -> Option<Shader> {
        self.by_hash.get(&(content_hash(spirv), stage)).copied()
    }

    /// Creates a module, or returns the existing one when the same SPIR-V for
    /// the same stage was already loaded this run.
    pub fn create(
        &mut self,
        device: &ash::Device,
        spirv: &[u8],
        stage: vk::ShaderStageFlags,
    ) -> Result<Shader> {
        let words = spirv_to_words(spirv)?;
        let hash = content_hash(spirv);
        if let Some(existing) = self.by_hash.get(&(hash, stage)) {
            self.reused += 1;
            return Ok(*existing);
        }

        let create_info = vk::ShaderModuleCreateInfo::default().code(words);
        let module = unsafe { device.create_shader_module(&create_info, None) }
            .map_err(|e| BackendError::vk("vkCreateShaderModule", e))?;

        let handle = self.registry.create(ShaderResource {
            module,
            hash,
            stage,
        });
        self.by_hash.insert((hash, stage), handle);
        self.compiled += 1;
        Ok(handle)
    }

    pub fn get(&self, handle: Shader) -> Option<&ShaderResource> {
        self.registry.get(handle)
    }

    pub fn destroy(&mut self, device: &ash::Device, handle: Shader) -> bool {
        let Some(resource) = self.registry.get(handle) else {
            return false;
        };
        self.by_hash.remove(&(resource.hash, resource.stage));
        if let Some(resource) = self.registry.remove(handle) {
            unsafe { device.destroy_shader_module(resource.module, None) };
            true
        } else {
            false
        }
    }

    /// Modules actually created this run, versus how many requests were served
    /// from the content-hash cache. A `reused` that never grows means the cache
    /// is not being consulted.
    pub fn stats(&self) -> (u32, u32) {
        (self.compiled, self.reused)
    }

    pub fn len(&self) -> usize {
        self.registry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}

impl Default for Shaders {
    fn default() -> Self {
        Self::new()
    }
}
