//! Headless binary: the same frame loop with no window and no GPU.
//!
//! The C++ sandbox ran one layer that drew a model into a window; the headless
//! use of it is the same thing without the window, so this is that: the engine
//! with `gpu: None`, the game plugins registered, a fixed number of steps, and a
//! summary line instead of a dump of ImGui panels.

mod config;

use adam::App;
use aimlab::AimLabPlugin;
use config::Config;
use femur::FemurPlugin;
use wire::WirePlugin;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let config = match Config::parse(&arguments) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("nocap: {error}");
            std::process::exit(2);
        }
    };

    if config.help {
        println!("{}", config::USAGE);
        return;
    }

    if let Err(error) = run(config) {
        eprintln!("nocap: {error}");
        std::process::exit(1);
    }
}

fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new()
        .add(AimLabPlugin::new())
        .add(WirePlugin::new())
        .add(FemurPlugin);

    // The repository's `assets/` is where the relative paths in a scene and in
    // aimlab resolve from, exactly as in the editor binary.
    app.engine_mut().assets.mount("assets");

    log::info!("nocap: plugins {}", app.plugin_names().join(", "));

    if config.list_plugins {
        for name in app.plugin_names() {
            println!("{name}");
        }
        return Ok(());
    }

    if let Some(path) = &config.scene {
        let adam::Engine { scene, assets, .. } = app.engine_mut();
        *scene =
            meats::load(path).map_err(|error| format!("--scene {}: {error}", path.display()))?;
        for error in scene.resolve_assets(assets) {
            log::warn!("nocap: {error}");
        }
    }

    app.run(config.frames())?;

    let engine = app.engine();
    // `bodies` is what the physics world holds, sleeping bodies included: there
    // is no awake-body accessor, and zero is the honest "physics did nothing".
    println!(
        "nocap: frames={} entities={} bodies={}",
        engine.time.frame(),
        engine.scene.entities().len(),
        engine
            .physics
            .as_ref()
            .map_or(0, |physics| physics.body_count()),
    );
    Ok(())
}
