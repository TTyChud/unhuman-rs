use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use adam::{App, Engine, Error, FrameCtx, Plugin, Time};

struct Counter {
    name: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
    updates: Arc<AtomicUsize>,
    renders: Arc<AtomicUsize>,
}

type Mutex<T> = std::sync::Mutex<T>;

impl Counter {
    fn new(name: &'static str, log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            name,
            log,
            updates: Arc::new(AtomicUsize::new(0)),
            renders: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Plugin for Counter {
    fn name(&self) -> &'static str {
        self.name
    }

    fn build(&mut self, _eng: &mut Engine) -> adam::Result<()> {
        self.log.lock().unwrap().push("build");
        Ok(())
    }

    fn on_start(&mut self, _eng: &mut Engine) -> adam::Result<()> {
        self.log.lock().unwrap().push("start");
        Ok(())
    }

    fn on_update(&mut self, _eng: &mut Engine, _ctx: &mut FrameCtx) {
        self.updates.fetch_add(1, Ordering::AcqRel);
    }

    fn on_frame(&mut self, _eng: &mut Engine, _ctx: &mut FrameCtx) {
        self.log.lock().unwrap().push("frame");
    }

    fn on_render(&mut self, _eng: &mut Engine, _ctx: &mut FrameCtx) {
        self.renders.fetch_add(1, Ordering::AcqRel);
    }

    fn on_stop(&mut self, _eng: &mut Engine) {
        self.log.lock().unwrap().push("stop");
    }
}

#[test]
fn hooks_run_in_registration_order() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let a = Arc::clone(&log);
    let b = Arc::clone(&log);

    let mut app = App::new()
        .add(Counter::new("a", a))
        .add(Counter::new("b", b));

    app.run_headless(1).unwrap();

    assert_eq!(
        *log.lock().unwrap(),
        vec!["build", "build", "start", "start", "frame", "frame", "stop", "stop"]
    );
}

#[test]
fn plugin_names_are_reported() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let a = Arc::clone(&log);
    let b = Arc::clone(&log);

    let app = App::new()
        .add(Counter::new("first", a))
        .add(Counter::new("second", b));

    assert_eq!(app.plugin_names(), vec!["first", "second"]);
}

#[test]
fn headless_run_advances_the_requested_frames() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let counter = Counter::new("counter", Arc::clone(&log));

    let mut app = App::new().add(counter);
    app.run_headless(10).unwrap();

    assert_eq!(app.engine().time.frame(), 10);
}

#[test]
fn headless_never_calls_on_render() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let counter = Counter::new("counter", Arc::clone(&log));
    let renders = Arc::clone(&counter.renders);

    let mut app = App::new().add(counter);
    app.run_headless(5).unwrap();

    assert_eq!(renders.load(Ordering::Acquire), 0);
    assert!(app.engine().is_headless());
}

#[test]
fn simulation_runs_in_fixed_steps() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let counter = Counter::new("counter", Arc::clone(&log));
    let updates = Arc::clone(&counter.updates);

    let mut engine = Engine::new();
    engine.time = Time::with_fixed_dt(Duration::from_micros(100));
    // Let real time accumulate before the frame, so the frame's first tick has
    // a backlog to drain.
    std::thread::sleep(Duration::from_millis(5));

    let mut app = App::with_engine(engine).add(counter);
    app.frame().unwrap();

    // One frame drains every pending step, so hooks see the simulation settle
    // regardless of frame rate: 5ms of backlog at a 100us step is ~50 substeps
    // inside a single frame.
    let steps = updates.load(Ordering::Acquire);
    assert!(steps > 10, "only {steps} fixed steps ran");
}

struct Quitter;

impl Plugin for Quitter {
    fn name(&self) -> &'static str {
        "quitter"
    }
    fn build(&mut self, _eng: &mut Engine) -> adam::Result<()> {
        Ok(())
    }
    fn on_frame(&mut self, _eng: &mut Engine, ctx: &mut FrameCtx) {
        if ctx.frame >= 3 {
            ctx.quit = true;
        }
    }
}

#[test]
fn a_hook_can_end_the_loop() {
    let mut app = App::new().add(Quitter);
    app.run(None).unwrap();

    assert_eq!(app.engine().time.frame(), 3);
    assert!(!app.is_running());
}

struct Failing;

impl Plugin for Failing {
    fn name(&self) -> &'static str {
        "failing"
    }
    fn build(&mut self, _eng: &mut Engine) -> adam::Result<()> {
        Err(adam::chassis::Error::Unsupported("no device".into()).into())
    }
}

#[test]
fn a_failing_build_reports_the_plugin_name() {
    let mut app = App::new().add(Failing);
    let error = app.run_headless(1).unwrap_err();

    match error {
        Error::Plugin { name, message } => {
            assert_eq!(name, "failing");
            assert!(message.contains("no device"), "message was {message:?}");
        }
        other => panic!("expected a plugin error, got {other:?}"),
    }
}

#[test]
fn app_without_plugins_still_runs() {
    let mut app = App::new();
    app.run_headless(3).unwrap();
    assert_eq!(app.engine().time.frame(), 3);
    assert!(app.plugin_names().is_empty());
}
