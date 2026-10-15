//! Windowed binary: a real window, a real swapchain, one frame loop.
//!
//! The C++ sandbox pushed one layer that loaded a glTF and drew it; this is that
//! with the layer replaced by plugins, and with the same frame the engine's
//! headless run uses — `App::frame` — so the two binaries differ only in what
//! they put in `Engine::gpu`.
//!
//! The GPU cannot exist before the window does: a `wgpu` surface is created from
//! a `winit` window and the window is winit's, handed over inside the frame
//! callback. So `App::start` is called from the first frame, once there is a
//! device for the plugins to build against.

use std::sync::{Arc, Mutex};

use adam::App;
use aimlab::AimLabPlugin;
use bifrost::BifrostPlugin;
use chassis::backend::{BackendChoice, BackendSelection};
use eyes::{FrameView, WindowDesc, WindowExt};
use femur::FemurPlugin;
use mirror::MirrorPlugin;
use wire::WirePlugin;

const TITLE: &str = "unhuman sandbox";
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

const USAGE: &str = "\
sandbox - windowed frame loop

USAGE:
    sandbox [--backend wgpu|vulkan]

OPTIONS:
    --backend NAME   wgpu (default) or vulkan
    -h, --help       print this help

The backend is also readable from UHE_BACKEND and from the `backend` key in the
settings file; the flag wins.";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return;
    }

    if let Err(error) = run(&arguments) {
        eprintln!("sandbox: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: &[String]) -> Result<(), String> {
    let (settings, skipped) = engine::load_settings();
    if let Some(reason) = skipped {
        eprintln!("sandbox: ignoring settings file ({reason})");
    }
    let environment = std::env::var(chassis::backend::ENVIRONMENT_VARIABLE).ok();
    let selection = BackendSelection::resolve(
        arguments,
        environment.as_deref(),
        settings.as_ref().and_then(chassis::settings::Settings::backend),
    )
    .map_err(|error| error.to_string())?;

    if selection.choice != BackendChoice::Wgpu {
        return Err(format!(
            "the windowed path is wgpu-only and {} asked for {}; creating a Vulkan \
             surface is not wired",
            selection.source,
            selection.choice.name()
        ));
    }
    log::info!(
        "sandbox: backend {} ({})",
        selection.choice.name(),
        selection.source
    );

    let instance = stormglass::create_instance();

    let mut app = App::new()
        .add(AimLabPlugin::new())
        .add(BifrostPlugin::new())
        .add(WirePlugin::new())
        .add(FemurPlugin)
        .add(MirrorPlugin::new());

    // The repository's `assets/` is where the relative paths in a scene and in
    // aimlab resolve from, exactly as in the editor binary.
    app.engine_mut().assets.mount("assets");

    log::info!("sandbox: plugins {}", app.plugin_names().join(", "));

    // The frame callback is `'static` and owns the app, so a failure inside it
    // has to be handed back out through something shared. Failure here is
    // terminal, and an exit status of 0 after "no wgpu device" is a lie a script
    // would believe.
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let reported = Arc::clone(&failure);
    let frame = move |view: &mut FrameView<'_>| -> bool {
        let fail = |message: String| {
            if let Ok(mut slot) = reported.lock() {
                *slot = Some(message);
            }
            false
        };

        if app.engine().gpu.is_none() {
            let window = view.window.clone();
            let (width, height) = window.size();
            let extent = chassis::Extent2d {
                width: width.max(1),
                height: height.max(1),
            };
            let surface = match instance.create_surface(window.clone()) {
                Ok(surface) => surface,
                Err(error) => return fail(format!("no wgpu surface: {error}")),
            };
            match stormglass::WgpuGpu::with_surface(&instance, surface, extent) {
                Ok(gpu) => *app.engine_mut().gpu = Some(Box::new(gpu)),
                Err(error) => return fail(format!("wgpu: {error}")),
            }
            if let Err(error) = app.start() {
                return fail(error.to_string());
            }
        }

        // The window layer pushes; `App::frame` drains into `Engine::input`, so
        // there is one copy of the held and pressed state and not two.
        for event in view.events {
            app.engine_mut().events.push(event.clone());
        }
        if let Err(error) = app.frame() {
            return fail(error.to_string());
        }
        if app.is_running() {
            return true;
        }
        // The loop is over: run the plugins' teardown hook while the window and
        // the device are still alive.
        if let Err(error) = app.stop() {
            return fail(error.to_string());
        }
        false
    };

    let desc = WindowDesc {
        title: TITLE.to_owned(),
        width: WIDTH,
        height: HEIGHT,
        vsync: true,
    };
    eyes::run_windowed(desc, frame).map_err(|error| format!("window: {error}"))?;

    let message = failure.lock().ok().and_then(|mut slot| slot.take());
    match message {
        Some(message) => Err(message),
        None => Ok(()),
    }
}
