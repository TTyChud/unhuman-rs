//! Framebuffers.
//!
//! Mirrors `VulkanFramebuffer.cpp`.
//!
//! **Nearly empty under dynamic rendering, and that is the point.** A
//! `VkFramebuffer` binds a set of attachments to a render pass, and
//! `VkRenderPass` no longer exists in this backend. Attachment sets for a
//! dynamic-rendering pass are described by `chassis::RenderPassDesc` at record
//! time, so there is nothing left for a framebuffer object to do.
//!
//! The C++ engine has both a `VulkanRenderPass.cpp` and a `VulkanFramebuffer.cpp`
//! while making dynamic rendering mandatory at `VulkanDevice.cpp:87-91`, which is
//! how dead code accumulates. This file exists for parity and carries the
//! compatibility path only: a framebuffer for a pass that renders *into* another
//! pass's attachments, which dynamic rendering does not cover.
//!
//! Nothing in this engine needs that yet, so the constructor is here, unused and
//! honest, rather than called from a render path.

use ash::vk;

use crate::vulkan_error::{BackendError, Result};

/// A `VkFramebuffer`: attachments bound to a render pass.
pub struct Framebuffer {
    device: ash::Device,
    framebuffer: vk::Framebuffer,
    width: u32,
    height: u32,
    layers: u32,
}

impl Framebuffer {
    /// Needs a render pass, so this is only reachable once one exists. Dynamic
    /// rendering does not need it, which is why there is no call site.
    pub fn create(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        attachments: &[vk::ImageView],
        width: u32,
        height: u32,
        layers: u32,
    ) -> Result<Self> {
        if attachments.is_empty() {
            return Err(BackendError::EmptyUsage {
                what: "framebuffer",
            });
        }
        if render_pass == vk::RenderPass::null() {
            return Err(BackendError::InvalidHandle);
        }

        let info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(attachments)
            .width(width)
            .height(height)
            .layers(layers);

        let framebuffer = unsafe { device.create_framebuffer(&info, None) }
            .map_err(|e| BackendError::vk("vkCreateFramebuffer", e))?;

        Ok(Self {
            device: device.clone(),
            framebuffer,
            width,
            height,
            layers,
        })
    }

    pub fn handle(&self) -> vk::Framebuffer {
        self.framebuffer
    }

    pub fn extent(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn layers(&self) -> u32 {
        self.layers
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe { self.device.destroy_framebuffer(self.framebuffer, None) };
    }
}
