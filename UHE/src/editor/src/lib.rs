//! The editor model: the outliner, the property panel, the content browser,
//! the theme, the gizmo, the play state and the undo history.
//!
//! Toolkit-agnostic on purpose. The C++ editor was an ImGui layer, so its panel
//! state could only exist inside a frame; here the state is a plain struct and
//! the gpui binary is one renderer of it. Nothing in this crate names a UI
//! library, draws a widget, or opens a window.
//!
//! `Editor` holds no scene reference. Every call that needs the scene takes
//! `&meats::Scene` or `&mut meats::Scene`, which is what keeps the editor out
//! of the engine's ownership and lets a headless run construct the same model.

pub mod commands;
pub mod content_browser;
pub mod gizmo;
pub mod hierarchy;
pub mod inspector;
pub mod play_state;
pub mod selection;
pub mod theme;

pub use commands::{Command, History, DEFAULT_CAPACITY};
pub use content_browser::{ContentBrowser, DragPayload, Entry};
pub use gizmo::{snap_angle, snap_scale, snap_translate, Axis, GizmoMode, GizmoTransform, Snap};
pub use hierarchy::{Hierarchy, Row};
pub use inspector::{Inspector, Property, PropertyKind, PropertyValue};
pub use play_state::{PlayState, PlayStatus};
pub use selection::Selection;
pub use theme::{Palette, Rgba, Theme, ThemeError, ThemeId};

use std::path::{Path, PathBuf};

use adam::{Engine, FrameCtx, Plugin};
use meats::{Entity, Scene};

use commands::set_tag;
use gizmo::TransformGizmo;
use hierarchy::{is_within_subtree, tag_name};

/// Frame timings for the status bar. The C++ read them straight out of
/// `ImGui::GetIO()`, which is a toolkit's counter and not the engine's.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Stats {
    pub frame_ms: f32,
    pub fps: f32,
}

impl Stats {
    pub fn update(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        self.frame_ms = dt * 1000.0;
        // Smoothed, because a raw reciprocal flickers too fast to read.
        let instantaneous = 1.0 / dt;
        self.fps = if self.fps <= 0.0 {
            instantaneous
        } else {
            self.fps * 0.9 + instantaneous * 0.1
        };
    }
}

pub struct Editor {
    pub selection: Selection,
    pub hierarchy: Hierarchy,
    pub inspector: Inspector,
    pub content_browser: ContentBrowser,
    pub theme: Theme,
    pub gizmo: TransformGizmo,
    pub play: PlayState,
    pub history: History,
    pub stats: Stats,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            selection: Selection::new(),
            hierarchy: Hierarchy::new(),
            inspector: Inspector::new(),
            content_browser: ContentBrowser::new(PathBuf::from("assets")),
            theme: Theme::new(),
            gizmo: TransformGizmo::new(),
            play: PlayState::new(),
            history: History::new(),
            stats: Stats::default(),
        }
    }

    pub fn selected_entity(&self) -> Option<Entity> {
        self.selection.selected
    }

    pub fn select(&mut self, entity: Option<Entity>) {
        self.selection.select(entity);
    }

    pub fn hover(&mut self, entity: Option<Entity>) {
        self.selection.hover(entity);
    }

    pub fn rows(&self, scene: &Scene) -> Vec<Row> {
        self.hierarchy.rows(scene)
    }

    pub fn set_expanded(&mut self, entity: Entity, expanded: bool) {
        self.hierarchy.set_expanded(entity, expanded);
    }

    pub fn toggle_expanded(&mut self, entity: Entity) {
        self.hierarchy.toggle_expanded(entity);
    }

    pub fn properties(&self, scene: &Scene) -> Vec<Property> {
        match self.selection.selected {
            Some(entity) => self.inspector.properties(scene, entity),
            None => Vec::new(),
        }
    }

    /// Edits the selected entity's property and records it for undo.
    pub fn set_property(&mut self, scene: &mut Scene, property: &Property, value: PropertyValue) {
        let Some(entity) = self.selection.selected else {
            return;
        };
        let before = property.value(scene, entity);
        if before == value {
            return;
        }
        self.inspector.set(scene, entity, property, value.clone());
        self.history.push(Command::SetProperty {
            entity,
            component: property.component,
            property: property.name,
            before,
            after: value,
        });
    }

    pub fn rename(&mut self, scene: &mut Scene, entity: Entity, name: String) {
        let before = tag_name(scene, entity);
        if name.is_empty() || name == before {
            return;
        }
        set_tag(scene, entity, &name);
        self.inspector.mark_dirty(entity);
        self.history.push(Command::Rename {
            entity,
            before,
            after: name,
        });
    }

    pub fn create_entity(&mut self, scene: &mut Scene, parent: Option<Entity>) -> Entity {
        let entity = scene.spawn(EMPTY_ENTITY);
        if let Some(parent) = parent {
            scene.set_parent(entity, Some(parent));
            self.hierarchy.set_expanded(parent, true);
        }
        self.selection.select(Some(entity));
        self.inspector.mark_dirty(entity);
        self.history.push(Command::CreateEntity {
            entity,
            parent,
            name: EMPTY_ENTITY.to_string(),
        });
        entity
    }

    /// Destroys the entity and its subtree. A selection inside that subtree is
    /// dropped first: the C++ panel kept it and the properties panel then drew
    /// a dangling handle.
    pub fn destroy_entity(&mut self, scene: &mut Scene, entity: Entity) {
        let parent = scene.parent(entity);
        let name = tag_name(scene, entity);
        if let Some(selected) = self.selection.selected {
            if is_within_subtree(scene, entity, selected) {
                self.selection.select(None);
            }
        }
        scene.destroy(entity);
        self.history.push(Command::DestroyEntity {
            entity,
            parent,
            name,
        });
    }

    /// Re-parents an entity. Refused when the target is inside the entity's own
    /// subtree, which would make a cycle.
    pub fn reparent(&mut self, scene: &mut Scene, entity: Entity, parent: Option<Entity>) -> bool {
        if let Some(parent) = parent {
            if !self.hierarchy.can_drop(scene, entity, parent) {
                return false;
            }
        }
        let before = scene.parent(entity);
        if before == parent {
            return true;
        }
        scene.set_parent(entity, parent);
        self.history.push(Command::Reparent {
            entity,
            before,
            after: parent,
        });
        true
    }

    pub fn play(&mut self, scene: &mut Scene) {
        self.play.play(scene);
    }

    pub fn pause(&mut self) {
        self.play.pause();
    }

    pub fn resume(&mut self) {
        self.play.resume();
    }

    pub fn stop(&mut self, scene: &mut Scene) {
        self.play.stop(scene);
    }

    /// Queues exactly one fixed step; only a paused editor honours it.
    pub fn step(&mut self) {
        self.play.step_once();
    }

    /// Advances the play state and the frame timings, returning the seconds to
    /// simulate. Property edits are flushed here too: a transform written by
    /// the panel has to reach the world transforms before the renderer reads
    /// them this frame.
    pub fn tick(&mut self, scene: &mut Scene, dt: f32) -> f32 {
        self.stats.update(dt);
        if !self.inspector.dirty().is_empty() {
            scene.update_transforms();
            self.inspector.clear_dirty();
        }
        self.play.tick(dt)
    }

    pub fn undo(&mut self, scene: &mut Scene) -> bool {
        let undone = self.history.undo(scene, &mut self.inspector);
        if undone {
            self.forget_dead_selection(scene);
        }
        undone
    }

    pub fn redo(&mut self, scene: &mut Scene) -> bool {
        let redone = self.history.redo(scene, &mut self.inspector);
        if redone {
            self.forget_dead_selection(scene);
        }
        redone
    }

    /// Replaces the scene with one loaded from disk, resolving its asset paths
    /// against the registry. A missing asset is a warning.
    pub fn open_scene(
        &mut self,
        scene: &mut Scene,
        assets: &mut vault::Assets,
        path: &Path,
    ) -> meats::Result<()> {
        let loaded = meats::load(path)?;
        self.reset(scene);
        *scene = loaded;
        for error in scene.resolve_assets(assets) {
            log::warn!("mirror: {error}");
        }
        self.history.clear();
        Ok(())
    }

    pub fn save_scene(&self, scene: &Scene, path: &Path) -> meats::Result<()> {
        meats::save(scene, path)
    }

    pub fn new_scene(&mut self, scene: &mut Scene) {
        self.reset(scene);
        scene.clear();
        self.history.clear();
    }

    fn reset(&mut self, scene: &mut Scene) {
        self.play.stop(scene);
        self.selection.clear();
        self.hierarchy.reset();
        self.inspector.clear_dirty();
        self.gizmo.end();
    }

    fn forget_dead_selection(&mut self, scene: &Scene) {
        if let Some(selected) = self.selection.selected {
            if scene.get(selected).is_none() {
                self.selection.select(None);
            }
        }
    }
}

/// The C++ `Scene::CreateEntity("Empty Entity")`.
const EMPTY_ENTITY: &str = "Empty Entity";

pub struct MirrorPlugin {
    editor: Editor,
}

impl Default for MirrorPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl MirrorPlugin {
    pub fn new() -> Self {
        Self {
            editor: Editor::new(),
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }
}

impl Plugin for MirrorPlugin {
    fn name(&self) -> &'static str {
        "mirror"
    }

    /// No GPU work and no window: the editor model is built here and the panel
    /// state is what the toolkit layer renders.
    fn build(&mut self, _eng: &mut Engine) -> adam::Result<()> {
        self.editor.theme = match Theme::load(Path::new(theme::DEFAULT_PATH)) {
            Ok(theme) => theme,
            Err(error) => {
                log::warn!("mirror: {error}; using the default theme");
                Theme::new()
            }
        };
        Ok(())
    }

    fn on_frame(&mut self, eng: &mut Engine, ctx: &mut FrameCtx) {
        self.editor.tick(&mut eng.scene, ctx.delta.as_secs_f32());
    }
}
