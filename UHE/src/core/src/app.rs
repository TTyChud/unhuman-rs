use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::plugin::{FrameCtx, Plugin};
/// The composition root.
///
/// `App` owns the plugin registry and the loop; `Engine` owns the data. They
/// are separate fields of the same struct precisely so that iterating plugins
/// and handing each one `&mut self.engine` are two disjoint borrows rather than
/// an aliasing problem.
///
/// Declaration order is drop order, and here it is load-bearing: **plugins drop
/// before the engine**, so a renderer's pipelines, buffers and bind groups are
/// destroyed while the device that created them is still alive. The other order
/// compiles, reports no error, and leaks every resource the renderer owns —
/// which is exactly what the Vulkan backend did before its device was moved
/// first in its own struct.
pub struct App {
    plugins: Vec<Box<dyn Plugin>>,
    engine: Engine,
    running: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            engine: Engine::new(),
            running: true,
        }
    }

    pub fn with_engine(engine: Engine) -> Self {
        Self {
            plugins: Vec::new(),
            engine,
            running: true,
        }
    }

    /// Registration order is execution order, and it is visible here rather
    /// than hidden in a trait.
    #[allow(clippy::should_implement_trait)]
    pub fn add<P: Plugin>(mut self, plugin: P) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    pub fn plugin_names(&self) -> Vec<&'static str> {
        self.plugins.iter().map(|p| p.name()).collect()
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Builds and starts every plugin in registration order.
    pub fn start(&mut self) -> Result<()> {
        for plugin in &mut self.plugins {
            plugin.build(&mut self.engine).map_err(|e| Error::Plugin {
                name: plugin.name(),
                message: e.to_string(),
            })?;
        }
        for plugin in &mut self.plugins {
            plugin
                .on_start(&mut self.engine)
                .map_err(|e| Error::Plugin {
                    name: plugin.name(),
                    message: e.to_string(),
                })?;
        }
        Ok(())
    }

    /// Advances one frame: fixed simulation substeps, then hooks, then GPU.
    pub fn frame(&mut self) -> Result<()> {
        let delta = self.engine.time.tick();
        let mut ctx = FrameCtx {
            frame: self.engine.time.frame(),
            delta,
            step: delta,
            scale: self.engine.time.scale(),
            quit: false,
        };

        // Input state is owned by the engine and fed from the event bus, one
        // consumer, one source of truth. The window layer pushes events; it does
        // not keep a second copy of the held/pressed state, which is how the C++
        // ended up with `WindowInput` disagreeing with the ImGui layer.
        self.engine.input.begin_frame();
        for event in self.engine.events.drain() {
            self.engine.input.apply(&event);
        }

        // Fixed-step substeps first, so rendering always sees a settled
        // simulation state regardless of frame rate.
        while let Some(step) = self.engine.time.consume_fixed_step() {
            ctx.step = step;
            self.engine.simulate_fixed_step(step);
            for plugin in &mut self.plugins {
                plugin.on_update(&mut self.engine, &mut ctx);
            }
        }

        ctx.step = self.engine.time.fixed_dt();
        for plugin in &mut self.plugins {
            plugin.on_frame(&mut self.engine, &mut ctx);
        }

        // `on_render` runs once, and only with a GPU: with no GPU there is
        // nothing to record into. The `gpu` borrow is a separate statement each
        // time rather than a binding held across the loop, because the hook
        // needs `&mut self.engine` as a whole and pulls the GPU out of it
        // itself.
        //
        // The order is the frame's contract with a backend: acquire, let every
        // plugin record (a renderer hands its compiled frame to
        // `Gpu::record_frame`), then `submit`, which is the one place a frame is
        // presented. A plugin must not present, and `record_frame` must not —
        // two present sites is a frame presented twice.
        if self.engine.gpu.is_some() {
            if let Some(gpu) = self.engine.gpu.as_mut() {
                gpu.frame_begin()?;
            }
            for plugin in &mut self.plugins {
                plugin.on_render(&mut self.engine, &mut ctx);
            }
            if let Some(gpu) = self.engine.gpu.as_mut() {
                gpu.submit()?;
            }
        }

        if ctx.quit {
            self.running = false;
        }

        self.engine.input.end_frame();
        Ok(())
    }

    /// Runs `steps` frames, or until a hook requests a quit when `steps` is
    /// `None`.
    pub fn run(&mut self, steps: Option<u64>) -> Result<()> {
        self.start()?;
        let mut remaining = steps;
        while self.running {
            self.frame()?;
            if let Some(n) = &mut remaining {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    break;
                }
            }
        }
        self.stop()
    }

    /// Deterministic fixed-step run with no window, no vsync and no present.
    pub fn run_headless(&mut self, steps: u64) -> Result<()> {
        self.running = true;
        self.run(Some(steps))
    }

    pub fn stop(&mut self) -> Result<()> {
        for plugin in &mut self.plugins {
            plugin.on_stop(&mut self.engine);
        }
        Ok(())
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
