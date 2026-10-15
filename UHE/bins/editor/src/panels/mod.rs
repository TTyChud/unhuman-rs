//! The editor panels, one file per panel, each a function over `mirror`'s model.
//!
//! A panel does not own state: it reads `mirror::Editor` and the engine's scene
//! for the frame and writes back only through `mirror`'s methods, from the gpui
//! callbacks it installs. That is the same split `ImGuiLayer.cpp` had — the panels
//! drew from `Scene`/`Editor` and mutated through it — with the toolkit's
//! retained widgets in place of ImGui's immediate mode.

pub mod content_browser;
pub mod hierarchy;
pub mod inspector;
pub mod viewport;

use gpui::{
    div, prelude::*, px, ClickEvent, Context, Div, Hsla, IntoElement, SharedString, Window,
};

use crate::app::EditorApp;
use crate::theme::EditorTheme;

pub const ROW_HEIGHT: f32 = 22.0;

/// Dimmed text for a panel with nothing to show.
pub fn placeholder(text: impl Into<SharedString>, theme: &EditorTheme) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(theme.text_dim)
        .child(text.into())
}

/// A panel's body: full size, a column, clipped to the panel.
pub fn body() -> Div {
    div().flex().flex_col().size_full().overflow_hidden()
}

pub fn button(
    id: &'static str,
    label: impl Into<SharedString>,
    active: bool,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
    on_click: impl Fn(&mut EditorApp, &mut Window, &mut Context<EditorApp>) + 'static,
) -> impl IntoElement {
    let (bg, fg, hover) = if active {
        (theme.accent, theme.accent_foreground, theme.accent_hover)
    } else {
        (
            with_alpha(theme.panel, 0.0),
            theme.text,
            with_alpha(theme.accent, 0.25),
        )
    };

    div()
        .id(id)
        .flex_none()
        .h(px(20.0))
        .px_2()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(bg)
        .text_color(fg)
        .text_xs()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label.into())
        .on_click(cx.listener(move |this, _event: &ClickEvent, window, cx| {
            on_click(this, window, cx)
        }))
}

/// A theme color with a different alpha: an unselected row is the panel with no
/// fill of its own, so hovering it can show one.
pub fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        h: color.h,
        s: color.s,
        l: color.l,
        a: alpha,
    }
}
