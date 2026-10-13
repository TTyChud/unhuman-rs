use stormgraph::{
    ErrorKind, ImageState, PassType, Queue, RenderGraph, ResourceAccess,
};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// A deferred-shaped frame: gbuffer -> lighting -> swapchain.
fn deferred_frame() -> RenderGraph {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let gbuffer = graph.create_texture("gbuffer", ImageState::Undefined, ImageState::ShaderRead);
    let depth = graph.create_texture("depth", ImageState::Undefined, ImageState::ShaderRead);

    graph
        .pass("gbuffer")
        .kind(PassType::Graphics)
        .color(gbuffer, BLACK)
        .depth(depth, 1.0);

    graph
        .pass("lighting")
        .queue(Queue::Graphics)
        .color(swapchain, BLACK)
        .read_texture(gbuffer)
        .read_texture(depth);

    graph
}

#[test]
fn a_simple_frame_compiles_in_declaration_order() {
    let compiled = deferred_frame().compile().expect("frame should compile");
    let names: Vec<&str> = compiled.passes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["gbuffer", "lighting"]);
}

#[test]
fn barriers_are_synthesised_per_pass() {
    let compiled = deferred_frame().compile().unwrap();

    let gbuffer_pass = compiled.find("gbuffer").unwrap();
    let states: Vec<ImageState> = gbuffer_pass.pre_barriers.iter().map(|b| b.dst_state).collect();
    assert_eq!(
        states,
        vec![ImageState::ColorAttachment, ImageState::DepthAttachment],
        "each attachment needs its own transition"
    );

    let lighting = compiled.find("lighting").unwrap();
    let targets: Vec<ImageState> = lighting.pre_barriers.iter().map(|b| b.dst_state).collect();
    assert_eq!(
        targets,
        vec![
            ImageState::ShaderRead,
            ImageState::ShaderRead,
            ImageState::ColorAttachment,
        ],
        "lighting samples the gbuffer, then writes the swapchain"
    );
}

#[test]
fn leaving_undefined_needs_no_source_scope() {
    let compiled = deferred_frame().compile().unwrap();
    let first = &compiled.find("gbuffer").unwrap().pre_barriers[0];

    assert_eq!(first.src_state, ImageState::Undefined);
    // Nothing to wait for when the previous contents are undefined.
    assert_eq!(first.src_stage, stormgraph::Stage::TopOfPipe);
    assert_eq!(first.src_access, stormgraph::Access::None);
    assert_eq!(first.dst_access, stormgraph::Access::ColorWrite);
}

#[test]
fn the_swapchain_reaches_present_after_its_last_writer() {
    let compiled = deferred_frame().compile().unwrap();

    // Lighting is the last pass to write the swapchain, so the transition to
    // Present is its post-barrier.
    let lighting = compiled.find("lighting").unwrap();
    assert_eq!(lighting.post_barriers.len(), 1);
    assert_eq!(lighting.post_barriers[0].dst_state, ImageState::Present);
    assert_eq!(lighting.post_barriers[0].src_state, ImageState::ColorAttachment);

    // No earlier pass carries it.
    assert!(compiled.find("gbuffer").unwrap().post_barriers.is_empty());
}

#[test]
fn each_pass_carries_only_its_own_pre_barriers() {
    let compiled = deferred_frame().compile().unwrap();

    // This is the property that makes parallel recording safe: a pass's command
    // list is self-contained, so no ordering constraint exists between the
    // lists and no post-barrier has to be pushed back into a producer's list.
    for pass in &compiled.passes {
        let own: Vec<ImageState> = pass.pre_barriers.iter().map(|b| b.dst_state).collect();
        match pass.name.as_str() {
            "gbuffer" => assert_eq!(
                own,
                vec![ImageState::ColorAttachment, ImageState::DepthAttachment]
            ),
            "lighting" => assert!(own.contains(&ImageState::ShaderRead)),
            other => panic!("unexpected pass {other}"),
        }
    }
}

#[test]
fn a_pass_writing_twice_produces_two_versions() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let pingpong = graph.create_texture("pingpong", ImageState::Undefined, ImageState::Storage);

    graph.pass("first").write_texture(pingpong, ImageState::Storage);
    graph.pass("second").write_texture(pingpong, ImageState::Storage);
    graph.pass("present").color(swapchain, BLACK).read_texture(pingpong);

    let slot = graph.resources().texture(pingpong).unwrap();
    assert_eq!(slot.version_log.len(), 3, "v0 plus two writes");

    let compiled = graph.compile().unwrap();
    let names: Vec<&str> = compiled.passes.iter().map(|p| p.name.as_str()).collect();
    // `first` writes v1, which nothing reads: `consume` binds to v2, so `first`
    // is dead and culled. Two writes to one texture remain two distinct edges
    // rather than collapsing into one.
    assert_eq!(names, vec!["second", "present"]);
}

#[test]
fn dead_passes_are_culled() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let used = graph.create_texture("used", ImageState::Undefined, ImageState::ShaderRead);
    let orphan = graph.create_texture("orphan", ImageState::Undefined, ImageState::ShaderRead);

    graph.pass("orphan_producer").write_texture(orphan, ImageState::Storage);
    graph.pass("real_producer").write_texture(used, ImageState::Storage);
    graph
        .pass("present")
        .color(swapchain, BLACK)
        .read_texture(used);

    let compiled = graph.compile().unwrap();
    let names: Vec<&str> = compiled.passes.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(names, vec!["real_producer", "present"]);
    assert!(
        !names.contains(&"orphan_producer"),
        "a pass nothing consumes was kept"
    );
}

#[test]
fn a_frame_with_no_imports_compiles_to_nothing() {
    let mut graph = RenderGraph::new();
    let scratch = graph.create_texture("scratch", ImageState::Undefined, ImageState::Storage);
    graph.pass("pointless").write_texture(scratch, ImageState::Storage);

    let compiled = graph.compile().unwrap();
    assert!(compiled.is_empty(), "unobservable work should be culled");
}

#[test]
fn lifetimes_span_first_to_last_use() {
    let compiled = deferred_frame().compile().unwrap();

    let gbuffer_life = compiled
        .lifetimes
        .iter()
        .find(|l| l.last_use > l.first_use)
        .expect("a resource used across two passes");
    assert_eq!(gbuffer_life.first_use, 0);
    assert_eq!(gbuffer_life.last_use, 1);
}

#[test]
fn declaration_order_is_the_execution_order() {
    let compiled = deferred_frame().compile().unwrap();
    for (position, pass) in compiled.passes.iter().enumerate() {
        assert_eq!(pass.index as usize, position, "order must match declaration");
    }
}

/// Cycles are unrepresentable, by construction.
///
/// A read binds to the version current *at declaration time*, and a write
/// allocates the next version. So a pass can never depend on a version that a
/// later pass produces: declaration order is always a valid topological order.
/// The two passes below look cyclic but genuinely are not — each reads the
/// pre-frame version its neighbour never wrote.
#[test]
fn a_cycle_cannot_be_declared() {
    let mut graph = RenderGraph::new();
    let a = graph.create_texture("a", ImageState::Undefined, ImageState::Storage);
    let b = graph.create_texture("b", ImageState::Undefined, ImageState::Storage);

    graph.pass("writes_a").write_texture(a, ImageState::Storage).read_texture(b);
    graph.pass("writes_b").write_texture(b, ImageState::Storage).read_texture(a);

    // No cycle error, and no dependency either: both reads bind to v0.
    assert!(graph.validate().is_empty(), "{:?}", graph.validate());
    assert!(graph.compile().is_ok());
}

#[test]
fn reading_before_writing_binds_to_the_pre_frame_version() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let late = graph.create_texture("late", ImageState::Undefined, ImageState::Storage);

    // `early` reads before `late_producer` writes, so it reads v0 and the two
    // passes have no edge between them.
    graph.pass("early").read_texture(late);
    graph.pass("late_producer").write_texture(late, ImageState::Storage);
    graph.pass("present").color(swapchain, BLACK).read_texture(late);

    assert_eq!(graph.passes()[0].reads, &[ResourceAccess::Texture(late, 0)]);
    assert_eq!(graph.passes()[2].reads, &[ResourceAccess::Texture(late, 1)]);

    let compiled = graph.compile().unwrap();
    let names: Vec<&str> = compiled.passes.iter().map(|p| p.name.as_str()).collect();
    // Declaration order is preserved, and `early` is culled because nothing
    // observable depends on a read of v0.
    assert_eq!(names, vec!["late_producer", "present"]);
}

#[test]
fn an_undeclared_attachment_is_reported() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);

    // Declares a colour attachment but never writes it.
    let pass = graph.pass("bad");
    let _ = pass.color(swapchain, BLACK);

    let mut passes = graph.passes().to_vec();
    passes[0].writes.clear();

    let errors = stormgraph::validate(&passes, graph.resources());
    assert!(
        errors.iter().any(|e| e.kind == ErrorKind::UndeclaredAttachment),
        "got {errors:?}"
    );
}

#[test]
fn a_double_write_is_reported() {
    let mut graph = RenderGraph::new();
    let target = graph.create_texture("target", ImageState::Undefined, ImageState::Storage);
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);

    graph
        .pass("bad")
        .write_texture(target, ImageState::Storage)
        .write_texture(target, ImageState::Storage);
    graph.pass("present").color(swapchain, BLACK).read_texture(target);

    let errors = graph.validate();
    assert!(
        errors.iter().any(|e| e.kind == ErrorKind::DoubleWrite),
        "got {errors:?}"
    );
}

#[test]
fn an_unsubmitted_pass_is_reported() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    graph.pass("").color(swapchain, BLACK);

    let errors = graph.validate();
    assert!(
        errors.iter().any(|e| e.kind == ErrorKind::UnsubmittedPass),
        "got {errors:?}"
    );
}

#[test]
fn reading_and_writing_one_texture_in_one_pass_is_reported() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let risky = graph.create_texture("risky", ImageState::Undefined, ImageState::Storage);

    graph
        .pass("ambiguous")
        .read_texture(risky)
        .write_texture(risky, ImageState::Storage);
    graph.pass("present").color(swapchain, BLACK);

    let errors = graph.validate();
    assert!(
        errors.iter().any(|e| e.kind == ErrorKind::PassStateConflict),
        "got {errors:?}"
    );
}

#[test]
fn errors_name_the_pass_and_resource() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let target = graph.create_texture("target", ImageState::Undefined, ImageState::Storage);

    graph
        .pass("writer")
        .write_texture(target, ImageState::Storage)
        .write_texture(target, ImageState::Storage);
    graph.pass("present").color(swapchain, BLACK);

    let errors = graph.validate();
    let error = errors.first().expect("double write");
    assert_eq!(error.pass, "writer");
    assert!(error.resource.starts_with("target"), "{}", error.resource);
    assert!(error.to_string().contains("DoubleWrite"));
}

#[test]
fn validation_collects_every_error_at_once() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let target = graph.create_texture("target", ImageState::Undefined, ImageState::Storage);

    graph
        .pass("twice")
        .write_texture(target, ImageState::Storage)
        .write_texture(target, ImageState::Storage);
    graph.pass("").write_texture(target, ImageState::Storage);
    graph.pass("present").color(swapchain, BLACK);

    let errors = graph.validate();
    let unsubmitted = errors
        .iter()
        .filter(|e| e.kind == ErrorKind::UnsubmittedPass)
        .count();
    let double = errors.iter().filter(|e| e.kind == ErrorKind::DoubleWrite).count();

    assert!(unsubmitted >= 1, "{errors:?}");
    assert!(double >= 1, "{errors:?}");
}

#[test]
fn dump_reports_resources_and_passes() {
    let dump = deferred_frame().dump();

    assert!(dump.contains("swapchain"));
    assert!(dump.contains("gbuffer"));
    assert!(dump.contains("passes:"));
    assert!(dump.contains("[0] gbuffer"));
    assert!(dump.contains("[1] lighting"));
}

#[test]
fn reset_empties_the_graph() {
    let mut graph = deferred_frame();
    graph.reset();

    assert_eq!(graph.resources().texture_count(), 0);
    assert!(graph.passes().is_empty());
    assert!(graph.compile().unwrap().is_empty());
}

#[test]
fn declaration_binds_reads_to_the_version_current_at_that_moment() {
    let mut graph = RenderGraph::new();
    let swapchain = graph.import_texture("swapchain", ImageState::Undefined, ImageState::Present);
    let pingpong = graph.create_texture("pingpong", ImageState::Undefined, ImageState::Storage);

    graph.pass("write_v1").write_texture(pingpong, ImageState::Storage);
    graph.pass("write_v2").write_texture(pingpong, ImageState::Storage);
    graph
        .pass("consume")
        .color(swapchain, BLACK)
        .read_texture(pingpong);

    let reads = &graph.passes()[2].reads;
    assert_eq!(
        reads,
        &[ResourceAccess::Texture(pingpong, 2)],
        "the read must bind to the newest version, not the oldest"
    );
}