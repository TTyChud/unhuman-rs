//! Conversions from backend-neutral enums to Vulkan values.
//!
//! This module and `barrier_encoder` are the *only* places in the backend that
//! decide how a graph barrier becomes Vulkan. That is the payoff of
//! `stormgraph` emitting backend-neutral `ImageBarrier`s: the `SyncTier` choice
//! lives in one function per concept instead of being baked into every barrier
//! the graph emits, which is how the C++ engine ended up with
//! `VulkanBarrierEncoder.h:58-74` doing the lowering and
//! `VulkanCommandBuffer.cpp:104,153` hand-writing `oldLayout = eUndefined`
//! alongside it.

use ash::vk;
use chassis::TextureFormat;
use stormgraph::{Access, ImageState, Stage};

pub fn layout(state: ImageState) -> vk::ImageLayout {
    layout_for(state, TextureFormat::Rgba8Unorm)
}

/// Depth24Stencil8 must use the combined layout: a stencil aspect with
/// `DEPTH_ATTACHMENT_OPTIMAL` is a validation error, and this is exactly the
/// kind of format coupling the C++ `RHITypes.h` descriptors hide.
pub fn layout_for(state: ImageState, format: TextureFormat) -> vk::ImageLayout {
    match state {
        ImageState::Undefined => vk::ImageLayout::UNDEFINED,
        ImageState::ColorAttachment => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        ImageState::DepthAttachment => {
            if format == TextureFormat::Depth24Stencil8 {
                vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
            } else {
                vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL
            }
        }
        ImageState::ShaderRead => vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        ImageState::TransferSrc => vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        ImageState::TransferDst => vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        ImageState::Present => vk::ImageLayout::PRESENT_SRC_KHR,
        ImageState::Storage => vk::ImageLayout::GENERAL,
    }
}

pub fn aspect(format: TextureFormat) -> vk::ImageAspectFlags {
    match format {
        TextureFormat::Depth16 | TextureFormat::Depth24 | TextureFormat::Depth32Float => {
            vk::ImageAspectFlags::DEPTH
        }
        TextureFormat::Depth24Stencil8 => {
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        }
        _ => vk::ImageAspectFlags::COLOR,
    }
}

pub fn access(access: Access) -> vk::AccessFlags2 {
    match access {
        Access::None => vk::AccessFlags2::empty(),
        Access::TransferRead => vk::AccessFlags2::TRANSFER_READ,
        Access::TransferWrite => vk::AccessFlags2::TRANSFER_WRITE,
        Access::ShaderRead => vk::AccessFlags2::SHADER_READ,
        Access::ShaderWrite => vk::AccessFlags2::SHADER_WRITE,
        Access::ColorWrite => vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        Access::DepthWrite => vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
        Access::MemoryRead => vk::AccessFlags2::MEMORY_READ,
        Access::MemoryWrite => vk::AccessFlags2::MEMORY_WRITE,
    }
}

pub fn stage(stage: Stage) -> vk::PipelineStageFlags2 {
    match stage {
        Stage::None => vk::PipelineStageFlags2::empty(),
        Stage::TopOfPipe => vk::PipelineStageFlags2::TOP_OF_PIPE,
        Stage::Transfer => vk::PipelineStageFlags2::TRANSFER,
        Stage::ComputeShader => vk::PipelineStageFlags2::COMPUTE_SHADER,
        Stage::VertexShader => vk::PipelineStageFlags2::VERTEX_SHADER,
        Stage::FragmentShader => vk::PipelineStageFlags2::FRAGMENT_SHADER,
        Stage::ColorAttachmentOutput => vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
        Stage::EarlyFragmentTests => vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS,
        Stage::LateFragmentTests => vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS,
        Stage::BottomOfPipe => vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
    }
}

/// Legacy `vk::AccessFlags`, for the `vkCmdPipelineBarrier` path.
pub fn access_legacy(access: Access) -> vk::AccessFlags {
    match access {
        Access::None => vk::AccessFlags::empty(),
        Access::TransferRead => vk::AccessFlags::TRANSFER_READ,
        Access::TransferWrite => vk::AccessFlags::TRANSFER_WRITE,
        Access::ShaderRead => vk::AccessFlags::SHADER_READ,
        Access::ShaderWrite => vk::AccessFlags::SHADER_WRITE,
        Access::ColorWrite => vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        Access::DepthWrite => vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
        Access::MemoryRead => vk::AccessFlags::MEMORY_READ,
        Access::MemoryWrite => vk::AccessFlags::MEMORY_WRITE,
    }
}

/// Legacy `vk::PipelineStageFlags`, for the `vkCmdPipelineBarrier` path.
pub fn stage_legacy(stage: Stage) -> vk::PipelineStageFlags {
    match stage {
        Stage::None => vk::PipelineStageFlags::empty(),
        Stage::TopOfPipe => vk::PipelineStageFlags::TOP_OF_PIPE,
        Stage::Transfer => vk::PipelineStageFlags::TRANSFER,
        Stage::ComputeShader => vk::PipelineStageFlags::COMPUTE_SHADER,
        Stage::VertexShader => vk::PipelineStageFlags::VERTEX_SHADER,
        Stage::FragmentShader => vk::PipelineStageFlags::FRAGMENT_SHADER,
        Stage::ColorAttachmentOutput => vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        Stage::EarlyFragmentTests => vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
        Stage::LateFragmentTests => vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
        Stage::BottomOfPipe => vk::PipelineStageFlags::BOTTOM_OF_PIPE,
    }
}