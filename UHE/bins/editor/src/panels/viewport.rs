//! The center panel: the engine's frame, or the reason there is not one.
//!
//! `mirror`'s gizmo is not drawn here either. `ImGuizmo` overlaid its handles on
//! the viewport image and picked against them with the mouse position in viewport
//! space; with no image there is no space to overlay, so the pick and the drag
//! (`mirror::gizmo::TransformGizmo`, `Camera { viewport_size }`) wait for the frame
//! the viewport cannot yet show. See `crate::viewport` for what is missing.

use gpui::{div, prelude::*, px, IntoElement};
use crate::theme::EditorTheme;
use crate::viewport::MISSING_FRAME_REASON;

pub fn panel(theme: &EditorTheme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .bg(theme.bg)
        .child(
            div()
                .text_xs()
                .text_color(theme.text_dim)
                .child("VIEWPORT"),
        )
        .child(
            div()
                .max_w(px(420.0))
                .text_sm()
                .text_center()
                .text_color(theme.text_dim)
                .child(MISSING_FRAME_REASON),
        )
}
