use std::sync::Arc;
use std::time::Duration;

use chassis::Gpu;
use grind::Jobs;

use crate::backend::{AudioBackend, PhysicsBackend};
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
/// The substitutable fields are trait objects, one implementation each, chosen
/// by the binary that builds the `App`: `gpu` is `None` in headless mode, and
/// that is the whole of "headless is the same binary" — not a second target.
///
/// Field order is drop order, and it is deliberate: the GPU is dropped last, so
/// a plugin's pipelines and buffers are destroyed while the device that made
/// them is still alive. Declaring `gpu` first is the bug that leaked every
/// command buffer in the Vulkan backend.
pub struct Engine {
    pub jobs: Arc<Jobs>,
    pub time: Time,
    pub events: pings::EventBus,
    pub input: eyes::Input,
    pub scene: meats::Scene,
    pub assets: vault::Assets,
    pub physics: Option<Box<dyn PhysicsBackend>>,
    pub audio: Option<Box<dyn AudioBackend>>,
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
            events: pings::EventBus::new(),
            input: eyes::Input::new(),
            scene: meats::Scene::new(),
            assets: vault::Assets::new("."),
            physics: None,
            audio: None,
            gpu: None,
        }
    }

    pub fn with_gpu(gpu: Box<dyn Gpu>) -> Self {
        Self {
            gpu: Some(gpu),
            ..Self::new()
        }
    }

    /// Assets root, before any plugin is built.
    pub fn with_asset_root(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            assets: vault::Assets::new(root),
            ..Self::new()
        }
    }

    pub fn is_headless(&self) -> bool {
        self.gpu.is_none()
    }

    pub fn is_dirty(&self) -> bool {
        self.time.pending_steps() > 0
    }

    /// The data side of one fixed step: transforms, physics, asset polling.
    ///
    /// Runs *before* the plugins' `on_update` hooks, so a hook that reads a
    /// world transform reads the one it will be drawn with.
    pub fn simulate_fixed_step(&mut self, step: Duration) {
        self.scene.update_transforms();

        if let Some(physics) = self.physics.as_mut() {
            physics.step(&mut self.scene, step.as_secs_f32());
        }

        for (id, result) in self.assets.poll() {
            if let Err(error) = result {
                log::warn!("asset {id:?} failed to load: {error}");
            }
        }
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
