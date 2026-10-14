//! Bind groups, through `chassis::Resources` and recorded into a command list.
//!
//! Both backends refused to implement `create_bind_group` with a comment saying
//! no layout was declared. This is the layout being declared, and the group
//! being built from it.
//!
//! The interesting assertions are the *failures*: a bind group is a thing that
//! binds nothing and looks fine until a shader reads the wrong memory, so the
//! cases that must be caught are the mismatched ones.

use ash::vk::Handle;
use chassis::{
    BindingDesc, BufferDesc, BufferUsage, DescriptorKind, DescriptorSetLayoutDesc, Resources,
    StageMask,
};

fn uniform_layout(binding: u32) -> DescriptorSetLayoutDesc {
    let mut layout = DescriptorSetLayoutDesc::new("frame");
    layout.bindings.push(BindingDesc::new(
        binding,
        DescriptorKind::UniformBuffer,
        StageMask::ALL,
    ));
    layout
}

fn device() -> Option<stormbreaker::vulkan_context::Context> {
    match stormbreaker::vulkan_device::create_device(
        stormbreaker::vulkan_device::DeviceOptions::from_env(c"unhuman-test"),
    ) {
        Ok(context) => Some(context),
        Err(error) => {
            eprintln!("SKIP: no vulkan device: {error}");
            None
        }
    }
}

#[test]
fn a_bind_group_allocates_a_set_against_its_layout() {
    let Some(context) = device() else { return };
    let mut resources = stormbreaker::vulkan_resources::VulkanResources::new(
        &context.device,
        context.memory_properties(),
    )
    .with_uniform_buffer_alignment(context.capabilities.min_uniform_buffer_offset_alignment);

    let buffer = resources
        .create_buffer(BufferDesc::new(256, BufferUsage::UNIFORM), None)
        .expect("a uniform buffer");

    let group = resources
        .create_bind_group(&uniform_layout(0), &[(0, buffer, 0)], &[], &[])
        .expect("a bind group from a declared layout");

    assert!(!group.is_null(), "a real handle, not a null one");

    // Destroying and rebuilding from the same descriptor must land on the same
    // cached layout, or every frame would make a new `vk::DescriptorSetLayout`
    // and the pipeline's layout would stop matching the sets allocated against it.
    let first = resources.descriptor_layout_count();
    let again = resources
        .create_bind_group(&uniform_layout(0), &[(0, buffer, 0)], &[], &[])
        .expect("the same descriptor again");
    assert!(!again.is_null());
    assert_eq!(
        resources.descriptor_layout_count(),
        first,
        "an identical layout is cached, not recreated",
    );
}

/// Writing a binding the layout does not declare is a caller mistake, and Vulkan
/// would accept it -- `vkUpdateDescriptorSets` writes without checking -- leaving
/// the mismatch to surface at draw time as "descriptor type mismatch", naming
/// neither the layout nor the write.
#[test]
fn writing_an_undeclared_binding_is_refused() {
    let Some(context) = device() else { return };
    let mut resources = stormbreaker::vulkan_resources::VulkanResources::new(
        &context.device,
        context.memory_properties(),
    )
    .with_uniform_buffer_alignment(context.capabilities.min_uniform_buffer_offset_alignment);

    let buffer = resources
        .create_buffer(BufferDesc::new(256, BufferUsage::UNIFORM), None)
        .expect("a uniform buffer");

    let error = resources
        .create_bind_group(&uniform_layout(0), &[(7, buffer, 0)], &[], &[])
        .expect_err("binding 7 is not in a layout that declares only 0");
    assert!(
        error.to_string().contains('7'),
        "the error names the binding: {error}",
    );
}

/// A sampled image is a real descriptor kind, and `create_bind_group` does not
/// write those yet. Saying so beats writing a zeroed descriptor that the shader
/// would read as a null image.
#[test]
fn an_unimplemented_descriptor_kind_says_so() {
    let Some(context) = device() else { return };
    let mut resources = stormbreaker::vulkan_resources::VulkanResources::new(
        &context.device,
        context.memory_properties(),
    )
    .with_uniform_buffer_alignment(context.capabilities.min_uniform_buffer_offset_alignment);

    let buffer = resources
        .create_buffer(BufferDesc::new(256, BufferUsage::UNIFORM), None)
        .expect("a uniform buffer");

    let mut layout = uniform_layout(0);
    layout.bindings[0].kind = DescriptorKind::SampledImage;

    let error = resources
        .create_bind_group(&layout, &[(0, buffer, 0)], &[], &[])
        .expect_err("sampled images are not written yet");
    assert!(
        error.to_string().contains("buffer"),
        "the error says what it can write: {error}",
    );
}

/// A handle to a buffer that was destroyed, or never existed.
#[test]
fn a_stale_buffer_handle_is_refused() {
    let Some(context) = device() else { return };
    let mut resources = stormbreaker::vulkan_resources::VulkanResources::new(
        &context.device,
        context.memory_properties(),
    )
    .with_uniform_buffer_alignment(context.capabilities.min_uniform_buffer_offset_alignment);

    let stale = resources
        .create_buffer(BufferDesc::new(256, BufferUsage::UNIFORM), None)
        .expect("a buffer");
    resources.destroy_buffer(stale);

    let error = resources
        .create_bind_group(&uniform_layout(0), &[(0, stale, 0)], &[], &[])
        .expect_err("a destroyed buffer has no memory to point a descriptor at");
    assert!(!error.to_string().is_empty());
}

/// An offset at or past the end of the buffer.
///
/// This one was a real bug. The range used to be computed with `saturating_sub`,
/// which turned an offset of 256 into a range of 0 -- and a zero range is
/// `VUID-VkDescriptorBufferInfo-range-00341`. The call returned `Ok` and the frame
/// was poisoned, which is the worst of both: it looked like it worked and the
/// descriptor was written.
#[test]
fn an_offset_at_or_past_the_end_is_refused() {
    let Some(context) = device() else { return };
    let mut resources = stormbreaker::vulkan_resources::VulkanResources::new(
        &context.device,
        context.memory_properties(),
    )
    .with_uniform_buffer_alignment(context.capabilities.min_uniform_buffer_offset_alignment);

    let buffer = resources
        .create_buffer(BufferDesc::new(256, BufferUsage::UNIFORM), None)
        .expect("a uniform buffer");

    let error = resources
        .create_bind_group(&uniform_layout(0), &[(0, buffer, 256)], &[], &[])
        .expect_err("offset 256 into a 256-byte buffer");
    assert!(
        error.to_string().contains("256"),
        "the error names the offset: {error}",
    );

    // One past the end is equally refused, and differently worded from "way
    // past the end" only by the number -- which is the point.
    assert!(
        resources
            .create_bind_group(&uniform_layout(0), &[(0, buffer, 1 << 20)], &[], &[])
            .is_err(),
        "a wildly out-of-range offset is refused too",
    );

    // And a misaligned offset is refused too, with its own message.
    //
    // This is a separate rule from the one above: a uniform buffer offset must
    // be a multiple of `minUniformBufferOffsetAlignment`. The test originally
    // used offset 255 -- "the last valid byte" -- and validation rejected it as
    // misaligned, which is how the check came to exist.
    let misaligned = resources
        .create_bind_group(&uniform_layout(0), &[(0, buffer, 255)], &[], &[])
        .expect_err("255 is not a multiple of 4");
    assert!(
        misaligned.to_string().contains("Alignment"),
        "the error says which rule: {misaligned}",
    );

    // Aligned, inside the buffer, and leaving a range that is itself a multiple
    // of the alignment: 252 + 4 == 256.
    assert!(
        resources
            .create_bind_group(&uniform_layout(0), &[(0, buffer, 252)], &[], &[])
            .is_ok(),
        "an aligned offset with an aligned range is the ordinary case",
    );
}

/// A sampler, and an image bound with it.
///
/// The descriptor path a renderer spends all its time in, and the one that was
/// missing entirely until now. Two bindings -- a `SampledImage` and a `Sampler`
/// -- which is what Vulkan wants when they are separate.
#[test]
fn an_image_and_a_sampler_bind_together() {
    let Some(context) = device() else { return };
    let mut resources = stormbreaker::vulkan_resources::VulkanResources::new(
        &context.device,
        context.memory_properties(),
    )
    .with_uniform_buffer_alignment(context.capabilities.min_uniform_buffer_offset_alignment);

    let texture = resources
        .create_texture(chassis::TextureDesc::new_2d(
            chassis::Extent {
                width: 16,
                height: 16,
                depth: 1,
            },
            chassis::TextureFormat::Rgba8Unorm,
            chassis::TextureUsage::SAMPLED,
        ))
        .expect("a sampled texture");
    let view = resources
        .create_image_view(&texture)
        .expect("an image view");
    let sampler = resources
        .create_sampler(&chassis::SamplerDesc::trilinear())
        .expect("a sampler");

    let layout = chassis::DescriptorSetLayoutDesc::new("material")
        .with(BindingDesc::new(
            0,
            DescriptorKind::SampledImage,
            StageMask::ALL,
        ))
        .with(BindingDesc::new(1, DescriptorKind::Sampler, StageMask::ALL));

    let group = resources
        .create_bind_group(&layout, &[], &[(0, view)], &[(1, sampler)])
        .expect("an image bind group");
    assert!(!group.is_null());
    assert!(!resources
        .resolve_image_view(view)
        .expect("the view resolves")
        .is_null());
    assert!(!resources
        .resolve_sampler(sampler)
        .expect("the sampler resolves")
        .is_null());

    resources.destroy_sampler(sampler);
    assert!(
        resources.resolve_sampler(sampler).is_err(),
        "a destroyed sampler does not resolve -- a handle that resolved to a \\
         destroyed object is worse than one that does not resolve",
    );
}

/// `CombinedImageSampler` is one binding on Vulkan and **does not exist on
/// wgpu**.
///
/// WebGPU has no combined descriptor, so `BindingResource` has no variant for it
/// and the RHI's most convenient kind cannot be expressed. Vulkan has
/// `COMBINED_IMAGE_SAMPLER` and uses it. The parity test says so from both sides.
#[test]
fn a_combined_image_sampler_needs_both_halves() {
    let Some(context) = device() else { return };
    let mut resources = stormbreaker::vulkan_resources::VulkanResources::new(
        &context.device,
        context.memory_properties(),
    )
    .with_uniform_buffer_alignment(context.capabilities.min_uniform_buffer_offset_alignment);

    let texture = resources
        .create_texture(chassis::TextureDesc::new_2d(
            chassis::Extent {
                width: 16,
                height: 16,
                depth: 1,
            },
            chassis::TextureFormat::Rgba8Unorm,
            chassis::TextureUsage::SAMPLED,
        ))
        .expect("a sampled texture");
    let view = resources
        .create_image_view(&texture)
        .expect("an image view");
    let sampler = resources
        .create_sampler(&chassis::SamplerDesc::clamped())
        .expect("a sampler");

    let layout = chassis::DescriptorSetLayoutDesc::new("material").with(BindingDesc::new(
        0,
        DescriptorKind::CombinedImageSampler,
        StageMask::ALL,
    ));

    // The image alone is not enough: Vulkan's combined descriptor needs both
    // halves, and a null sampler reads as black rather than as an error.
    let error = resources
        .create_bind_group(&layout, &[], &[(0, view)], &[])
        .expect_err("a combined descriptor with no sampler");
    assert!(error.to_string().contains("sampler"), "{error}");

    // With both, it is one binding.
    let group = resources
        .create_bind_group(&layout, &[], &[(0, view)], &[(0, sampler)])
        .expect("a combined bind group");
    assert!(!group.is_null());
}
