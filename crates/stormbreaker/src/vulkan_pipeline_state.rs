//! Graphics and compute pipelines.
//!
//! Mirrors `VulkanPipelineState.cpp`. Two structural notes:
//!
//! * Pipelines are created through **dynamic rendering**
//!   (`VkPipelineRenderingCreateInfo`) rather than `VkRenderPass`. The C++ engine
//!   requires dynamic rendering as a hard floor (`VulkanDevice.cpp:87-91`)
//!   while still carrying `VkRenderPass`-shaped attachments around
//!   (`RHITypes.h:373-384`), so the render-pass objects are dead weight. Here
//!   `chassis::RenderPassDesc` *is* the rendering info.
//! * Pipelines are cached by a structural hash of the description, so a
//!   per-frame pipeline request for an unchanged pipeline compiles once. The C++
//!   tree has a `VulkanPipelineCache` for driver-level caching but no map from
//!   description to pipeline, which is a different problem: driver cache hits
//!   still cost a `vkCreateGraphicsPipelines` call.

use std::collections::HashMap;

use ash::vk;
use chassis::{Pipeline, PipelineDesc, Registry};

use crate::vulkan_descriptor_manager::Descriptors;
use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_extension_check::Capabilities;
use crate::vulkan_shader::{content_hash, Shaders};

pub struct PipelineResource {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    label: String,
    /// True when the description was served from the description cache rather
    /// than compiled.
    from_cache: bool,
}

impl PipelineResource {
    pub fn handle(&self) -> vk::Pipeline {
        self.pipeline
    }

    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn from_cache(&self) -> bool {
        self.from_cache
    }
}

pub struct Pipelines {
    device: ash::Device,
    registry: Registry<PipelineResource, chassis::PipelineTag>,
    /// Structural description hash -> pipeline handle.
    by_description: HashMap<u64, Pipeline>,
    /// Driver-side cache, shared by every create call.
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

    /// Hashes everything that affects the compiled pipeline. Attribute offsets
    /// and vertex formats change the compiled code, so they are in; the label is
    /// not, so renaming a pass does not invalidate it.
    fn hash(desc: &PipelineDesc) -> u64 {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&desc.shader.index().to_le_bytes());
        bytes.extend_from_slice(&desc.vertex.stride.to_le_bytes());
        for attribute in &desc.vertex.attributes {
            bytes.extend_from_slice(&attribute.location.to_le_bytes());
            bytes.extend_from_slice(&attribute.format.bytes_per_texel().unwrap_or(0).to_le_bytes());
            bytes.extend_from_slice(&attribute.offset.to_le_bytes());
        }
        bytes.extend_from_slice(&desc.index_format.map(|f| f as u32).unwrap_or(0xff).to_le_bytes());
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
        content_hash(&bytes)
    }

    pub fn create(
        &mut self,
        desc: &PipelineDesc,
        shaders: &Shaders,
        _descriptors: &mut Descriptors,
        capabilities: &Capabilities,
    ) -> Result<Pipeline> {
        let hash = Self::hash(desc);
        if let Some(existing) = self.by_description.get(&hash).copied() {
            self.cache_hits += 1;
            // Mark the shared entry as served from cache so callers can see it.
            if let Some(resource) = self.registry.get_mut(existing) {
                resource.from_cache = true;
            }
            return Ok(existing);
        }

        if !capabilities.dynamic_rendering {
            return Err(BackendError::Unsupported {
                what: "dynamic rendering".into(),
            });
        }

        let pipeline_layout = self.create_layout()?;
        let handle = self.create_graphics(desc, shaders, pipeline_layout)?;

        let resource = PipelineResource {
            pipeline: handle,
            layout: pipeline_layout,
            label: desc.label.clone(),
            from_cache: false,
        };
        let pipeline = self.registry.create(resource);
        self.by_description.insert(hash, pipeline);
        self.created += 1;
        Ok(pipeline)
    }

    fn create_layout(&mut self) -> Result<vk::PipelineLayout> {
        // One set per stage group: push constants sit at set 0 and are not a
        // descriptor set at all, so layout sets start at 0 here.
        let set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();
        let info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&[]);

        unsafe { self.device.create_pipeline_layout(&info, None) }
            .map_err(|e| BackendError::vk("vkCreatePipelineLayout", e))
    }

    fn create_graphics(
        &mut self,
        desc: &PipelineDesc,
        shaders: &Shaders,
        layout: vk::PipelineLayout,
    ) -> Result<vk::Pipeline> {
        let Some(shader) = shaders.get(desc.shader) else {
            return Err(BackendError::InvalidHandle);
        };

        let entry_name = c"main";
        let entry = vk::PipelineShaderStageCreateInfo {
            stage: vk::ShaderStageFlags::VERTEX,
            module: shader.module(),
            p_name: entry_name.as_ptr(),
            ..Default::default()
        };

        let attributes: Vec<vk::VertexInputAttributeDescription> = desc
            .vertex
            .attributes
            .iter()
            .map(|attribute| vk::VertexInputAttributeDescription {
                location: attribute.location,
                binding: 0,
                format: crate::vulkan_types::vk_format(attribute.format),
                offset: attribute.offset,
            })
            .collect();

        let bindings = if attributes.is_empty() {
            Vec::new()
        } else {
            vec![vk::VertexInputBindingDescription {
                binding: 0,
                stride: desc.vertex.stride,
                input_rate: vk::VertexInputRate::VERTEX,
            }]
        };

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(topology_of(desc.primitive.topology))
            .primitive_restart_enable(false);

        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width: 0, height: 0 },
        };
        // The arrays must outlive the create call, which reads them through the
        // state structs.
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);

        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(cull_mode_of(desc.primitive.cull_mode))
            .front_face(front_face_of(desc.primitive.front_face))
            .depth_bias_enable(false)
            .line_width(1.0);

        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(crate::vulkan_types::vk_samples(desc.samples))
            .sample_shading_enable(false);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(desc.depth.format.is_some())
            .depth_write_enable(desc.depth.write_enabled)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);

        let mut color_blend_attachments: Vec<vk::PipelineColorBlendAttachmentState> = Vec::new();
        for target in &desc.color_targets {
            // Standard alpha blend when asked for, otherwise a plain write.
            let blending = target.blend;
            color_blend_attachments.push(vk::PipelineColorBlendAttachmentState {
                blend_enable: if blending { vk::TRUE } else { vk::FALSE },
                src_color_blend_factor: if blending {
                    vk::BlendFactor::SRC_ALPHA
                } else {
                    vk::BlendFactor::ONE
                },
                dst_color_blend_factor: if blending {
                    vk::BlendFactor::ONE_MINUS_SRC_ALPHA
                } else {
                    vk::BlendFactor::ZERO
                },
                color_blend_op: vk::BlendOp::ADD,
                src_alpha_blend_factor: vk::BlendFactor::ONE,
                dst_alpha_blend_factor: vk::BlendFactor::ZERO,
                alpha_blend_op: vk::BlendOp::ADD,
                color_write_mask: vk::ColorComponentFlags::from_raw(target.write_mask as u32),
            });
        }

        let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&color_blend_attachments);

        // Only the vertex stage is built here. A fragment stage belongs on the
        // descriptor, and a compute pipeline is a different object entirely --
        // `compute_pipeline.rs`.
        let stages = [entry];

        let color_formats: Vec<vk::Format> = desc
            .color_targets
            .iter()
            .map(|target| crate::vulkan_types::vk_format(target.format))
            .collect();
        let depth_format = desc
            .depth
            .format
            .map(crate::vulkan_types::vk_format)
            .unwrap_or(vk::Format::UNDEFINED);

        let rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats)
            .depth_attachment_format(depth_format);

        let create_info = vk::GraphicsPipelineCreateInfo {
            p_stages: stages.as_ptr(),
            p_vertex_input_state: &vertex_input,
            p_input_assembly_state: &input_assembly,
            p_viewport_state: &viewport_state,
            p_rasterization_state: &rasterization,
            p_multisample_state: &multisample,
            p_depth_stencil_state: &depth_stencil,
            p_color_blend_state: &color_blend,
            layout,
            // Null render pass: dynamic rendering supplies the attachments, which
            // is why there is no VulkanRenderPass object in this backend.
            render_pass: vk::RenderPass::null(),
            subpass: 0,
            p_next: &rendering as *const _ as *const _,
            ..Default::default()
        };

        let pipelines = unsafe {
            self.device
                .create_graphics_pipelines(self.driver_cache, &[create_info], None)
        }
        .map_err(|(partial, error)| BackendError::vk(
            "vkCreateGraphicsPipelines",
            error,
        ).with_partial(partial.len()))?;

        let mut pipelines = pipelines;
        Ok(pipelines.remove(0))
    }

    pub fn get(&self, handle: Pipeline) -> Option<&PipelineResource> {
        self.registry.get(handle)
    }

    /// Pipelines actually compiled, versus requests served from the description
    /// cache. A `cache_hits` that stays at zero while a frame requests the same
    /// pipeline repeatedly means the hash is missing a field.
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

fn topology_of(value: u32) -> vk::PrimitiveTopology {
    match value {
        0 => vk::PrimitiveTopology::POINT_LIST,
        1 => vk::PrimitiveTopology::LINE_LIST,
        2 => vk::PrimitiveTopology::LINE_STRIP,
        3 => vk::PrimitiveTopology::TRIANGLE_STRIP,
        _ => vk::PrimitiveTopology::TRIANGLE_LIST,
    }
}

fn cull_mode_of(value: u32) -> vk::CullModeFlags {
    match value {
        0 => vk::CullModeFlags::NONE,
        2 => vk::CullModeFlags::FRONT,
        _ => vk::CullModeFlags::BACK,
    }
}

fn front_face_of(value: u32) -> vk::FrontFace {
    if value == 1 {
        vk::FrontFace::CLOCKWISE
    } else {
        vk::FrontFace::COUNTER_CLOCKWISE
    }
}