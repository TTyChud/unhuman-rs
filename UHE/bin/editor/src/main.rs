use eyes::{Frame, HeadlessWindow, Window, WindowConfig, WinitWindow};
use mirror::theme::{DEFAULT_THEME, Theme, theme_named};
use mirror::{Command, CommandRegistry, Layout, PaintList};

use adam::{App, Engine, FrameCtx, Plugin};

struct EditorUi {
    theme: Theme,
    commands: CommandRegistry,
    layout: Layout,
    paints: PaintList,
}

struct EditorPlugin {
    theme_name: String,
    frames_target: u64,
    ui: Option<EditorUi>,
    printed: bool,
}

impl Plugin for EditorPlugin {
    fn name(&self) -> &'static str {
        "editor"
    }

    fn build(&mut self, _eng: &mut Engine) -> adam::Result<()> {
        let theme_def = theme_named(&self.theme_name).copied().unwrap_or(DEFAULT_THEME);
        let theme = theme_def.derive();

        let mut commands = CommandRegistry::new();
        let _ = commands.register(Command::new("app.quit").label("Quit").menu("File"));
        let _ = commands.register(Command::new("scene.new").label("New Scene").menu("File"));

        let mut layout = Layout::single("viewport".to_string());
        layout.split(mirror::layout::SplitAxis::Vertical, 0.25, "hierarchy".to_string());
        layout.split(mirror::layout::SplitAxis::Horizontal, 0.7, "inspector".to_string());

        let mut paints = PaintList::new();
        let _ = &mut paints;
        // Keep the list construction compile-visible; no GPU is required for this.
        let _ = paints.len();

        println!(
            "editor: theme startup: {} ({})",
            theme.def.name,
            if theme.first_legibility_failure().is_none() {
                "legible"
            } else {
                "legibility failure"
            }
        );
        println!(
            "editor: ui panel: {} commands, layout={}",
            commands.commands().len(),
            layout.to_string_repr()
        );

        self.ui = Some(EditorUi {
            theme,
            commands,
            layout,
            paints,
        });
        Ok(())
    }

    fn on_frame(&mut self, _eng: &mut Engine, ctx: &mut FrameCtx) {
        if !self.printed {
            if let Some(ui) = &self.ui {
                println!(
                    "editor: frame {} running UI-only loop (commands={}, layout={}, paints={}, theme={})",
                    ctx.frame,
                    ui.commands.commands().len(),
                    ui.layout.to_string_repr(),
                    ui.paints.len(),
                    ui.theme.def.name
                );
            }
            self.printed = true;
        }
        if ctx.frame >= self.frames_target {
            ctx.quit = true;
        }
    }
}

fn parse_args() -> (u64, String, bool) {
    let mut frames = 5u64;
    let mut theme = DEFAULT_THEME.name.to_string();
    let mut headless = false;
    let mut skip = false;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if skip {
            skip = false;
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--frames" => {
                if let Some(v) = args.get(i + 1) {
                    frames = v.parse().unwrap_or(frames).max(1);
                    skip = true;
                }
            }
            "--theme" => {
                if let Some(v) = args.get(i + 1) {
                    theme = v.clone();
                    skip = true;
                }
            }
            "--headless" => headless = true,
            _ => {}
        }
        i += 1;
    }
    (frames, theme, headless)
}

fn main() {
    let (frames, theme_name, headless) = parse_args();

    let config = WindowConfig::titled("unhuman editor");
    let mut window: Box<dyn Window> = if headless {
        Box::new(HeadlessWindow::with_config(config.clone()))
    } else {
        match WinitWindow::new(config.clone()) {
            Ok(window) => Box::new(window),
            Err(error) => {
                eprintln!("editor: no display available ({error}); falling back to headless window");
                Box::new(HeadlessWindow::with_config(config))
            }
        }
    };

    let plugin = EditorPlugin {
        theme_name,
        frames_target: frames,
        ui: None,
        printed: false,
    };

    let mut app = App::new().add(plugin);
    if let Err(error) = app.start() {
        eprintln!("editor: start failed: {error}");
        std::process::exit(1);
    }

    let run_result = window.run(&mut |frame: &mut dyn Frame| {
        app.frame().map_err(|e| eyes::Error::Window(e.to_string()))?;
        if !app.is_running() {
            frame.request_close();
        }
        if app.engine().time.frame() >= frames {
            frame.request_close();
        }
        Ok(())
    });

    let _ = app.stop();
    match run_result {
        Ok(()) => println!("editor: completed {} frames UI-only", frames),
        Err(error) => {
            eprintln!("editor: window loop failed: {error}");
            std::process::exit(1);
        }
    }
}
