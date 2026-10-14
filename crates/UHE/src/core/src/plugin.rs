use std::time::Duration;

use crate::engine::Engine;
use crate::error::Result;

/// Per-frame plumbing handed to every hook.
pub struct FrameCtx {
    /// Index of the frame being executed, starting at 1.
    pub frame: u64,
    /// Real time since the previous frame, unscaled.
    pub delta: Duration,
    /// The fixed step this hook is running under. Equals `delta` on render
    /// frames, and `fixed_dt` on each simulation substep.
    pub step: Duration,
    /// Simulation speed multiplier in effect.
    pub scale: f32,
    /// Set by any hook to end the loop after this frame.
    pub quit: bool,
}

impl FrameCtx {
    pub fn step_seconds(&self) -> f32 {
        self.step.as_secs_f32()
    }
}

/// A unit of engine behavior.
///
/// Plugins are compiled in, not discovered at runtime. The alternatives that
/// genuinely swap out — a model loader, a physics backend, a UI toolkit — are
/// sibling crates, one per alternative, and a Cargo feature picks which one
/// exists. `NAME` gives `App::plugin_names()` and the boot log for free without
/// a registry entry.
///
/// The registry lives on `App`, **beside** `Engine`, never inside it. The
/// previous Rust attempt put plugins in a field of the same struct the hooks
/// borrow, so iterating plugins aliased `&mut Engine` with
/// `&mut Vec<Box<dyn Plugin>>` and needed `std::mem::take` around every loop
/// (`crates/unhuman/src/app/app.rs:101`).
pub trait Plugin: Send + 'static {
    /// Plugin identifier, for `--list-plugins` and the boot log.
    ///
    /// A method rather than `const NAME` on purpose: an associated const makes
    /// the trait not dyn-compatible, which would defeat the registry.
    fn name(&self) -> &'static str;

    /// Populate the engine's data from this plugin. Called once, in
    /// registration order.
    fn build(&mut self, eng: &mut Engine) -> Result<()>;

    fn on_start(&mut self, _eng: &mut Engine) -> Result<()> {
        Ok(())
    }

    /// Simulation substep. Called once per fixed step, not once per frame.
    fn on_update(&mut self, _eng: &mut Engine, _ctx: &mut FrameCtx) {}

    /// Once per rendered frame, after the fixed-step substeps and before
    /// `on_render`. Frame-rate logic belongs here: editor and UI updates, and
    /// anything that decides the frame should end. `on_update` cannot serve
    /// this, because a frame whose accumulator has not reached one fixed step
    /// runs no substeps at all.
    fn on_frame(&mut self, _eng: &mut Engine, _ctx: &mut FrameCtx) {}

    /// Record this frame's GPU work. Only called when `Engine::gpu` is `Some`.
    fn on_render(&mut self, _eng: &mut Engine, _ctx: &mut FrameCtx) {}

    fn on_stop(&mut self, _eng: &mut Engine) {}
}
