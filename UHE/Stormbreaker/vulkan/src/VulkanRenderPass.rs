//! Legacy `VkRenderPass`, for hardware without dynamic rendering.
//!
//! Ported from `UHE/src/Platform/Vulkan/VulkanRenderPass.cpp`.
//!
//! ## Why this exists when the engine uses dynamic rendering everywhere
//!
//! The C++ carries it for one reason, stated in the header's own `\todo`: the
//! dynamic-rendering render-pass builder was not finished, so old hardware had
//! no fallback. `VulkanGraphicPipeline.cpp:176` passes `renderPass = nullptr`,
//! which is correct for dynamic rendering, and `VulkanGraphicPipeline.h:49`
//! holds a `m_FallbackRenderPass` for when it is not.
//!
//! So this is not dead code to be tidied away. It is the only path that works on
//! a 1.0 device, this engine targets 1.1, and it is also the escape hatch if a
//! driver turns out to ship a broken dynamic-rendering implementation.
//!
//! ## What is not wired up yet
//!
//! Device creation picks dynamic rendering when
//! `Capabilities::dynamic_rendering` is set, and nothing picks otherwise. That
//! decision belongs at device creation, not here. What is missing is not this
//! file -- it is the one branch that says "not supported, so use the other
//! thing".
//!
//! ## The thing to be careful about
//!
//! A `VkRenderPass` built here has to agree with the pipeline's
//! `VkPipelineRenderingCreateInfo` formats, and neither is derived from the
//! other. When both paths are live one reads formats from `PipelineDesc` and the
//! other from `RenderPassDesc`; if they disagree the failure is a
//! pipeline-creation error naming a format number, not a render-pass error.

use ash::vk;
use chassis::RenderPassDesc;

use crate::vulkan_error::{BackendError, Result};
use crate::vulkan_types;

/// A legacy render pass.
///
/// Holds the device so `Drop` can destroy the handle. Every other owner in this
/// backend does the same, and a Vulkan handle with no `Drop` is a leak that
/// validation only reports at `vkDestroyDevice` as
/// VUID-vkDestroyDevice-device-05137 -- an error about teardown that names
/// nothing about which object survived.
pub struct RenderPass {
    device: ash::Device,
    handle: vk::RenderPass,
    label: String,
    color_attachments: u32,
    has_depth: bool,
}

impl RenderPass {
    /// Build a render pass: subpass 0, one colour attachment per colour plus an
    /// optional depth attachment.
    pub fn new(device: &ash::Device, desc: &RenderPassDesc, label: &str) -> Result<Self> {
        let mut attachments = Vec::with_capacity(desc.colors.len() + 1);
        for color in &desc.colors {
            attachments.push(attachment(
                vulkan_types::vk_format(color.format),
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                load_op_of(color.load),
                store_op_of(color.store),
            ));
        }
        let depth_layout = desc.depth.as_ref().map(|depth| {
            // `Depth24` is `D24_UNORM_S8_UINT`, so it is a *combined* format and
            // a depth-only layout on it is invalid. Keyed on the same predicate
            // the barrier lowering uses, because two places deciding this
            // separately is how they disagree.
            vulkan_types::render_pass_depth_layout(depth.format)
        });
        if let Some(depth) = &desc.depth {
            attachments.push(attachment(
                vulkan_types::vk_format(depth.format),
                depth_layout.unwrap_or(vk::ImageLayout::UNDEFINED),
                load_op_of(depth.load),
                store_op_of(depth.store),
            ));
        }

        // Materialised before the subpass that points at them. Building them
        // inline inside `.map(..).collect::<Vec<_>>().as_ptr()` leaves the
        // subpass holding a pointer into a temporary that is dropped on the
        // same statement -- a dangling pointer that validation reports as
        // nothing at all, because it is never dereferenced by us.
        let colour_references: Vec<vk::AttachmentReference> = (0..desc.colors.len())
            .map(|index| vk::AttachmentReference {
                attachment: index as u32,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            })
            .collect();
        let depth_reference = vk::AttachmentReference {
            attachment: desc.colors.len() as u32,
            layout: depth_layout.unwrap_or(vk::ImageLayout::UNDEFINED),
        };

        // Struct literals, not builders: ash's builder for these sets the
        // pointer and leaves the count at zero. That is the third time this has
        // bitten -- `stage_count` segfaulted RADV, `dynamic_state_count` was a
        // validation error -- and the pattern is consistent enough to be worth
        // writing down rather than rediscovering.
        let subpass = vk::SubpassDescription {
            pipeline_bind_point: vk::PipelineBindPoint::GRAPHICS,
            color_attachment_count: colour_references.len() as u32,
            p_color_attachments: colour_references.as_ptr(),
            p_depth_stencil_attachment: if desc.depth.is_some() {
                &depth_reference as *const _
            } else {
                std::ptr::null()
            },
            ..Default::default()
        };

        // One external dependency covering both attachments. Splitting it per
        // attachment would be finer-grained and is what a production engine does
        // eventually; one dependency is correct and costs a broader wait.
        let dependencies = [vk::SubpassDependency {
            src_subpass: vk::SUBPASS_EXTERNAL,
            dst_subpass: 0,
            src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            src_access_mask: vk::AccessFlags::empty(),
            dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            dependency_flags: vk::DependencyFlags::empty(),
        }];

        let info = vk::RenderPassCreateInfo {
            attachment_count: attachments.len() as u32,
            p_attachments: attachments.as_ptr(),
            subpass_count: 1,
            p_subpasses: &subpass as *const _,
            dependency_count: dependencies.len() as u32,
            p_dependencies: dependencies.as_ptr(),
            ..Default::default()
        };

        let handle = unsafe { device.create_render_pass(&info, None) }
            .map_err(|e| BackendError::vk("vkCreateRenderPass", e))?;

        // No debug name here. `VulkanScopes` owns the debug-utils loader and is
        // the one place object naming happens; a render pass that reached around
        // it for a loader would be a second path to the same extension, and two
        // paths to an extension is how one of them ends up not enabled.

        Ok(Self {
            device: device.clone(),
            handle,
            label: label.to_owned(),
            color_attachments: desc.colors.len() as u32,
            has_depth: desc.depth.is_some(),
        })
    }

    pub fn handle(&self) -> vk::RenderPass {
        self.handle
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// How many colour attachments this pass was built for.
    ///
    /// A pipeline created against it has to declare the same count in its
    /// `VkPipelineRenderingCreateInfo`, or the driver reports a format-index
    /// mismatch with nothing pointing here.
    pub fn color_attachment_count(&self) -> u32 {
        self.color_attachments
    }

    pub fn has_depth(&self) -> bool {
        self.has_depth
    }
}

impl Drop for RenderPass {
    fn drop(&mut self) {
        unsafe { self.device.destroy_render_pass(self.handle, None) };
    }
}

fn attachment(
    format: vk::Format,
    final_layout: vk::ImageLayout,
    load_op: vk::AttachmentLoadOp,
    store_op: vk::AttachmentStoreOp,
) -> vk::AttachmentDescription {
    vk::AttachmentDescription {
        flags: vk::AttachmentDescriptionFlags::empty(),
        format,
        // Single-sampled. MSAA needs one attachment per sample plus a resolve
        // target, which is a different builder, and a legacy render pass is
        // only for hardware that cannot do better.
        samples: vk::SampleCountFlags::TYPE_1,
        load_op,
        store_op,
        // The stencil aspect of a combined format has to be loaded and stored
        // deliberately. DONT_CARE is right for a depth-only format and leaves
        // the stencil undefined for a combined one -- which renders, just not
        // what was asked for.
        stencil_load_op: if final_layout == vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL {
            load_op
        } else {
            vk::AttachmentLoadOp::DONT_CARE
        },
        stencil_store_op: if final_layout == vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL {
            store_op
        } else {
            vk::AttachmentStoreOp::DONT_CARE
        },
        // `UNDEFINED` on entry discards whatever was there, and that is the
        // right choice for Clear and DontCare. It is *invalid* for Load:
        // VUID-VkAttachmentDescription-format-06699 requires that a LOAD has a
        // real entry layout, because loading from UNDEFINED reads undefined
        // contents -- a bug that renders garbage rather than failing.
        initial_layout: if load_op == vk::AttachmentLoadOp::LOAD {
            final_layout
        } else {
            vk::ImageLayout::UNDEFINED
        },
        final_layout,
    }
}

fn load_op_of(load: chassis::LoadOp) -> vk::AttachmentLoadOp {
    match load {
        chassis::LoadOp::Clear => vk::AttachmentLoadOp::CLEAR,
        chassis::LoadOp::Load => vk::AttachmentLoadOp::LOAD,
        chassis::LoadOp::DontCare => vk::AttachmentLoadOp::DONT_CARE,
    }
}

fn store_op_of(store: bool) -> vk::AttachmentStoreOp {
    if store {
        vk::AttachmentStoreOp::STORE
    } else {
        vk::AttachmentStoreOp::DONT_CARE
    }
}
