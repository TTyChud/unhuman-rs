//! The renderer as a plugin.
//!
//! `Engine` owns data and plugins own behavior (spine §1 rule 4), so `Bifrost`
//! lives here and never in `adam::Engine`: that is what keeps `adam` free of a
//! dependency on a renderer, and why the C++'s `s_Data3D` global has no
//! successor.
//!
//! `build` succeeds with `gpu: None`, because headless is the same binary with
//! no device (rule 6) and a renderer that refuses to construct would make the
//! headless run a different program. With no device there is no renderer and
//! every hook returns.

use std::sync::Arc;

use adam::{Engine, FrameCtx, Plugin};

use crate::camera::{CameraInput, EditorCamera};
use crate::passes::OverlayPasses;
use crate::renderer::{Bifrost, ViewSource};

pub struct BifrostPlugin {
    renderer: Option<Bifrost>,
    camera: EditorCamera,
    input: CameraInput,
    overlay: Option<Arc<dyn OverlayPasses>>,
    /// Frames that failed to record, so a backend that refuses every frame does
    /// not log one error per frame for the life of the process.
    frame_failures: u32,
}

impl BifrostPlugin {
    pub fn new() -> Self {
        Self {
            renderer: None,
            camera: EditorCamera::new(45.0, 16.0 / 9.0, 0.1, 1000.0),
            input: CameraInput::default(),
            overlay: None,
            frame_failures: 0,
        }
    }

    /// Installs the 2D and text slice's passes.
    pub fn with_overlay(mut self, overlay: Arc<dyn OverlayPasses>) -> Self {
        self.overlay = Some(overlay);
        self
    }

    /// The orbit camera. A caller with input owns the one copy of it and writes
    /// it here.
    pub fn camera_mut(&mut self) -> &mut EditorCamera {
        &mut self.camera
    }

    pub fn camera(&self) -> &EditorCamera {
        &self.camera
    }

    /// Where a caller copies `eyes::Input` into: bifrost's manifest has no
    /// `eyes`, and the camera takes the state as a value either way.
    pub fn input_mut(&mut self) -> &mut CameraInput {
        &mut self.input
    }

    pub fn renderer(&self) -> Option<&Bifrost> {
        self.renderer.as_ref()
    }

    pub fn renderer_mut(&mut self) -> Option<&mut Bifrost> {
        self.renderer.as_mut()
    }
}

impl Default for BifrostPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for BifrostPlugin {
    fn name(&self) -> &'static str {
        "bifrost"
    }

    fn build(&mut self, eng: &mut Engine) -> adam::Result<()> {
        let Some(gpu) = eng.gpu.as_mut() else {
            // Headless: there is no device and no frame to draw. Not an error.
            return Ok(());
        };

        let mut renderer = Bifrost::new(gpu.as_mut(), &mut eng.assets)?;
        if let Some(overlay) = self.overlay.take() {
            renderer.set_overlay(overlay);
        }

        if let Some(swapchain) = gpu.swapchain() {
            let extent = swapchain.extent();
            self.camera
                .set_viewport_size(extent.width as f32, extent.height as f32);
            renderer.resize(extent);
        }

        self.renderer = Some(renderer);
        Ok(())
    }

    fn on_frame(&mut self, eng: &mut Engine, _ctx: &mut FrameCtx) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(gpu) = eng.gpu.as_mut() else {
            return;
        };

        if let Some(swapchain) = gpu.swapchain() {
            let extent = swapchain.extent();
            let (width, height) = self.camera.viewport_size();
            if (extent.width as f32, extent.height as f32) != (width, height) {
                self.camera
                    .set_viewport_size(extent.width as f32, extent.height as f32);
                renderer.resize(extent);
            }
        }

        // The editor camera moves before the scene is read, so a frame draws the
        // camera state it was given rather than the previous frame's.
        self.camera.update(&self.input);

        // Upload anything the frame will want. A hit after the first frame is a
        // hash lookup; the C++ uploaded geometry once per glTF mesh at load time
        // for the same reason.
        for sample in eng.scene.meshes() {
            if let Err(error) = renderer.ensure_mesh(gpu.as_mut(), &eng.assets, sample.mesh) {
                log::warn!("bifrost: mesh {:?} failed to upload: {error}", sample.mesh);
            }
        }
    }

    fn on_render(&mut self, eng: &mut Engine, _ctx: &mut FrameCtx) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(gpu) = eng.gpu.as_mut() else {
            return;
        };

        // The scene's primary camera if it has one, the editor camera otherwise:
        // the C++ picks between the two `BeginScene` overloads the same way.
        let view = match eng.scene.primary_camera() {
            Some(entity) => ViewSource::Scene(entity),
            None => ViewSource::Editor,
        };

        if let Err(error) = renderer.render(
            gpu.as_mut(),
            &eng.jobs,
            &eng.scene,
            view,
            Some(&self.camera),
        ) {
            if self.frame_failures == 0 {
                log::error!("bifrost: frame failed: {error}");
            } else {
                log::debug!("bifrost: frame failed again ({error})");
            }
            self.frame_failures += 1;
        } else {
            self.frame_failures = 0;
        }
    }

    fn on_stop(&mut self, eng: &mut Engine) {
        if let (Some(renderer), Some(gpu)) = (self.renderer.as_mut(), eng.gpu.as_mut()) {
            renderer.destroy(gpu.as_mut());
        }
        self.renderer = None;
    }
}
