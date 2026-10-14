use std::sync::Arc;
use std::time::Duration;

use chassis::Gpu;
use grind::Jobs;

use crate::time::Time;

/// Everything the engine owns.
///
/// **Data only.** No behavior and no plugin types: a plugin owns its own state
/// and is handed `&mut Engine` per hook, which is what keeps `adam` free of any
/// dependency on `bifrost`, `stormbreaker`, or anything else. In the C++ engine
/// the equivalent type could not exist, because `Renderer3D::s_Data3D`,
/// `AudioEngine::s_ActiveSounds` and `g_VulkanContext` were file-static globals
/// that nothing could construct, so `tests/scene` had to recompile engine
/// sources with a stubbed renderer instead of linking them.
///
/// The service fields (`assets`, `models`, `physics`, `ui`) land here as their
/// crates appear. `gpu` is an `Option` from day one: headless is the same
/// binary with it set to `None`, not a second target.
pub struct Engine {
    pub jobs: Arc<Jobs>,
    pub time: Time,
    pub gpu: Option<Box<dyn Gpu>>,
}

impl Engine {
    /// Headless engine: no GPU, no swapchain, deterministic fixed steps.
    pub fn headless() -> Self {
        Self::new()
    }

    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Jobs::new()),
            time: Time::new(),
            gpu: None,
        }
    }

    pub fn with_gpu(gpu: Box<dyn Gpu>) -> Self {
        Self {
            gpu: Some(gpu),
            ..Self::new()
        }
    }

    pub fn is_headless(&self) -> bool {
        self.gpu.is_none()
    }

    pub fn is_dirty(&self) -> bool {
        self.time.pending_steps() > 0
    }

    /// Fires one fixed simulation step. Hooks are not called yet — there is
    /// nothing to run until `meats` and `eyes` land.
    pub fn simulate_fixed_step(&mut self) {
        let _ = self.time.consume_fixed_step();
    }

    pub fn frame_delta(&self) -> Duration {
        self.time.delta()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
