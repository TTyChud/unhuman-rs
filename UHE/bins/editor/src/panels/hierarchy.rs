//! The left panel: the entity tree.
//!
//! The C++ `SceneHierachyPanel::DrawEntityNode` drew one ImGui tree node per
//! entity with the tag as its label, the selected row in `ImGuiCol_Header` under
//! an accent tint, and a context menu holding "Create Empty Entity", "Unparent to
//! Root" and "Delete Entity". The tree, the selection and the expansion state are
//! `mirror::hierarchy`'s; the two actions that need a selected entity live in the
//! inspector, beside the entity they act on, because gpui has no built-in
//! per-item context menu.

use gpui::{div, prelude::*, px, AnyElement, ClickEvent, Context, IntoElement};
use meats::{Entity, Scene};
use mirror::hierarchy::Row;
use mirror::Editor;

use crate::app::EditorApp;
use crate::panels::{body, button, placeholder, with_alpha, ROW_HEIGHT};
use crate::theme::EditorTheme;

pub fn panel(
    editor: &Editor,
    scene: &Scene,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    let selected = editor.selection.selected;
    let rows = editor.rows(scene);

    let mut list: Vec<AnyElement> = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        list.push(row_element(index, row, selected, theme, cx).into_any_element());
    }

    body()
        .bg(theme.panel)
        .child(
            div()
                .flex_none()
                .h(px(26.0))
                .px_1()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(theme.border)
                .text_xs()
                .text_color(theme.text_dim)
                .child("ENTITIES")
                .child(button(
                    "hierarchy-create",
                    "+",
                    false,
                    theme,
                    cx,
                    |this, _window, cx| {
                        let scene = &mut this.engine.engine_mut().scene;
                        this.editor.create_entity(scene, None);
                        cx.notify();
                    },
                )),
        )
        .child(
            div()
                .id("hierarchy-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .py_1()
                .overflow_y_scroll()
                .when(rows.is_empty(), |list| {
                    list.child(placeholder("No entities. Use + to add one.", theme))
                })
                .children(list),
        )
}

fn row_element(
    index: usize,
    row: &Row,
    selected: Option<Entity>,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    let entity = row.entity;
    let is_selected = selected == Some(entity);

    let (row_bg, row_text) = if is_selected {
        (theme.accent_muted, theme.accent_foreground)
    } else {
        (with_alpha(theme.panel, 0.0), theme.text)
    };
    let hover_bg = theme.row_hover;
    let depth = 4.0 + row.depth as f32 * 12.0;

    div()
        .id(("hierarchy-row", index))
        .flex_none()
        .h(px(ROW_HEIGHT))
        .pl(px(depth))
        .pr_1()
        .flex()
        .items_center()
        .gap_1()
        .rounded_sm()
        .text_sm()
        .cursor_pointer()
        .bg(row_bg)
        .text_color(row_text)
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
            let scene = &mut this.engine.engine_mut().scene;
            this.editor.select(scene, Some(entity));
            cx.notify();
        }))
        .child(disclosure(index, row, theme, cx))
        .child(div().flex_1().min_w_0().truncate().child(row.name.clone()))
}

fn disclosure(
    index: usize,
    row: &Row,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    if !row.has_children {
        return div().w(px(10.0)).flex_none();
    }

    let entity = row.entity;
    let expanded = row.expanded;

    div()
        .id(("hierarchy-disclosure", index))
        .w(px(10.0))
        .flex_none()
        .flex()
        .justify_center()
        .text_color(theme.text_dim)
        .child(if expanded { "▾" } else { "▸" })
        .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
            this.editor.set_expanded(entity, !expanded);
            cx.notify();
        }))
}
