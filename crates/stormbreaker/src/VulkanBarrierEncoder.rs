//! The single lowering point from graph barriers to Vulkan commands.
//!
//! `stormgraph` emits backend-neutral [`stormgraph::ImageBarrier`]s; this turns
//! them into `vkCmdPipelineBarrier2` or `vkCmdPipelineBarrier` depending on the
//! device's `SyncTier`. There is no second path: a pass cannot hand-write a
//! barrier, because `chassis::CommandList` has no barrier method at all.
//!
//! That is the whole reason the C++ engine has two owners of synchronization.
//! `VulkanCommandBuffer::BeginRenderPass` writes its own barriers
//! (`VulkanCommandBuffer.cpp:104,153`) while the graph synthesizes others, and
//! the two are reconciled by a boolean handshake
//! (`MarkSwapchainRenderedByLegacy`, `VulkanDevice.h:82`).

use ash::vk;
use chassis::TextureFormat;
use stormgraph::{BufferBarrier, ImageBarrier};

use crate::vulkan_extension_check::SyncTier;
use crate::vulkan_types;

/// Which handle the barrier applies to. Resolved by the caller from the graph's
/// texture slot; the graph itself never sees a Vulkan handle.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub image: vk::Image,
    pub subresource: vk::ImageSubresourceRange,
}

impl Target {
    pub fn color(image: vk::Image, mip_levels: u32, layers: u32) -> Self {
        Self {
            image,
            subresource: vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: mip_levels,
                base_array_layer: 0,
                layer_count: layers,
            },
        }
    }

    pub fn with_aspect(mut self, aspect: vk::ImageAspectFlags) -> Self {
        self.subresource.aspect_mask = aspect;
        self
    }
}

/// The buffer counterpart of [`image_barrier2`].
///
/// Same tier split, and the same reason for existing as a separate function:
/// `vk::BufferMemoryBarrier2` carries stage and access masks in the struct and
/// `vk::BufferMemoryBarrier` does not, so one cannot be built from the other.
pub fn buffer_barrier2(
    buffer: vk::Buffer,
    offset: u64,
    size: vk::DeviceSize,
    barrier: BufferBarrier,
) -> vk::BufferMemoryBarrier2<'static> {
    vk::BufferMemoryBarrier2::default()
        .src_stage_mask(vulkan_types::stage(barrier.src_stage))
        .src_access_mask(vulkan_types::access(barrier.src_access))
        .dst_stage_mask(vulkan_types::stage(barrier.dst_stage))
        .dst_access_mask(vulkan_types::access(barrier.dst_access))
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(offset)
        .size(size)
}

pub fn buffer_barrier_legacy(
    buffer: vk::Buffer,
    offset: u64,
    size: vk::DeviceSize,
    barrier: BufferBarrier,
) -> vk::BufferMemoryBarrier<'static> {
    vk::BufferMemoryBarrier::default()
        .src_access_mask(vulkan_types::access_legacy(barrier.src_access))
        .dst_access_mask(vulkan_types::access_legacy(barrier.dst_access))
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(offset)
        .size(size)
}

/// ash gives these structs a lifetime parameter that only exists for its
/// builder. Nothing here borrows, so `'static` is accurate.
///
/// Split out from recording so the lowering is testable with no command buffer,
/// no device and no driver — these are plain structs.
pub fn image_barrier2(
    target: Target,
    barrier: ImageBarrier,
    format: TextureFormat,
) -> vk::ImageMemoryBarrier2<'static> {
    vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vulkan_types::stage(barrier.src_stage))
        .src_access_mask(vulkan_types::access(barrier.src_access))
        .dst_stage_mask(vulkan_types::stage(barrier.dst_stage))
        .dst_access_mask(vulkan_types::access(barrier.dst_access))
        .old_layout(vulkan_types::layout_for(barrier.src_state, format))
        .new_layout(vulkan_types::layout_for(barrier.dst_state, format))
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(target.image)
        .subresource_range(subresource_for(target, format))
}

/// The `vk::ImageMemoryBarrier` encoding for the pre-1.3 path. Stages and
/// accesses live in the mask arguments rather than the struct, which is why the
/// two encodings are separate functions instead of one struct with fields left
/// empty.
pub fn image_barrier_legacy(
    target: Target,
    barrier: ImageBarrier,
    format: TextureFormat,
) -> vk::ImageMemoryBarrier<'static> {
    vk::ImageMemoryBarrier::default()
        .src_access_mask(vulkan_types::access_legacy(barrier.src_access))
        .dst_access_mask(vulkan_types::access_legacy(barrier.dst_access))
        .old_layout(vulkan_types::layout_for(barrier.src_state, format))
        .new_layout(vulkan_types::layout_for(barrier.dst_state, format))
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(target.image)
        .subresource_range(subresource_for(target, format))
}

/// The target's range, narrowed to the aspects the format actually has. A
/// stencil aspect on a depth-only format is a validation error, and a colour
/// aspect on a depth image is worse.
fn subresource_for(target: Target, format: TextureFormat) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vulkan_types::aspect(format) & target.subresource.aspect_mask,
        ..target.subresource
    }
}

pub struct BarrierEncoder<'a> {
    tier: SyncTier,
    device: &'a ash::Device,
}

impl<'a> BarrierEncoder<'a> {
    pub fn new(tier: SyncTier, device: &'a ash::Device) -> Self {
        Self { tier, device }
    }

    pub fn tier(&self) -> SyncTier {
        self.tier
    }

    /// Records one barrier. Empty transitions are dropped, so the graph does not
    /// have to filter them before recording.
    pub fn encode(
        &self,
        command_buffer: vk::CommandBuffer,
        target: Target,
        barrier: &ImageBarrier,
        format: TextureFormat,
    ) {
        if barrier.is_empty() {
            return;
        }

        match self.tier {
            SyncTier::Legacy => unsafe {
                let barriers = [image_barrier_legacy(target, *barrier, format)];
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vulkan_types::stage_legacy(barrier.src_stage),
                    vulkan_types::stage_legacy(barrier.dst_stage),
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &barriers,
                );
            },
            SyncTier::Sync2 | SyncTier::Sync2Timeline => unsafe {
                let barriers = [image_barrier2(target, *barrier, format)];
                let dependency = vk::DependencyInfo::default()
                    .dependency_flags(vk::DependencyFlags::empty())
                    .image_memory_barriers(&barriers);
                self.device.cmd_pipeline_barrier2(command_buffer, &dependency);
            },
        }
    }
}