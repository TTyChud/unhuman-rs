//! Graphics pipelines.
//!
//! Mirrors `VulkanGraphicPipeline.cpp`. The shared description, its structural
//! hash and the pipeline cache live in `vulkan_pipeline_state.rs`; compute is in
//! `vulkan_compute_pipeline.rs`.
//!
//! Created through **dynamic rendering** (`VkPipelineRenderingCreateInfo`) rather
//! than `VkRenderPass`. The C++ engine makes dynamic rendering a hard floor
//! (`VulkanDevice.cpp:87-91`) while still carrying `VkRenderPass`-shaped
//! attachments in its descriptors (`RHITypes.h:373-384`), so its render-pass
//! objects are dead weight. Here `chassis::RenderPassDesc` *is* the rendering
//! info and there is no render-pass object to create or destroy.

use ash::vk;
use chassis::Pipeline;

use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_pipeline_state::{PipelineHash, PipelineResource, Pipelines};
use crate::vulkan_shader::Shaders;
use crate::vulkan_types;

pub fn create(
    pipelines: &mut Pipelines,
    desc: &chassis::PipelineDesc,
    shaders: &Shaders,
) -> Result<Pipeline> {
    let hash = PipelineHash::of(desc);
    if let Some(existing) = pipelines.by_description(hash) {
        pipelines.mark_cache_hit(existing);
        return Ok(existing);
    }

    let layout = pipelines.create_layout(&())?;
    let handle = build(pipelines, desc, shaders, layout)?;

    Ok(pipelines.insert(hash, PipelineResource::new(handle, layout, &desc.label)))
}

/// Compiles one graphics pipeline. Dynamic rendering, no `VkRenderPass`.
fn build(
    pipelines: &Pipelines,
    desc: &chassis::PipelineDesc,
    shaders: &Shaders,
    layout: vk::PipelineLayout,
) -> Result<vk::Pipeline> {
    let device = pipelines.device();

    let Some(shader) = shaders.get(desc.shader) else {
        return Err(BackendError::InvalidHandle);
    };

    let entry_name = c"main";
    let stage = vk::PipelineShaderStageCreateInfo {
        stage: vk::ShaderStageFlags::VERTEX,
        module: shader.module(),
        p_name: entry_name.as_ptr(),
        ..Default::default()
    };
    let stages = [stage];

    let attributes: Vec<vk::VertexInputAttributeDescription> = desc
        .vertex
        .attributes
        .iter()
        .map(|attribute| vk::VertexInputAttributeDescription {
            location: attribute.location,
            binding: 0,
            format: vulkan_types::vk_format(attribute.format),
            offset: attribute.offset,
        })
        .collect();

    // One binding, because the layout is flat: a second binding would need a
    // binding index on every attribute, and nothing needs one yet.
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

    // Dynamic viewport and scissor: every pass reads the extent from the
    // swapchain, and baking one in means recompiling on every resize.
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .depth_clamp_enable(false)
        .rasterizer_discard_enable(false)
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(cull_mode_of(desc.primitive.cull_mode))
        .front_face(front_face_of(desc.primitive.front_face))
        .depth_bias_enable(false)
        .line_width(1.0);

    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vulkan_types::vk_samples(desc.samples))
        .sample_shading_enable(false);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(desc.depth.format.is_some())
        .depth_write_enable(desc.depth.write_enabled)
        .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let color_blend_attachments: Vec<vk::PipelineColorBlendAttachmentState> = desc
        .color_targets
        .iter()
        .map(|target| blend_state(target.blend, target.write_mask))
        .collect();

    let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
        .logic_op_enable(false)
        .attachments(&color_blend_attachments);

    let color_formats: Vec<vk::Format> = desc
        .color_targets
        .iter()
        .map(|target| vulkan_types::vk_format(target.format))
        .collect();
    let depth_format = desc
        .depth
        .format
        .map(vulkan_types::vk_format)
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
        // Null render pass: dynamic rendering supplies the attachments.
        render_pass: vk::RenderPass::null(),
        subpass: 0,
        p_next: &rendering as *const _ as *const _,
        ..Default::default()
    };

    let pipelines_out =
        unsafe { device.create_graphics_pipelines(pipelines.driver_cache(), &[create_info], None) }
            .map_err(|(partial, error)| {
                BackendError::vk("vkCreateGraphicsPipelines", error).with_partial(partial.len())
            })?;

    let mut pipelines_out = pipelines_out;
    Ok(pipelines_out.remove(0))
}

fn blend_state(blend: bool, write_mask: u8) -> vk::PipelineColorBlendAttachmentState {
    // Standard alpha blend when asked for, otherwise a plain write.
    vk::PipelineColorBlendAttachmentState {
        blend_enable: if blend { vk::TRUE } else { vk::FALSE },
        src_color_blend_factor: if blend {
            vk::BlendFactor::SRC_ALPHA
        } else {
            vk::BlendFactor::ONE
        },
        dst_color_blend_factor: if blend {
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA
        } else {
            vk::BlendFactor::ZERO
        },
        color_blend_op: vk::BlendOp::ADD,
        src_alpha_blend_factor: vk::BlendFactor::ONE,
        dst_alpha_blend_factor: vk::BlendFactor::ZERO,
        alpha_blend_op: vk::BlendOp::ADD,
        color_write_mask: vk::ColorComponentFlags::from_raw(write_mask as u32),
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