//! The 2D and text slice's passes, wired into a bifrost frame.
//!
//! `batch2d` and `text` own the shaders, the pipelines, the batching, the
//! vertex formats and the drawing loop — everything about *how* a sprite or a
//! glyph is drawn. This is the other half: which passes a frame declares, what
//! they read and write, the buffers each batch's contents need, and the bind
//! groups their texture lookups resolve to.
//!
//! ## Interior mutability, and why
//!
//! A caller pushes quads and text runs with `edit`, before `prepare` uploads the
//! result, and the frame holds the same object as `Arc<dyn OverlayPasses>` to
//! record with. So the batching state is behind a mutex: one owner per frame,
//! two accessors, and a `PassRecorder` that has to be `Sync` because a backend
//! may record passes on worker threads. A `Cell` would be enough for one thread
//! and would silently stop that.
//!
//! ## Why the buffers are per frame
//!
//! `Resources` has no update-in-place call, so the vertices a frame produced are
//! created (and the previous frame's destroyed) in `prepare`. One allocation per
//! batch per frame; `batch2d::create_buffers` says the same.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chassis::graph::{PassScope, RenderGraph};
use chassis::{
    BindGroup, Buffer, CommandList, DescriptorSetLayoutDesc, Error, ImageView, Resources, Sampler,
    Texture,
};
use glam::Mat4;

use crate::batch2d::{Batch2D, Globals, PipelineError, QuadPipeline, SpriteGlobals};
use crate::font::Atlas;
use crate::passes::{FrameTargets, OverlayPasses};
use crate::text::{TextGlobals, TextRenderer};

/// The two passes this slice draws in.
pub const PASS_2D: &str = "bifrost.2d";
pub const PASS_TEXT: &str = "bifrost.text";

/// Everything a frame mutates: the batches, and the GPU objects built from them.
struct State {
    batch: Batch2D,
    texts: TextRenderer,

    sprite_vertices: Option<Buffer>,
    sprite_indices: Option<Buffer>,
    text_vertices: Option<Buffer>,
    text_indices: Option<Buffer>,
    sprite_globals: Option<Globals>,
    text_globals: Option<Globals>,

    /// One group per distinct texture a batch bound. `None` is the untextured
    /// case, which binds the 1x1 white texel the C++ keeps in slot 0
    /// (`Renderer2D.cpp:30,127-131`).
    sprite_textures: HashMap<Option<Texture>, BindGroup>,
    text_textures: HashMap<Texture, BindGroup>,
}

impl State {
    fn new(quads: usize) -> Self {
        Self {
            batch: Batch2D::with_capacity(quads),
            texts: TextRenderer::new(),
            sprite_vertices: None,
            sprite_indices: None,
            text_vertices: None,
            text_indices: None,
            sprite_globals: None,
            text_globals: None,
            sprite_textures: HashMap::new(),
            text_textures: HashMap::new(),
        }
    }

    fn has_sprites(&self) -> bool {
        !self.batch.batches().is_empty()
    }

    fn has_text(&self) -> bool {
        !self.texts.runs().is_empty()
    }

    fn destroy_groups(&mut self, resources: &mut dyn Resources) {
        for (_, group) in self.sprite_textures.drain() {
            resources.destroy_bind_group(group);
        }
        for (_, group) in self.text_textures.drain() {
            resources.destroy_bind_group(group);
        }
        for mut globals in [self.sprite_globals.take(), self.text_globals.take()]
            .into_iter()
            .flatten()
        {
            globals.destroy(resources);
        }
        for buffer in [
            self.sprite_vertices.take(),
            self.sprite_indices.take(),
            self.text_vertices.take(),
            self.text_indices.take(),
        ]
        .into_iter()
        .flatten()
        {
            resources.destroy_buffer(buffer);
        }
    }
}

/// The 2D and text passes over one frame's batches.
pub struct SpriteTextOverlay {
    sprite: QuadPipeline,
    text: QuadPipeline,
    color_format: chassis::TextureFormat,
    state: Mutex<State>,
}

impl SpriteTextOverlay {
    /// Builds both pipelines. The colour format is the scene colour's: both draw
    /// into it, not into the swapchain.
    pub fn new(
        resources: &mut dyn Resources,
        color_format: chassis::TextureFormat,
    ) -> Result<Self, PipelineError> {
        Ok(Self {
            sprite: QuadPipeline::sprite(resources, color_format, None)?,
            text: crate::text::create_text_pipeline(resources, color_format, None)?,
            color_format,
            state: Mutex::new(State::new(0)),
        })
    }

    pub fn color_format(&self) -> chassis::TextureFormat {
        self.color_format
    }

    pub fn sprite_texture_layout(&self) -> &DescriptorSetLayoutDesc {
        self.sprite.texture_layout()
    }

    pub fn text_texture_layout(&self) -> &DescriptorSetLayoutDesc {
        self.text.texture_layout()
    }

    /// The shared handle `Bifrost::set_overlay` takes.
    pub fn into_arc(self) -> Arc<dyn OverlayPasses> {
        Arc::new(self)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("overlay mutex poisoned")
    }

    /// Starts a frame: the batches from the previous one are cleared.
    pub fn begin(&self) {
        let mut state = self.state();
        state.batch.begin();
        state.texts.begin();
    }

    /// Push quads and text runs. One closure so the lock is taken once.
    pub fn edit(&self, edit: impl FnOnce(&mut Batch2D, &mut TextRenderer)) {
        let mut state = self.state();
        let State { batch, texts, .. } = &mut *state;
        edit(batch, texts);
    }

    pub fn is_empty(&self) -> bool {
        let state = self.state();
        !state.has_sprites() && !state.has_text()
    }

    /// Uploads this frame's batches and builds their bind groups.
    ///
    /// `textures` resolves a batch's texture to the view that will be bound — the
    /// caller owns the asset-to-view half, because it owns the asset registry.
    /// `atlas_texture` is the view of the font atlas, and `white` is the view an
    /// untextured quad binds.
    pub fn prepare(
        &self,
        resources: &mut dyn Resources,
        view_projection: Mat4,
        atlas: &Atlas,
        atlas_texture: Texture,
        white: ImageView,
        sampler: Sampler,
        textures: &[(Texture, ImageView)],
    ) -> Result<(), Error> {
        // An untextured quad binds this, so a null handle would reach the driver
        // as a null image view. Refused here, where the caller can be named.
        if white.is_null() {
            return Err(Error::InvalidDescriptor(
                "an untextured 2D quad needs a 1x1 white view; the caller passed a null one"
                    .to_owned(),
            ));
        }

        // One `DerefMut` and then plain field projections: `state.batch` is read
        // while `state.sprite_textures` is written, and two projections through
        // a guard are not guaranteed to split.
        let mut guard = self.state();
        let state = &mut *guard;
        state.destroy_groups(resources);

        if state.has_sprites() {
            let mut globals = self.sprite.globals();
            globals.upload(resources, &SpriteGlobals::new(view_projection))?;
            state.sprite_globals = Some(globals);

            let (vertices, indices) = crate::batch2d::create_buffers(resources, &state.batch)?;
            state.sprite_vertices = Some(vertices);
            state.sprite_indices = Some(indices);

            for batch in state.batch.batches() {
                if state.sprite_textures.contains_key(&batch.texture) {
                    continue;
                }
                let view = match batch.texture {
                    Some(texture) => resolve(textures, texture).ok_or_else(|| {
                        Error::InvalidDescriptor(format!(
                            "the 2D batch binds {texture:?}, which the caller did not resolve \
                             to a view"
                        ))
                    })?,
                    None => white,
                };
                let group = resources.create_bind_group(
                    self.sprite.texture_layout(),
                    &[],
                    &[(0, view)],
                    &[(1, sampler)],
                )?;
                state.sprite_textures.insert(batch.texture, group);
            }
        }

        if state.has_text() {
            let mut globals = self.text.globals();
            globals.upload(resources, &TextGlobals::new(view_projection, atlas))?;
            state.text_globals = Some(globals);

            let (vertices, indices) = crate::text::create_buffers(resources, &state.texts)?;
            state.text_vertices = Some(vertices);
            state.text_indices = Some(indices);

            let atlas_view = resolve(textures, atlas_texture).ok_or_else(|| {
                Error::InvalidDescriptor("the font atlas has no bound view".to_owned())
            })?;
            for run in state.texts.runs() {
                if state.text_textures.contains_key(&run.texture) {
                    continue;
                }
                let group = resources.create_bind_group(
                    self.text.texture_layout(),
                    &[],
                    &[(0, atlas_view)],
                    &[(1, sampler)],
                )?;
                state.text_textures.insert(run.texture, group);
            }
        }

        Ok(())
    }

    /// Drops every GPU resource this overlay owns.
    pub fn destroy(&mut self, resources: &mut dyn Resources) {
        let mut guard = self.state();
        guard.destroy_groups(resources);
        resources.destroy_pipeline(self.sprite.pipeline());
        resources.destroy_shader(self.sprite.vertex_shader());
        resources.destroy_shader(self.sprite.fragment_shader());
        resources.destroy_pipeline(self.text.pipeline());
        resources.destroy_shader(self.text.vertex_shader());
        resources.destroy_shader(self.text.fragment_shader());
    }

    pub fn sprite_pipeline(&self) -> &QuadPipeline {
        &self.sprite
    }

    pub fn text_pipeline(&self) -> &QuadPipeline {
        &self.text
    }
}

impl OverlayPasses for SpriteTextOverlay {
    fn declare(&self, graph: &mut RenderGraph, targets: &FrameTargets) {
        let state = self.state();
        // Both load the scene colour: the 3D passes wrote it, and the frame's
        // composition reads it after these.
        if state.has_sprites() {
            graph.pass(PASS_2D).color_load(targets.scene_color);
        }
        if state.has_text() {
            graph.pass(PASS_TEXT).color_load(targets.scene_color);
        }
    }

    fn record(
        &self,
        pass: &str,
        _scope: &PassScope<'_>,
        list: &mut dyn CommandList,
    ) -> Result<(), Error> {
        let state = self.state();

        match pass {
            PASS_2D => {
                let (Some(vertices), Some(indices)) =
                    (state.sprite_vertices, state.sprite_indices)
                else {
                    return Ok(());
                };
                let groups = &state.sprite_textures;
                let globals = state
                    .sprite_globals
                    .as_ref()
                    .and_then(Globals::group)
                    .ok_or_else(|| Error::InvalidDescriptor(
                        "pass 'bifrost.2d' has no globals: prepare did not run".to_owned(),
                    ))?;
                state.batch.record(
                    list,
                    &self.sprite.pipeline(),
                    &vertices,
                    &indices,
                    &globals,
                    |texture| {
                        *groups.get(&texture).expect(
                            "a batch texture with no group: prepare skipped one, which is a bug",
                        )
                    },
                );
                Ok(())
            }
            PASS_TEXT => {
                let (Some(vertices), Some(indices)) = (state.text_vertices, state.text_indices)
                else {
                    return Ok(());
                };
                let groups = &state.text_textures;
                let globals = state
                    .text_globals
                    .as_ref()
                    .and_then(Globals::group)
                    .ok_or_else(|| Error::InvalidDescriptor(
                        "pass 'bifrost.text' has no globals: prepare did not run".to_owned(),
                    ))?;
                state.texts.record(
                    list,
                    &self.text.pipeline(),
                    &vertices,
                    &indices,
                    &globals,
                    |texture| {
                        *groups.get(&texture).expect(
                            "an atlas with no group: prepare skipped one, which is a bug",
                        )
                    },
                );
                Ok(())
            }
            other => Err(Error::InvalidDescriptor(format!(
                "the overlay has no pass called '{other}'"
            ))),
        }
    }
}

fn resolve(textures: &[(Texture, ImageView)], texture: Texture) -> Option<ImageView> {
    textures
        .iter()
        .find(|(candidate, _)| *candidate == texture)
        .map(|(_, view)| *view)
}
