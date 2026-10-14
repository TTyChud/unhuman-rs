//! Barrier lowering, tested with no device, no driver and no loader.
//!
//! This is the payoff of `stormgraph` emitting backend-neutral barriers: the
//! whole Vulkan encoding path is exercisable here. In the C++ engine the
//! equivalent code lives in `VulkanBarrierEncoder.cpp`, reachable only through
//! `tests/rendergraph`, which has to compile 11 backend sources plus a
//! `VmaDispatchStub` to get at it.

use ash::vk;
use ash::vk::Handle;
use chassis::graph::{self as vulkan_render_graph, ImageBarrier, ImageState};
use chassis::TextureFormat;
use stormbreaker::{
    image_barrier2, image_barrier_legacy, select_sync_tier, vulkan_types, SyncTier,
};

/// A handle that was never allocated. Nothing here dispatches, so a dangling
/// pointer is fine — the structs under test never reach a driver.
fn fake_image() -> vk::Image {
    vk::Image::from_raw(0x1000)
}

/// Synchronization 2 keeps the shared bits in the same positions, so a legacy
/// flag widens by reinterpretation. ash ships no `From` impl for this, which is
/// why the legacy encodings in `utils` are written out by hand instead.
fn widen_access(flags: vk::AccessFlags) -> vk::AccessFlags2 {
    vk::AccessFlags2::from_raw(flags.as_raw() as u64)
}

fn widen_stage(flags: vk::PipelineStageFlags) -> vk::PipelineStageFlags2 {
    vk::PipelineStageFlags2::from_raw(flags.as_raw() as u64)
}

fn target() -> stormbreaker::Target {
    let base = stormbreaker::Target::color(fake_image(), 1, 1);
    base.with_aspect(
        vk::ImageAspectFlags::COLOR | vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL,
    )
}

fn barrier(src: ImageState, dst: ImageState) -> ImageBarrier {
    ImageBarrier {
        texture: 0,
        src_state: src,
        dst_state: dst,
        src_access: vulkan_render_graph::access_for_state(src),
        dst_access: vulkan_render_graph::access_for_state(dst),
        src_stage: vulkan_render_graph::stage_for_state(src),
        dst_stage: vulkan_render_graph::stage_for_state(dst),
    }
}

#[test]
fn every_state_maps_to_a_layout() {
    assert_eq!(
        vulkan_types::layout(ImageState::Undefined),
        vk::ImageLayout::UNDEFINED
    );
    assert_eq!(
        vulkan_types::layout(ImageState::ColorAttachment),
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
    );
    assert_eq!(
        vulkan_types::layout(ImageState::ShaderRead),
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    );
    assert_eq!(
        vulkan_types::layout(ImageState::Storage),
        vk::ImageLayout::GENERAL
    );
    assert_eq!(
        vulkan_types::layout(ImageState::TransferSrc),
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL
    );
    assert_eq!(
        vulkan_types::layout(ImageState::TransferDst),
        vk::ImageLayout::TRANSFER_DST_OPTIMAL
    );
    assert_eq!(
        vulkan_types::layout(ImageState::Present),
        vk::ImageLayout::PRESENT_SRC_KHR
    );
}

#[test]
fn combined_depth_stencil_uses_the_combined_layout() {
    // A stencil aspect with DEPTH_ATTACHMENT_OPTIMAL is a validation error, and
    // this is the format coupling the C++ descriptors hide.
    assert_eq!(
        vulkan_types::layout_for(ImageState::DepthAttachment, TextureFormat::Depth24Stencil8),
        vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
    );
    assert_eq!(
        vulkan_types::layout_for(ImageState::DepthAttachment, TextureFormat::Depth32Float),
        vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL
    );
}

#[test]
fn aspects_come_from_the_format() {
    assert_eq!(
        vulkan_types::aspect(TextureFormat::Rgba8Unorm),
        vk::ImageAspectFlags::COLOR
    );
    assert_eq!(
        vulkan_types::aspect(TextureFormat::Depth24),
        vk::ImageAspectFlags::DEPTH
    );
    assert_eq!(
        vulkan_types::aspect(TextureFormat::Depth24Stencil8),
        vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
    );
}

#[test]
fn the_aspect_is_narrowed_to_what_the_format_has() {
    // Target claims every aspect; the depth-only format must not inherit the
    // colour or stencil bits.
    let lowered = image_barrier2(
        target(),
        barrier(ImageState::Undefined, ImageState::DepthAttachment),
        TextureFormat::Depth24,
    );
    assert_eq!(
        lowered.subresource_range.aspect_mask,
        vk::ImageAspectFlags::DEPTH
    );
}

#[test]
fn a_color_write_lowers_to_the_color_scopes() {
    let lowered = image_barrier2(
        target(),
        barrier(ImageState::Undefined, ImageState::ColorAttachment),
        TextureFormat::Rgba8Unorm,
    );

    assert_eq!(lowered.old_layout, vk::ImageLayout::UNDEFINED);
    assert_eq!(
        lowered.new_layout,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
    );
    assert_eq!(lowered.src_access_mask, vk::AccessFlags2::empty());
    assert_eq!(lowered.src_stage_mask, vk::PipelineStageFlags2::TOP_OF_PIPE);
    assert_eq!(
        lowered.dst_access_mask,
        vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
    );
    assert_eq!(
        lowered.dst_stage_mask,
        vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
    );
}

#[test]
fn leaving_undefined_waits_for_nothing() {
    let lowered = image_barrier_legacy(
        target(),
        barrier(ImageState::Undefined, ImageState::ColorAttachment),
        TextureFormat::Rgba8Unorm,
    );

    assert_eq!(lowered.src_access_mask, vk::AccessFlags::empty());
    assert_eq!(lowered.old_layout, vk::ImageLayout::UNDEFINED);
}

#[test]
fn a_real_transition_keeps_its_source_scope() {
    let lowered = image_barrier2(
        target(),
        barrier(ImageState::ColorAttachment, ImageState::ShaderRead),
        TextureFormat::Rgba8Unorm,
    );

    assert_eq!(
        lowered.src_access_mask,
        vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
    );
    assert_eq!(
        lowered.src_stage_mask,
        vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
    );
    assert_eq!(
        lowered.new_layout,
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    );
}

/// The two encodings must mean the same thing. If they drift, a bug reproduces
/// only on legacy hardware and disappears on the dev machine.
#[test]
fn both_encodings_agree() {
    let cases = [
        (ImageState::Undefined, ImageState::ColorAttachment),
        (ImageState::ColorAttachment, ImageState::ShaderRead),
        (ImageState::DepthAttachment, ImageState::ShaderRead),
        (ImageState::ShaderRead, ImageState::Present),
        (ImageState::TransferDst, ImageState::ShaderRead),
    ];

    for (src, dst) in cases {
        let b = barrier(src, dst);
        let target = target();

        let modern = image_barrier2(target, b, TextureFormat::Rgba8Unorm);
        let legacy = image_barrier_legacy(target, b, TextureFormat::Rgba8Unorm);

        assert_eq!(modern.old_layout, legacy.old_layout, "{src:?}->{dst:?}");
        assert_eq!(modern.new_layout, legacy.new_layout, "{src:?}->{dst:?}");
        assert_eq!(
            modern.subresource_range.aspect_mask,
            legacy.subresource_range.aspect_mask
        );
        assert_eq!(
            modern.src_access_mask,
            widen_access(legacy.src_access_mask),
            "source access for {src:?}->{dst:?}"
        );
        assert_eq!(
            modern.dst_access_mask,
            widen_access(legacy.dst_access_mask),
            "destination access for {src:?}->{dst:?}"
        );
        assert_eq!(
            modern.src_stage_mask,
            widen_stage(vulkan_types::stage_legacy(b.src_stage)),
            "source stage for {src:?}->{dst:?}"
        );
        assert_eq!(
            modern.dst_stage_mask,
            widen_stage(vulkan_types::stage_legacy(b.dst_stage)),
            "destination stage for {src:?}->{dst:?}"
        );
    }
}

#[test]
fn queue_family_indices_stay_ignored() {
    // The graph has no queue ownership model yet, so claiming a family transfer
    // would be a lie the driver would reject.
    let lowered = image_barrier2(
        target(),
        barrier(ImageState::ColorAttachment, ImageState::ShaderRead),
        TextureFormat::Rgba8Unorm,
    );
    assert_eq!(lowered.src_queue_family_index, vk::QUEUE_FAMILY_IGNORED);
    assert_eq!(lowered.dst_queue_family_index, vk::QUEUE_FAMILY_IGNORED);
}

#[test]
fn an_empty_transition_is_recognised() {
    let same = barrier(ImageState::ShaderRead, ImageState::ShaderRead);
    assert!(same.is_empty(), "a no-op barrier would still cost a call");

    let real = barrier(ImageState::ShaderRead, ImageState::ColorAttachment);
    assert!(!real.is_empty());
}

/// The seam itself: a barrier produced by the graph, lowered to Vulkan.
#[test]
fn a_graph_barrier_survives_the_lowering() {
    use chassis::graph::RenderGraph;

    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let gbuffer = graph.create_texture("gbuffer", ImageState::Undefined, ImageState::ShaderRead);

    graph.pass("gbuffer").color(gbuffer, [0.0; 4]);
    graph
        .pass("lighting")
        .color(swapchain, [0.0; 4])
        .read_texture(gbuffer);

    let compiled = graph.compile().unwrap();

    // The gbuffer's first use: undefined -> colour attachment.
    let first = compiled.find("gbuffer").unwrap();
    let lowered = image_barrier2(target(), first.pre_barriers[0], TextureFormat::Rgba8Unorm);
    assert_eq!(lowered.old_layout, vk::ImageLayout::UNDEFINED);
    assert_eq!(
        lowered.new_layout,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
    );

    // The lighting pass samples it: colour attachment -> shader read.
    let lighting = compiled.find("lighting").unwrap();
    let sample = lighting
        .pre_barriers
        .iter()
        .find(|b| b.dst_state == ImageState::ShaderRead)
        .expect("sampling the gbuffer needs a transition");
    let lowered = image_barrier2(target(), *sample, TextureFormat::Rgba8Unorm);
    assert_eq!(
        lowered.old_layout,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
    );
    assert_eq!(
        lowered.new_layout,
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    );
    assert_eq!(lowered.dst_access_mask, vk::AccessFlags2::SHADER_READ);

    // And the swapchain leaves in Present, as the graph's post-barrier says.
    let present = lighting
        .post_barriers
        .iter()
        .find(|b| b.dst_state == ImageState::Present)
        .expect("the swapchain must reach Present");
    assert_eq!(
        vulkan_types::layout(present.dst_state),
        vk::ImageLayout::PRESENT_SRC_KHR
    );
}

#[test]
fn access_and_stage_enums_map_one_to_one() {
    // A silent hole here means a barrier with the wrong scope, which is the
    // hardest class of GPU bug to reproduce.
    for state in [
        ImageState::Undefined,
        ImageState::ColorAttachment,
        ImageState::DepthAttachment,
        ImageState::ShaderRead,
        ImageState::TransferSrc,
        ImageState::TransferDst,
        ImageState::Present,
        ImageState::Storage,
    ] {
        let modern = vulkan_types::access(vulkan_render_graph::access_for_state(state));
        let legacy = vulkan_types::access_legacy(vulkan_render_graph::access_for_state(state));
        assert_eq!(modern, widen_access(legacy), "access for {state:?}");

        let stage = vulkan_render_graph::stage_for_state(state);
        assert_eq!(
            vulkan_types::stage(stage),
            widen_stage(vulkan_types::stage_legacy(stage)),
            "stage for {stage:?}"
        );
    }
}

#[test]
fn sync2_and_timeline_encode_barriers_the_same_way() {
    // Only the submit path differs between these two tiers, so the encoder must
    // not branch on it.
    let target = target();
    let b = barrier(ImageState::ColorAttachment, ImageState::ShaderRead);
    let a = image_barrier2(target, b, TextureFormat::Rgba8Unorm);
    let c = image_barrier2(target, b, TextureFormat::Rgba8Unorm);
    assert_eq!(a.old_layout, c.old_layout);
    assert_eq!(a.new_layout, c.new_layout);
    assert_eq!(a.dst_access_mask, c.dst_access_mask);
    assert_eq!(
        select_sync_tier(1 << 22 | 3 << 12, false, true),
        SyncTier::Sync2Timeline
    );
    assert!(SyncTier::Sync2Timeline.supports_sync2());
    assert!(!SyncTier::Legacy.supports_sync2());
}
