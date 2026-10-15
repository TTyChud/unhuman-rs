//! `DescriptorSetLayoutDesc`'s two kind-sensitive operations, `is_compatible`
//! and `hash`.
//!
//! They exist for the same job -- deciding whether two declarations can share one
//! layout object -- and the comment on `hash` says they have to agree: one
//! dedupes, the other is the cache key for the same deduping, and which of the
//! two a backend believes decides whether a bind group and a pipeline end up with
//! the same object. So both are asserted here, on the pair of kinds a renderer is
//! most likely to confuse.

use chassis::{BindingDesc, DescriptorKind, DescriptorSetLayoutDesc, StageMask};

fn layout(kind: DescriptorKind) -> DescriptorSetLayoutDesc {
    DescriptorSetLayoutDesc::new("frame").with(BindingDesc::new(0, kind, StageMask::ALL))
}

/// A depth texture read is its own kind of binding, not a `SampledImage`.
///
/// wgpu gives a depth view `TextureSampleType::Depth`, so a layout that called it
/// a sampled image would build a bind group the view cannot be put in. `chassis`
/// only has to keep the two apart -- and keep `hash` agreeing with
/// `is_compatible`, or one backend dedupes a layout the other does not.
#[test]
fn a_depth_read_is_its_own_layout() {
    let image = layout(DescriptorKind::SampledImage);
    let depth = layout(DescriptorKind::SampledDepth);

    assert_ne!(DescriptorKind::SampledImage, DescriptorKind::SampledDepth);
    assert!(!depth.is_compatible(&image), "an image layout took a depth one");
    assert_ne!(
        depth.hash(),
        image.hash(),
        "hash and is_compatible disagree about the kind",
    );

    // The same kind still dedupes, and the label is still excluded -- the two
    // halves of what `hash` documents.
    assert!(depth.is_compatible(&layout(DescriptorKind::SampledDepth)));
    assert_eq!(depth.hash(), layout(DescriptorKind::SampledDepth).hash());

    let mut renamed = layout(DescriptorKind::SampledDepth);
    renamed.label = "shadow".into();
    assert!(depth.is_compatible(&renamed));
    assert_eq!(depth.hash(), renamed.hash());
}
