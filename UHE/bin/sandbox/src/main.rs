use std::fs;
use std::path::PathBuf;

use adam::{App, Engine, Error as EngineError, FrameCtx, Plugin};
use bifrost::Frame;
use chassis::Extent2d;
use vault::{EntityDoc, SceneDoc, Uuid, load_scene, save_scene};

struct SandboxPlugin {
    scene_entities: usize,
    issued: bool,
}

impl Plugin for SandboxPlugin {
    fn name(&self) -> &'static str {
        "sandbox"
    }

    fn build(&mut self, _eng: &mut Engine) -> adam::Result<()> {
        Ok(())
    }

    fn on_frame(&mut self, _eng: &mut Engine, ctx: &mut FrameCtx) {
        if !self.issued {
            let mut frame = Frame::begin(Extent2d {
                width: 1280,
                height: 720,
            });
            frame.set_number(ctx.frame);
            println!(
                "sandbox: issued frame {} (passes={}, scene entities={})",
                ctx.frame,
                frame.pass_count(),
                self.scene_entities
            );
            // Do not append further frames to the log: one issued frame proves the
            // render frontend works without a GPU or swapchain.
            self.issued = true;
        }
    }

    fn on_stop(&mut self, _eng: &mut Engine) {}
}

#[derive(Debug)]
struct Args {
    frames: u64,
    scene_bin: Option<PathBuf>,
    scene_text: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut args = Args {
        frames: 8,
        scene_bin: None,
        scene_text: None,
    };
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--frames" => {
                if let Some(value) = it.next() {
                    args.frames = value.parse().unwrap_or(8).max(1);
                }
            }
            "--scene-bin" => args.scene_bin = it.next().map(PathBuf::from),
            "--scene-text" => args.scene_text = it.next().map(PathBuf::from),
            _ => {}
        }
    }
    args
}

fn prepare_scene(args: &Args) -> Result<usize, String> {
    let (bin, text) = if let (Some(bin), Some(text)) = (&args.scene_bin, &args.scene_text) {
        (bin.clone(), text.clone())
    } else if let Some(bin) = &args.scene_bin {
        let text = bin.with_extension("toml");
        (bin.clone(), text)
    } else if let Some(text) = &args.scene_text {
        let bin = text.with_extension("bin");
        (bin, text.clone())
    } else {
        let base = std::env::temp_dir().join("unhuman-sandbox");
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let bin = base.join("scene.bin");
        let text = base.join("scene.toml");
        let mut scene = SceneDoc::default();
        scene.entities.push(EntityDoc {
            id: Uuid([1; 16]),
            parent: Uuid::NONE,
            name: "sandbox-root".to_string(),
            tags: vec!["sandbox".to_string()],
            transform: Default::default(),
            components: Vec::new(),
        });
        save_scene(&scene, &bin, &text).map_err(|e| e.to_string())?;
        (bin, text)
    };

    let loaded = load_scene(&bin, &text).map_err(|e| e.to_string())?;
    let _ = loaded.validate();
    Ok(loaded.entities.len())
}

fn main() {
    let args = parse_args();
    let scene_entities = match prepare_scene(&args) {
        Ok(n) => {
            println!("sandbox: loaded scene ({n} entities)");
            n
        }
        Err(error) => {
            eprintln!("sandbox: scene load failed: {error}");
            1
        }
    };

    let mut app = App::new().add(SandboxPlugin {
        scene_entities,
        issued: false,
    });
    match app.run_headless(args.frames) {
        Ok(()) => println!("sandbox: completed {frames} frames headless", frames = args.frames),
        Err(error) => {
            match error {
                EngineError::Plugin { name, message } => {
                    eprintln!("sandbox: plugin {name} failed: {message}");
                }
                other => eprintln!("sandbox: run failed: {other}"),
            }
            std::process::exit(1);
        }
    }
}
