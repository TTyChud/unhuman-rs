//! The wgpu executor, on a real compiled frame.
//!
//! The graph is `chassis::graph`, built once and used by both backends. That is
//! the claim being tested: one frame description, two very different executors.

use chassis::graph as rg;
use chassis::graph::{NoRecording, PassRecorder, PassScope};
use chassis::CommandList;
use stormglass::wgpu_device::Adapter;
use stormglass::wgpu_executor::{ExecutorStats, WgpuExecutor};
use stormglass::wgpu_resources::WgpuResources;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;

/// A frame the executor can run, built with the shared graph.
///
/// Two declared passes, which compile to two. The frame's *final* transition --
/// the image to `TransferSrc` for readback -- is a post-barrier on the last
/// writer, not a pass of its own. That is the difference between a barrier and a
/// pass, and it is why the count here is 2 and not 3.
fn two_pass_frame() -> rg::RenderGraph {
    let mut graph = rg::RenderGraph::new();
    let target = graph.import_texture(
        "target",
        rg::ImageState::Undefined,
        rg::ImageState::TransferSrc,
    );
    graph
        .pass("draw")
        .kind(rg::PassType::Graphics)
        .color_load(target);
    graph
        .pass("resolve")
        .kind(rg::PassType::Graphics)
        .color_load(target);
    graph
}

/// One compiled frame, submitted. No pixels yet -- the pipeline and the command
/// list are covered elsewhere; this is the executor's own job.
#[test]
fn a_compiled_frame_runs_through_the_executor() {
    let Some(adapter) = Adapter::request(0) else {
        eprintln!("skipping: no wgpu adapter");
        return;
    };
    let Some((device, queue)) = adapter.create_device("stormglass-executor") else {
        return;
    };
    let resources = WgpuResources::new(&device);
    let mut executor = WgpuExecutor::new(
        queue,
        resources,
        chassis::Extent2d {
            width: WIDTH,
            height: HEIGHT,
        },
    );

    let graph = two_pass_frame();
    let compiled = graph.compile().expect("the frame compiles");
    assert_eq!(compiled.passes.len(), 2, "both passes survive culling");

    executor
        .execute(&compiled, &NoRecording)
        .expect("a frame with no recordings still submits");
    executor.wait_idle().expect("the queue drains");

    let stats = executor.stats();
    assert_eq!(
        stats,
        ExecutorStats {
            passes: 2,
            buffers: 2,
            frames: 1,
        }
    );
}

/// Two frames, because a second frame is where ordering bugs show up.
///
/// Nothing carries over between frames in wgpu -- each submit is independent and
/// the queue orders them -- so this is cheap to assert and catches an executor
/// that reused something it should have rebuilt.
#[test]
fn two_frames_in_a_row_both_run() {
    let Some(adapter) = Adapter::request(0) else {
        return;
    };
    let Some((device, queue)) = adapter.create_device("stormglass-two-frames") else {
        return;
    };
    let resources = WgpuResources::new(&device);
    let mut executor = WgpuExecutor::new(
        queue,
        resources,
        chassis::Extent2d {
            width: WIDTH,
            height: HEIGHT,
        },
    );
    let graph = two_pass_frame();
    let compiled = graph.compile().expect("compiles");

    for _ in 0..2 {
        executor
            .execute(&compiled, &NoRecording)
            .expect("each frame runs");
    }
    executor.wait_idle().expect("drains");
    assert_eq!(executor.stats().frames, 2);
}

/// An empty frame is not an error and submits nothing.
///
/// A frame that culls to nothing is a real condition -- a window minimised, a
/// camera off screen -- and treating it as a failure is how a frame loop ends up
/// logging an error every time the user alt-tabs.
#[test]
fn an_empty_frame_submits_nothing() {
    let Some(adapter) = Adapter::request(0) else {
        return;
    };
    let Some((device, queue)) = adapter.create_device("stormglass-empty") else {
        return;
    };
    let resources = WgpuResources::new(&device);
    let mut executor = WgpuExecutor::new(
        queue,
        resources,
        chassis::Extent2d {
            width: WIDTH,
            height: HEIGHT,
        },
    );

    let graph = rg::RenderGraph::new();
    let compiled = graph.compile().expect("an empty graph compiles");
    executor
        .execute(&compiled, &NoRecording)
        .expect("not an error");
    executor.wait_idle().expect("drains");

    let stats = executor.stats();
    assert_eq!(stats.passes, 0);
    assert_eq!(stats.frames, 1, "the frame still counted");
}

/// The seam, stated: a renderer implements `PassRecorder` once and the executor
/// drives it.
///
/// This compiles only because `PassRecorder` and `PassScope` live in `chassis`,
/// not in either backend. Before the move this file could not have existed
/// without wgpu depending on stormbreaker.
#[test]
fn a_renderer_writes_one_pass_recorder_for_either_backend() {
    let Some(adapter) = Adapter::request(0) else {
        return;
    };
    let Some((device, queue)) = adapter.create_device("stormglass-seam") else {
        return;
    };
    let resources = WgpuResources::new(&device);
    let mut executor = WgpuExecutor::new(
        queue,
        resources,
        chassis::Extent2d {
            width: WIDTH,
            height: HEIGHT,
        },
    );

    let graph = two_pass_frame();
    let compiled = graph.compile().expect("compiles");

    // A plain closure: no backend type, no backend import, nothing but chassis.
    //
    // Behind a `Mutex`, because `PassRecorder`'s blanket impl takes `Fn` and a
    // closure that accumulates state is `FnMut`. That is the shape every renderer
    // has to use and it is worth being blunt about: a plain `Vec` push would not
    // compile, so a renderer that wants to collect per-pass state reaches for
    // interior mutability immediately. A `Cell` would do for a single thread and
    // would silently stop the cross-thread recording the bound exists to allow.
    let visited = std::sync::Mutex::new(Vec::new());
    let record = |scope: &PassScope<'_>, _list: &mut dyn CommandList| {
        visited
            .lock()
            .expect("not poisoned")
            .push(scope.name.to_owned());
        assert_eq!(scope.extent.width, WIDTH);
        Ok(())
    };

    executor.execute(&compiled, &record).expect("runs");
    let visited = visited.into_inner().expect("not poisoned");
    assert_eq!(
        visited,
        vec!["draw", "resolve"],
        "every pass was visited, in compiled order",
    );
}

/// A pass that ends while still inside a render scope is refused, naming it.
///
/// Submitted as-is, wgpu ends the scope at `finish` and the tail of the pass is
/// quietly lost. That is the worst possible failure: a frame that renders, just
/// not all of it.
#[test]
fn a_pass_that_never_closes_its_render_scope_is_refused() {
    let Some(adapter) = Adapter::request(0) else {
        return;
    };
    let Some((device, queue)) = adapter.create_device("stormglass-unclosed") else {
        return;
    };
    let resources = WgpuResources::new(&device);
    let mut executor = WgpuExecutor::new(
        queue,
        resources,
        chassis::Extent2d {
            width: WIDTH,
            height: HEIGHT,
        },
    );

    let mut graph = rg::RenderGraph::new();
    let target = graph.import_texture(
        "target",
        rg::ImageState::Undefined,
        rg::ImageState::ShaderRead,
    );
    graph
        .pass("leaky")
        .kind(rg::PassType::Graphics)
        .color_load(target);
    let compiled = graph.compile().expect("compiles");

    let record = |scope: &PassScope<'_>, list: &mut dyn CommandList| {
        // Opens a scope and never ends it.
        let mut desc = chassis::RenderPassDesc::color(chassis::TextureFormat::Rgba8Unorm);
        desc.label = scope.name.to_owned();
        list.begin_rendering(&desc);
        Ok(())
    };

    let error = executor
        .execute(&compiled, &record)
        .expect_err("an unclosed scope must not be submitted");
    assert!(
        error.to_string().contains("leaky"),
        "the error names the pass, got {error}",
    );
}

/// The recorder is Send + Sync, which is what lets a backend record passes on
/// worker threads.
///
/// Asserted at compile time rather than described in a comment, because the whole
/// justification for `PassRecorder` living in a shared crate is that both
/// backends may call it from anywhere.
#[test]
fn the_pass_recorder_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<NoRecording>();
    // A closure capturing nothing is both, which is the trivial case; the trait
    // bound on `execute` is what actually matters and the compiler checks it.
    fn takes_recorder(_: &dyn PassRecorder) {}
    takes_recorder(&NoRecording);
}
