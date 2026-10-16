//! The editor application: the gpui root view, the layout, and the single place
//! the engine loop is advanced.
//!
//! ## One thread, one loop
//!
//! The engine frame and the gpui frame are the same frame. gpui owns the main
//! thread — its platform run loop calls back into this view for every drawn frame
//! — and the engine is advanced from that callback (`next_frame`), so there is no
//! second thread, no channel, and no lock between the model the panels read and
//! the scene the engine writes. That is only possible because this is the *only*
//! owner of both: `adam::App` and `mirror::Editor` are fields of one struct, and
//! `Engine`'s data is never borrowed by two of them at once.
//!
//! The engine is built headless, because the window is gpui's. See
//! `crate::viewport` for what that leaves the viewport showing.

use std::ffi::CStr;

use gpui::{
    canvas, div, prelude::*, px, Context, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Render, Window,
};
use mirror::gizmo::GizmoMode;
use mirror::play_state::PlayStatus;
use mirror::Editor;

use crate::panels::{self, button};
use crate::theme::EditorTheme;
use crate::viewport;

const ENGINE_NAME: &CStr = c"unhuman-editor";

const TOOLBAR_HEIGHT: f32 = 34.0;
const STATUS_HEIGHT: f32 = 22.0;
const SPLITTER: f32 = 5.0;

const MIN_HIERARCHY: f32 = 150.0;
const MAX_HIERARCHY: f32 = 520.0;
const MIN_INSPECTOR: f32 = 190.0;
const MAX_INSPECTOR: f32 = 620.0;
const MIN_CONTENT: f32 = 70.0;
const MAX_CONTENT: f32 = 460.0;

pub struct EditorApp {
    pub(crate) engine: adam::App,
    pub(crate) editor: Editor,
    theme: EditorTheme,
    theme_source: mirror::theme::Theme,
    backend: Option<String>,
    layout: Layout,
}

impl EditorApp {
    pub fn new() -> Self {
        let mut engine = adam::Engine::new();
        engine.assets.mount("assets");

        let arguments: Vec<String> = std::env::args().collect();
        let environment = std::env::var("UHE_BACKEND").ok();
        let backend = match engine::create_gpu_from_process(
            &arguments,
            environment.as_deref(),
            ENGINE_NAME,
            engine::default_headless_extent(),
        ) {
            Ok((gpu, _selection)) => {
                let name = gpu.info().name.clone();
                log::info!("editor renderer: {name}");
                engine.gpu = Some(gpu);
                Some(name)
            }
            Err(error) => {
                log::warn!("no GPU backend ({error}); the engine runs with gpu: None");
                None
            }
        };

        let mut app = adam::App::with_engine(engine).add(bifrost::BifrostPlugin::new());
        if let Err(error) = app.start() {
            log::error!("plugin build failed: {error}");
        }

        let editor = Editor::new();
        let theme_source = editor.theme;

        Self {
            engine: app,
            editor,
            theme: EditorTheme::from(&theme_source),
            theme_source,
            backend,
            layout: Layout::default(),
        }
    }

    /// Schedules the first engine frame. Every frame re-arms this one, so the
    /// engine advances at the rate the platform draws.
    pub fn start_frame_loop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.on_next_frame(window, next_frame);
    }

    /// One engine frame: the editor's play state decides how much simulation
    /// runs, then `adam::App` runs it.
    ///
    /// The delta comes from the engine's own clock (`Time::delta`, the interval
    /// the last frame was ticked with), so there is one clock and not two.
    ///
    /// `Time::scale` is what makes pause mean pause. `mirror::PlayState` reports
    /// the seconds this frame should simulate — `0.0` when paused or stopped, one
    /// fixed step after `step_once` — and a scale of zero keeps those seconds out
    /// of the fixed-step accumulator, so pausing freezes the simulation instead of
    /// queueing a catch-up burst, while the render half of the frame keeps
    /// running. `mirror` emits whole fixed steps and `adam` consumes whole fixed
    /// steps from the seconds it is handed, so the substep count stays the
    /// engine's decision.
    fn advance_frame(&mut self) {
        let delta = self.engine.engine().time.delta().as_secs_f32();
        let scene = &mut self.engine.engine_mut().scene;
        let seconds = self.editor.tick(scene, delta);
        self.engine
            .engine_mut()
            .time
            .set_scale(if seconds > 0.0 { 1.0 } else { 0.0 });

        if let Err(error) = self.engine.frame() {
            log::error!("engine frame failed: {error}");
        }

        if self.editor.theme != self.theme_source {
            self.theme_source = self.editor.theme;
            self.theme = EditorTheme::from(&self.theme_source);
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let status = self.editor.play.status;
        let mode = self.editor.gizmo.mode;

        div()
            .flex_none()
            .h(px(TOOLBAR_HEIGHT))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .bg(theme.toolbar)
            .border_b_1()
            .border_color(theme.border)
            .child(button(
                "toolbar-play",
                "Play",
                status == PlayStatus::Playing,
                &theme,
                cx,
                |this, _window, cx| {
                    let scene = &mut this.engine.engine_mut().scene;
                    this.editor.play(scene);
                    cx.notify();
                },
            ))
            .child(button(
                "toolbar-pause",
                "Pause",
                status == PlayStatus::Paused,
                &theme,
                cx,
                |this, _window, cx| {
                    this.editor.pause();
                    cx.notify();
                },
            ))
            .child(button(
                "toolbar-step",
                "Step",
                false,
                &theme,
                cx,
                |this, _window, cx| {
                    this.editor.step();
                    cx.notify();
                },
            ))
            .child(button(
                "toolbar-stop",
                "Stop",
                status == PlayStatus::Stopped,
                &theme,
                cx,
                |this, _window, cx| {
                    let scene = &mut this.engine.engine_mut().scene;
                    this.editor.stop(scene);
                    cx.notify();
                },
            ))
            .child(separator(&theme))
            .child(button(
                "toolbar-move",
                GizmoMode::Translate.name(),
                mode == GizmoMode::Translate,
                &theme,
                cx,
                |this, _window, cx| {
                    this.editor.gizmo.mode = GizmoMode::Translate;
                    cx.notify();
                },
            ))
            .child(button(
                "toolbar-rotate",
                GizmoMode::Rotate.name(),
                mode == GizmoMode::Rotate,
                &theme,
                cx,
                |this, _window, cx| {
                    this.editor.gizmo.mode = GizmoMode::Rotate;
                    cx.notify();
                },
            ))
            .child(button(
                "toolbar-scale",
                GizmoMode::Scale.name(),
                mode == GizmoMode::Scale,
                &theme,
                cx,
                |this, _window, cx| {
                    this.editor.gizmo.mode = GizmoMode::Scale;
                    cx.notify();
                },
            ))
            .child(div().flex_1())
            .child(panels::dim_label(
                viewport::backend_label(self.backend.as_deref()),
                &theme,
            ))
            .child(panels::dim_label(
                viewport::fps_label(self.editor.stats.fps),
                &theme,
            ))
    }

    fn status_bar(&self) -> impl IntoElement {
        let theme = self.theme;
        let entities = self.engine.engine().scene.entities().len();

        div()
            .flex_none()
            .h(px(STATUS_HEIGHT))
            .px_2()
            .flex()
            .items_center()
            .gap_3()
            .bg(theme.status)
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.text_dim)
            .child(play_label(self.editor.play.status))
            .child(self.editor.gizmo.mode.name())
            .child(entity_label(entities))
            .child(viewport::frame_status(
                self.backend.as_deref(),
                self.editor.stats.fps,
                self.editor.stats.frame_ms,
            ))
            .child(div().flex_1())
            .child(self.theme_source.name())
    }

    /// A drag handle. The bar itself only starts the drag; the move and release
    /// listeners live in its `canvas` paint callback, which is the one place gpui
    /// lets a view register a window-level mouse listener — an element's own
    /// `on_mouse_move` stops firing the moment the pointer leaves the 5px bar.
    fn splitter(&self, splitter: Splitter, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let dragging = self
            .layout
            .drag
            .is_some_and(|drag| drag.splitter == splitter);
        let entity = cx.entity();

        let element = div()
            .id(splitter.id())
            .flex_none()
            .bg(if dragging { theme.accent } else { theme.border })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.layout.begin_drag(
                        splitter,
                        event.position.x.as_f32(),
                        event.position.y.as_f32(),
                    );
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |_bounds, _, window, _cx| {
                        window.on_mouse_event({
                            let entity = entity.clone();
                            move |event: &MouseMoveEvent, _phase, _window, cx| {
                                if !event.dragging() {
                                    return;
                                }
                                entity.update(cx, |this, cx| {
                                    let resized = this.layout.drag_to(
                                        event.position.x.as_f32(),
                                        event.position.y.as_f32(),
                                    );
                                    if resized {
                                        cx.notify();
                                    }
                                });
                            }
                        });
                        window.on_mouse_event(move |_: &MouseUpEvent, _phase, _window, cx| {
                            entity.update(cx, |this, cx| {
                                if this.layout.end_drag() {
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .size_full(),
            );

        match splitter.axis() {
            SplitterAxis::Width => element.w(px(SPLITTER)).h_full().cursor_ew_resize(),
            SplitterAxis::Height => element.h(px(SPLITTER)).w_full().cursor_ns_resize(),
        }
    }
}

impl Render for EditorApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let scene = &self.engine.engine().scene;
        let hierarchy_width = self.layout.hierarchy_width;
        let inspector_width = self.layout.inspector_width;
        let content_height = self.layout.content_height;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .child(self.toolbar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex_none()
                            .h_full()
                            .w(px(hierarchy_width))
                            .child(panels::hierarchy::panel(&self.editor, scene, &theme, cx)),
                    )
                    .child(self.splitter(Splitter::Hierarchy, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(panels::viewport::panel(&theme)),
                    )
                    .child(self.splitter(Splitter::Inspector, cx))
                    .child(
                        div()
                            .flex_none()
                            .h_full()
                            .w(px(inspector_width))
                            .child(panels::inspector::panel(&self.editor, scene, &theme, cx)),
                    ),
            )
            .child(self.splitter(Splitter::Content, cx))
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .h(px(content_height))
                    .child(panels::content_browser::panel(&self.editor, &theme, cx)),
            )
            .child(self.status_bar())
    }
}

impl Default for EditorApp {
    fn default() -> Self {
        Self::new()
    }
}

/// One gpui frame is one engine frame: advance the engine, mark the view dirty,
/// and ask for the frame after this one.
fn next_frame(this: &mut EditorApp, window: &mut Window, cx: &mut Context<EditorApp>) {
    this.advance_frame();
    cx.notify();
    cx.on_next_frame(window, next_frame);
}

fn separator(theme: &EditorTheme) -> impl IntoElement {
    div().flex_none().w(px(1.0)).h(px(18.0)).bg(theme.border)
}

pub fn play_label(status: PlayStatus) -> &'static str {
    match status {
        PlayStatus::Stopped => "stopped",
        PlayStatus::Playing => "playing",
        PlayStatus::Paused => "paused",
    }
}

pub fn entity_label(count: usize) -> String {
    if count == 1 {
        "1 entity".to_string()
    } else {
        format!("{count} entities")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Splitter {
    Hierarchy,
    Inspector,
    Content,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SplitterAxis {
    Width,
    Height,
}

impl Splitter {
    fn id(self) -> &'static str {
        match self {
            Splitter::Hierarchy => "splitter-hierarchy",
            Splitter::Inspector => "splitter-inspector",
            Splitter::Content => "splitter-content",
        }
    }

    fn axis(self) -> SplitterAxis {
        match self {
            Splitter::Hierarchy | Splitter::Inspector => SplitterAxis::Width,
            Splitter::Content => SplitterAxis::Height,
        }
    }
}

#[derive(Clone, Copy)]
struct Drag {
    splitter: Splitter,
    origin: (f32, f32),
    start: f32,
}

pub struct Layout {
    hierarchy_width: f32,
    inspector_width: f32,
    content_height: f32,
    drag: Option<Drag>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            hierarchy_width: 240.0,
            inspector_width: 300.0,
            content_height: 160.0,
            drag: None,
        }
    }
}

impl Layout {
    fn value(&self, splitter: Splitter) -> f32 {
        match splitter {
            Splitter::Hierarchy => self.hierarchy_width,
            Splitter::Inspector => self.inspector_width,
            Splitter::Content => self.content_height,
        }
    }

    fn begin_drag(&mut self, splitter: Splitter, x: f32, y: f32) -> bool {
        if self.drag.is_some() {
            return false;
        }
        self.drag = Some(Drag {
            splitter,
            origin: (x, y),
            start: self.value(splitter),
        });
        true
    }

    /// Moves the dragged edge to follow the pointer. Returns whether the layout
    /// actually changed, so a frame that moves nothing does not redraw.
    fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };

        let (dx, dy) = (x - drag.origin.0, y - drag.origin.1);
        let moved = match drag.splitter {
            Splitter::Hierarchy => (drag.start + dx).clamp(MIN_HIERARCHY, MAX_HIERARCHY),
            // The inspector's edge moves left as its width grows, and the content
            // browser's moves up as its height grows.
            Splitter::Inspector => (drag.start - dx).clamp(MIN_INSPECTOR, MAX_INSPECTOR),
            Splitter::Content => (drag.start - dy).clamp(MIN_CONTENT, MAX_CONTENT),
        };

        let slot = match drag.splitter {
            Splitter::Hierarchy => &mut self.hierarchy_width,
            Splitter::Inspector => &mut self.inspector_width,
            Splitter::Content => &mut self.content_height,
        };
        if *slot == moved {
            return false;
        }
        *slot = moved;
        true
    }

    fn end_drag(&mut self) -> bool {
        self.drag.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_status_reads_as_a_word() {
        assert_eq!(play_label(PlayStatus::Stopped), "stopped");
        assert_eq!(play_label(PlayStatus::Playing), "playing");
        assert_eq!(play_label(PlayStatus::Paused), "paused");
    }

    #[test]
    fn entity_count_is_plural_where_it_should_be() {
        assert_eq!(entity_label(0), "0 entities");
        assert_eq!(entity_label(1), "1 entity");
        assert_eq!(entity_label(2), "2 entities");
    }

    #[test]
    fn a_drag_follows_the_pointer_on_its_own_edge() {
        let mut layout = Layout::default();
        assert!(layout.begin_drag(Splitter::Hierarchy, 240.0, 10.0));
        assert!(layout.drag_to(300.0, 10.0));
        assert_eq!(layout.hierarchy_width, 300.0);

        let mut layout = Layout::default();
        layout.begin_drag(Splitter::Inspector, 900.0, 10.0);
        assert!(layout.drag_to(830.0, 10.0));
        assert_eq!(layout.inspector_width, 370.0);

        let mut layout = Layout::default();
        layout.begin_drag(Splitter::Content, 10.0, 700.0);
        assert!(layout.drag_to(10.0, 640.0));
        assert_eq!(layout.content_height, 220.0);
    }

    #[test]
    fn a_drag_stops_at_its_limits() {
        let mut layout = Layout::default();
        layout.begin_drag(Splitter::Hierarchy, 0.0, 0.0);
        assert!(layout.drag_to(-10_000.0, 0.0));
        assert_eq!(layout.hierarchy_width, MIN_HIERARCHY);
        assert!(layout.drag_to(10_000.0, 0.0));
        assert_eq!(layout.hierarchy_width, MAX_HIERARCHY);

        layout.end_drag();
        assert!(!layout.drag_to(500.0, 0.0));
    }

    #[test]
    fn a_move_with_nothing_dragged_does_nothing() {
        let mut layout = Layout::default();
        assert!(!layout.drag_to(400.0, 400.0));
        assert_eq!(layout.hierarchy_width, 240.0);
        assert_eq!(layout.content_height, 160.0);
    }

    #[test]
    fn a_second_press_does_not_steal_the_drag() {
        let mut layout = Layout::default();
        assert!(layout.begin_drag(Splitter::Hierarchy, 0.0, 0.0));
        assert!(!layout.begin_drag(Splitter::Content, 0.0, 0.0));
        assert!(layout.end_drag());
        assert!(!layout.end_drag());
    }
}
