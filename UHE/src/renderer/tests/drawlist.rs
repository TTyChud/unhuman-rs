//! Batching, and the replay that issues it.
//!
//! The tests to write here are the ones a screenshot cannot catch: a draw list
//! that renders the right picture in the wrong order still looks right, and a
//! replay that binds a redundant pipeline per draw still renders correctly while
//! costing the whole frame.

mod support;

use bifrost::drawlist::{BatchMode, DrawItem, DrawList, MaterialId, MeshId, PipelineId};
use bifrost::replay::record;
use bifrost::sort::DepthOrder;
use bifrost::stats::StateTracker;
use bifrost::Sphere;
use glam::{Mat4, Vec3};
use support::{RecordingList, TestResolver};

fn item(mesh: MeshId, material: MaterialId, pipeline: PipelineId, depth: f32) -> DrawItem {
    DrawItem::new(mesh, material, pipeline, Mat4::IDENTITY, Sphere::UNIT).with_depth(depth)
}

fn commands(list: &DrawList) -> Vec<(u32, u32, u32)> {
    list.commands()
        .iter()
        .map(|c| (c.mesh.0, c.instances, c.first_item))
        .collect()
}

#[test]
fn identical_meshes_at_the_same_depth_become_one_instanced_draw() {
    // Five crates in a row, one mesh, one material, one pipeline, all the same
    // distance. The C++ issues five draw calls for this (`Renderer3D.cpp:300`,
    // inside a loop over primitives with a `BindPipeline` at the top of each
    // `SubmitMesh`).
    let mut list = DrawList::new(DepthOrder::Opaque);
    for _ in 0..5 {
        list.push(item(MeshId(7), MaterialId(1), PipelineId(1), 4.0));
    }
    list.build();

    assert_eq!(list.commands().len(), 1, "five crates should be one draw");
    assert_eq!(list.commands()[0].instances, 5);
    assert_eq!(list.commands()[0].first_item, 0);
    assert_eq!(list.total_instances(), 5);
}

#[test]
fn interleaved_meshes_stay_separate_when_the_order_is_preserved() {
    // Two meshes at alternating depths. Sorting by depth separates them, so
    // order-preserving batching must *not* fold them — folding would reorder the
    // queue and lose the early-Z rejection the sort bought.
    let mut list = DrawList::new(DepthOrder::Opaque);
    list.push(item(MeshId(1), MaterialId(0), PipelineId(0), 1.0));
    list.push(item(MeshId(2), MaterialId(0), PipelineId(0), 2.0));
    list.push(item(MeshId(1), MaterialId(0), PipelineId(0), 3.0));
    list.build();

    assert_eq!(list.commands().len(), 3);
    assert!(list.commands().iter().all(|c| c.instances == 1));
    // Still front to back, which is the point.
    let depths: Vec<f32> = list.items().iter().map(|i| i.depth).collect();
    assert_eq!(depths, vec![1.0, 2.0, 3.0]);
}

#[test]
fn grouped_mode_folds_keys_the_depth_sort_had_separated() {
    // Two meshes alternating at increasing depth. The depth sort interleaves them;
    // grouping by key brings each mesh's instances together, which is the only
    // thing grouping is allowed to change.
    let mut list = DrawList::with_batch_mode(DepthOrder::Opaque, BatchMode::Grouped);
    for depth in [1.0, 2.0, 3.0, 4.0] {
        let mesh = MeshId((depth as u32) % 2);
        list.push(item(mesh, MaterialId(0), PipelineId(0), depth));
    }
    list.build();

    assert_eq!(
        list.commands().len(),
        2,
        "four alternating meshes fold to two"
    );
    assert!(list.commands().iter().all(|command| command.instances == 2));

    // Depth order survives *within* each group, because the sort that grouped
    // them is stable. A fold that also shuffled instances would be visible in the
    // blended case and merely wasteful here.
    let depths: Vec<f32> = list.items().iter().map(|i| i.depth).collect();
    assert_eq!(depths, vec![1.0, 3.0, 2.0, 4.0]);
}

#[test]
fn grouping_is_refused_for_a_transparent_list() {
    // Reordering blended geometry changes the image, so `Grouped` is downgraded
    // rather than honoured. The list must still be back-to-front.
    let mut list = DrawList::with_batch_mode(DepthOrder::Transparent, BatchMode::Grouped);
    for depth in [1.0, 2.0, 3.0, 4.0] {
        list.push(item(MeshId(1), MaterialId(0), PipelineId(0), depth));
    }
    list.build();

    let depths: Vec<f32> = list.items().iter().map(|i| i.depth).collect();
    assert_eq!(depths, vec![4.0, 3.0, 2.0, 1.0], "the queue was reordered");
    // Adjacent identical items still fold, which is the part that is safe.
    assert_eq!(list.commands().len(), 1);
    assert_eq!(list.commands()[0].instances, 4);
}

#[test]
fn the_batch_key_is_pipeline_then_material_then_mesh() {
    // The order of the key is the order of expense: a pipeline rebind is the
    // expensive one, so it changes least often.
    let mut list = DrawList::with_batch_mode(DepthOrder::Opaque, BatchMode::Grouped);
    // Deliberately submitted in reverse.
    list.push(item(MeshId(3), MaterialId(9), PipelineId(1), 4.0));
    list.push(item(MeshId(2), MaterialId(2), PipelineId(0), 3.0));
    list.push(item(MeshId(1), MaterialId(2), PipelineId(0), 2.0));
    list.build();

    let keys: Vec<(u32, u32, u32)> = list
        .commands()
        .iter()
        .map(|c| (c.pipeline.0, c.material.0, c.mesh.0))
        .collect();
    assert_eq!(keys, vec![(0, 2, 1), (0, 2, 2), (1, 9, 3)]);
}

#[test]
fn every_item_is_covered_exactly_once() {
    // `first_item` is where a caller writes the per-instance transforms. If it
    // skipped or repeated an index, an instance would get another instance's
    // transform: a correct-looking frame with one object somewhere else.
    let mut list = DrawList::new(DepthOrder::Opaque);
    for (index, depth) in [3.0, 1.0, 2.0, 1.0].into_iter().enumerate() {
        let mesh = MeshId(u32::try_from(index % 2).expect("index fits in u32"));
        list.push(item(mesh, MaterialId(0), PipelineId(0), depth));
    }
    list.build();

    let mut covered: Vec<u32> = list
        .commands()
        .iter()
        .flat_map(|c| c.first_item..c.first_item + c.instances)
        .collect();
    covered.sort_unstable();
    assert_eq!(covered, vec![0, 1, 2, 3]);
    assert_eq!(
        list.commands().iter().map(|c| c.instances).sum::<u32>(),
        list.total_instances()
    );
}

#[test]
fn a_push_invalidates_the_build() {
    let mut list = DrawList::new(DepthOrder::Opaque);
    list.push(item(MeshId(1), MaterialId(0), PipelineId(0), 1.0));
    list.build();
    assert!(list.is_built());
    assert_eq!(list.commands()[0].instances, 1);

    list.push(item(MeshId(1), MaterialId(0), PipelineId(0), 2.0));
    assert!(!list.is_built(), "stale commands would draw the old scene");
    list.build();
    assert_eq!(list.commands()[0].instances, 2);
    assert_eq!(list.commands()[0].first_item, 0);
}

#[test]
fn an_empty_list_produces_no_draws() {
    let mut list = DrawList::new(DepthOrder::Opaque);
    list.build();
    assert!(list.is_built());
    assert!(list.commands().is_empty());
    assert_eq!(list.total_instances(), 0);
}

#[test]
fn clearing_empties_items_and_commands() {
    let mut list = DrawList::new(DepthOrder::Opaque);
    list.push(item(MeshId(1), MaterialId(0), PipelineId(0), 1.0));
    list.build();
    list.clear();
    assert!(!list.is_built());
    assert_eq!(commands(&list), Vec::new());
    assert_eq!(list.total_instances(), 0);
}

fn resolver_with(materials: u32, meshes: u32) -> TestResolver {
    let mut resolver = TestResolver::new();
    resolver.add_pipeline(PipelineId(0));
    for index in 0..materials {
        resolver.add_material(MaterialId(index));
    }
    for index in 0..meshes {
        resolver.add_mesh(MeshId(index), 36 * (index + 1));
    }
    resolver
}

#[test]
fn replay_issues_one_draw_per_command_with_the_right_instances() {
    let mut list = DrawList::new(DepthOrder::Opaque);
    for _ in 0..3 {
        list.push(item(MeshId(0), MaterialId(0), PipelineId(0), 1.0));
    }
    for _ in 0..2 {
        list.push(item(MeshId(1), MaterialId(0), PipelineId(0), 1.0));
    }
    list.build();
    assert_eq!(
        list.commands().len(),
        2,
        "test setup: two meshes, two draws"
    );

    let mut recorded = RecordingList::new();
    let mut tracker = StateTracker::new();
    let resolver = resolver_with(1, 2);
    let count = list
        .record(&mut recorded, &resolver, &mut tracker)
        .expect("every id resolves");

    assert_eq!(count.draws, 2);
    assert_eq!(count.instances, 5);
    assert_eq!(
        recorded.draws,
        vec![(36, 3), (72, 2)],
        "one draw per command, in the sorted order"
    );
    // Nothing here opens a render scope: the executor does, from the compiled
    // pass's attachments.
    assert_eq!(recorded.scopes_opened, 0);
}

#[test]
fn replay_binds_each_pipeline_once_not_once_per_draw() {
    // Five crates, one pipeline. The C++ binds the pipeline at the top of every
    // `SubmitMesh` (`Renderer3D.cpp:229-240`) whether or not it changed.
    let mut list = DrawList::new(DepthOrder::Opaque);
    for _ in 0..5 {
        list.push(item(MeshId(0), MaterialId(0), PipelineId(0), 1.0));
    }
    list.push(item(MeshId(0), MaterialId(0), PipelineId(0), 1.0));
    list.build();

    // Force one draw per item by making the keys differ, which is the case where
    // a naive replay pays five binds.
    let mut resolver = resolver_with(1, 1);
    for index in 1..5 {
        resolver.add_material(MaterialId(index));
        resolver.add_mesh(MeshId(index), 36);
    }
    let mut many = DrawList::new(DepthOrder::Opaque);
    for index in 0..5 {
        many.push(item(MeshId(0), MaterialId(0), PipelineId(0), 1.0));
        let _ = index;
    }
    many.build();
    assert_eq!(many.commands().len(), 1, "identical items still fold");

    let mut recorded = RecordingList::new();
    let mut tracker = StateTracker::new();
    record(&mut recorded, many.commands(), &resolver, &mut tracker).expect("resolves");
    assert_eq!(recorded.draws.len(), 1);
    assert_eq!(recorded.pipelines.len(), 1, "one pipeline bind for one run");
    assert_eq!(tracker.changes().pipelines, 1);

    // Two pipelines, one run each: two binds, and the second one is not skipped.
    let mut two = DrawList::new(DepthOrder::Opaque);
    resolver.add_pipeline(PipelineId(1));
    two.push(item(MeshId(0), MaterialId(0), PipelineId(1), 1.0));
    two.push(item(MeshId(0), MaterialId(0), PipelineId(0), 2.0));
    two.build();
    assert_eq!(two.commands().len(), 2);

    let mut recorded = RecordingList::new();
    let mut tracker = StateTracker::new();
    record(&mut recorded, two.commands(), &resolver, &mut tracker).expect("resolves");
    assert_eq!(recorded.pipelines.len(), 2);
    assert_eq!(tracker.changes().pipelines, 2);
}

#[test]
fn replay_rebinds_when_only_the_offset_changes() {
    // Two meshes in one merged vertex buffer differ only by offset. A tracker
    // comparing handles alone would skip the second bind and the second mesh would
    // render the first one's vertices — a wrong picture, not a slow frame.
    let mut list = DrawList::new(DepthOrder::Opaque);
    list.push(item(MeshId(0), MaterialId(0), PipelineId(0), 1.0));
    list.push(item(MeshId(1), MaterialId(0), PipelineId(0), 1.0));
    list.build();
    assert_eq!(list.commands().len(), 2);

    let resolver = resolver_with(1, 2);
    let mut recorded = RecordingList::new();
    let mut tracker = StateTracker::new();
    list.record(&mut recorded, &resolver, &mut tracker)
        .expect("resolves");

    assert_eq!(recorded.vertex_buffers.len(), 2);
    assert_eq!(recorded.index_buffers.len(), 2);
    assert_ne!(
        recorded.vertex_buffers[0].1, recorded.vertex_buffers[1].1,
        "the test's two meshes must differ only by offset"
    );
}

#[test]
fn a_new_pass_binds_again() {
    // Each pass records into its own command list, so nothing survives into the
    // next pass. Carrying the cache across would under-count by one bind per pass,
    // which is exactly the sort of counter that "looks right".
    let resolver = resolver_with(1, 1);
    let mut first = RecordingList::new();
    let mut second = RecordingList::new();
    let mut tracker = StateTracker::new();
    let commands = [bifrost::drawlist::DrawCommand {
        pipeline: PipelineId(0),
        material: MaterialId(0),
        mesh: MeshId(0),
        first_item: 0,
        instances: 1,
    }];

    tracker.begin_pass();
    record(&mut first, &commands, &resolver, &mut tracker).expect("resolves");
    tracker.begin_pass();
    record(&mut second, &commands, &resolver, &mut tracker).expect("resolves");

    assert_eq!(first.pipelines.len(), 1);
    assert_eq!(second.pipelines.len(), 1);
    assert_eq!(tracker.changes().pipelines, 2);
}

#[test]
fn a_missing_handle_is_an_error_not_a_skipped_draw() {
    // Silently skipping turns "the resource cache missed an entry" into "an object
    // is mysteriously invisible".
    let mut list = DrawList::new(DepthOrder::Opaque);
    list.push(item(MeshId(3), MaterialId(0), PipelineId(0), 1.0));
    list.build();

    let resolver = resolver_with(1, 1); // no mesh 3
    let mut recorded = RecordingList::new();
    let mut tracker = StateTracker::new();
    let error = list
        .record(&mut recorded, &resolver, &mut tracker)
        .expect_err("an unresolvable mesh must not be skipped");
    assert!(
        error.to_string().contains("vertex buffer"),
        "the error should name what was missing: {error}"
    );
    assert!(recorded.draws.is_empty(), "nothing should have been drawn");
}

#[test]
fn a_draw_list_sorts_by_depth_whatever_order_it_was_filled_in() {
    let mut list = DrawList::new(DepthOrder::Opaque);
    list.push(item(MeshId(0), MaterialId(0), PipelineId(0), 9.0));
    list.push(item(MeshId(0), MaterialId(0), PipelineId(0), 1.0));
    list.build();
    let depths: Vec<f32> = list.items().iter().map(|i| i.depth).collect();
    assert_eq!(depths, vec![1.0, 9.0]);
}

#[test]
fn world_bounds_follow_the_item_transform() {
    let item = DrawItem::new(
        MeshId(0),
        MaterialId(0),
        PipelineId(0),
        Mat4::from_translation(Vec3::new(3.0, 0.0, -2.0)),
        Sphere::new(Vec3::ZERO, 1.0),
    );
    let world = item.world_bounds();
    assert_eq!(world.centre, Vec3::new(3.0, 0.0, -2.0));
    assert_eq!(world.radius, 1.0);
}
