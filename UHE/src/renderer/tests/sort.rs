//! Sort orderings.
//!
//! These test the two orders and the tie-break, because both are decisions that
//! look arbitrary until they are wrong: front-to-back and back-to-front differ by
//! one word, and a tie-break that sorts by nothing is stable and therefore
//! accidentally correct for ties and accidentally *incorrect* for batching.

use bifrost::drawlist::{DrawItem, MaterialId, MeshId, PipelineId};
use bifrost::sort::{sort, sort_opaque, sort_transparent, DepthOrder};
use bifrost::Sphere;
use glam::{Mat4, Vec3};

/// An item whose only interesting field is its depth, and whose transform is a
/// translation on X so "equal depth and equal key" is still distinguishable.
fn item(depth: f32, x: f32) -> DrawItem {
    DrawItem::new(
        MeshId(0),
        MaterialId(0),
        PipelineId(0),
        Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
        Sphere::UNIT,
    )
    .with_depth(depth)
}

fn depths(items: &[DrawItem]) -> Vec<f32> {
    items.iter().map(|item| item.depth).collect()
}

fn translations_x(items: &[DrawItem]) -> Vec<f32> {
    items.iter().map(|item| item.transform.w_axis.x).collect()
}

#[test]
fn opaque_draws_nearest_first() {
    let mut items = vec![item(30.0, 0.0), item(10.0, 1.0), item(20.0, 2.0)];
    sort_opaque(&mut items);
    assert_eq!(depths(&items), vec![10.0, 20.0, 30.0]);
}

#[test]
fn transparent_draws_farthest_first() {
    let mut items = vec![item(30.0, 0.0), item(10.0, 1.0), item(20.0, 2.0)];
    sort_transparent(&mut items);
    assert_eq!(depths(&items), vec![30.0, 20.0, 10.0]);
}

#[test]
fn the_two_orders_are_opposites() {
    let items = vec![item(3.0, 0.0), item(1.0, 1.0), item(2.0, 2.0)];

    let mut opaque = items.clone();
    sort_opaque(&mut opaque);
    let mut transparent = items.clone();
    sort_transparent(&mut transparent);

    let forward: Vec<f32> = opaque.iter().map(|i| i.depth).collect();
    let backward: Vec<f32> = transparent.iter().map(|i| i.depth).collect();
    let mut reversed = forward.clone();
    reversed.reverse();
    assert_eq!(
        backward, reversed,
        "one order must be the reverse of the other"
    );
}

#[test]
fn ties_break_on_the_batch_key_not_on_input_order() {
    // Three objects at one depth with three pipelines. Whatever the submission
    // order, the pipeline runs are contiguous and ordered, so the batcher can
    // fold them.
    let items: Vec<DrawItem> = [2u32, 0, 1]
        .into_iter()
        .map(|pipeline| {
            DrawItem::new(
                MeshId(0),
                MaterialId(0),
                PipelineId(pipeline),
                Mat4::IDENTITY,
                Sphere::UNIT,
            )
            .with_depth(5.0)
        })
        .collect();

    let mut sorted = items;
    sort_opaque(&mut sorted);
    let pipelines: Vec<u32> = sorted.iter().map(|i| i.pipeline.0).collect();
    assert_eq!(pipelines, vec![0, 1, 2]);
}

#[test]
fn items_equal_in_every_key_keep_their_order() {
    // Stability is load-bearing: instances are per-instance data, so a sort that
    // reshuffled indistinguishable items would move an object's transform onto a
    // different object.
    let mut items = vec![item(1.0, 0.0), item(1.0, 1.0), item(1.0, 2.0)];
    sort(&mut items, DepthOrder::Transparent);
    assert_eq!(translations_x(&items), vec![0.0, 1.0, 2.0]);
}

#[test]
fn a_nan_depth_sorts_without_panicking() {
    // `partial_cmp` returns `None` for NaN, and a `sort_by` comparator that does
    // so leaves the order unspecified — which on a frame loop means a different
    // order every frame for reasons nobody can see. `total_cmp` keeps it total.
    let mut items = vec![item(f32::NAN, 0.0), item(1.0, 1.0), item(2.0, 2.0)];
    sort_opaque(&mut items);
    assert_eq!(items.len(), 3);
    // Whatever the order, a finite depth must have survived intact.
    assert!(items.iter().any(|i| i.depth == 1.0));
    assert!(items.iter().any(|i| i.depth == 2.0));
}

#[test]
fn only_transparency_is_order_dependent() {
    assert!(!DepthOrder::Opaque.is_order_dependent());
    assert!(DepthOrder::Transparent.is_order_dependent());
}
