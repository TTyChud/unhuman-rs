//! Compute pipelines.
//!
//! Mirrors `VulkanComputePipeline.cpp`. Separate from graphics because the two
//! share only the layout and the description cache — everything else differs:
//! no vertex input, no rasterisation, no blending, and a different create call.

use ash::vk;
use chassis::Pipeline;

use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_pipeline_state::{PipelineHash, PipelineResource, Pipelines};
use crate::vulkan_shader::Shaders;

/// What a compute pass declares. No vertex layout, no attachments: a compute
/// pipeline has none of either.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ComputePipelineDesc {
    pub label: String,
    pub shader: chassis::Shader,
}

impl ComputePipelineDesc {
    pub fn new(label: impl Into<String>, shader: chassis::Shader) -> Self {
        Self {
            label: label.into(),
            shader,
        }
    }
}

pub fn create(
    pipelines: &mut Pipelines,
    desc: &ComputePipelineDesc,
    shaders: &Shaders,
) -> Result<Pipeline> {
    let hash = hash_of(desc);
    if let Some(existing) = pipelines.by_description(hash) {
        pipelines.mark_cache_hit(existing);
        return Ok(existing);
    }

    let layout = pipelines.create_layout(&())?;
    let handle = build(pipelines, desc, shaders, layout)?;
    Ok(pipelines.insert(hash, PipelineResource::new(handle, layout, &desc.label)))
}

/// Hashes the compute description. Kept separate from `PipelineHash` because the
/// two spaces share no fields, and mixing them would let a graphics and a compute
/// pipeline collide.
fn hash_of(desc: &ComputePipelineDesc) -> PipelineHash {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"compute");
    bytes.extend_from_slice(&desc.shader.index().to_le_bytes());
    bytes.extend_from_slice(&desc.shader.generation().to_le_bytes());
    PipelineHash::from_bytes(&bytes)
}

fn build(
    pipelines: &Pipelines,
    desc: &ComputePipelineDesc,
    shaders: &Shaders,
    layout: vk::PipelineLayout,
) -> Result<vk::Pipeline> {
    let Some(shader) = shaders.get(desc.shader) else {
        return Err(BackendError::InvalidHandle);
    };

    let entry_name = c"main";
    let stage = vk::PipelineShaderStageCreateInfo {
        stage: vk::ShaderStageFlags::COMPUTE,
        module: shader.module(),
        p_name: entry_name.as_ptr(),
        ..Default::default()
    };

    let create_info = vk::ComputePipelineCreateInfo {
        stage,
        layout,
        ..Default::default()
    };

    let created = unsafe {
        pipelines.device().create_compute_pipelines(
            pipelines.driver_cache(),
            &[create_info],
            None,
        )
    }
    .map_err(|(partial, error)| {
        BackendError::vk("vkCreateComputePipelines", error).with_partial(partial.len())
    })?;

    let mut created = created;
    Ok(created.remove(0))
}
