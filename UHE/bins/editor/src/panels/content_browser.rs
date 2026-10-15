//! The bottom panel: the asset tree.
//!
//! The C++ `ContentBrowserPanel::OnImGuiRender` was a two-column table — a
//! directory tree on the left, a thumbnail grid of the current directory on the
//! right, with a breadcrumb trail of "back" buttons. `mirror::content_browser`
//! owns the root, the current directory, the filter and the selection; this draws
//! the breadcrumbs, the entries and the navigation.
//!
//! The thumbnails are names, not pictures: previews went through
//! `Texture2D::Create` on a decoded PNG, and gpui's image elements load through
//! gpui's own asset cache, not `vault`. The filter box is a label for the same
//! reason — gpui has no text input widget in this crate, only the text renderer.

use gpui::{div, prelude::*, px, ClickEvent, Context, IntoElement, SharedString};
use mirror::content_browser::Entry;
use mirror::Editor;

use crate::app::EditorApp;
use crate::panels::{body, placeholder, with_alpha};
use crate::theme::EditorTheme;

const CELL_WIDTH: f32 = 132.0;
const CELL_HEIGHT: f32 = 28.0;

pub fn panel(
    editor: &Editor,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    let browser = &editor.content_browser;
    let entries = browser.entries();
    let breadcrumbs = browser.breadcrumbs();
    let selected = browser.selected.clone();

    let mut crumbs: Vec<gpui::AnyElement> = Vec::with_capacity(breadcrumbs.len() + 1);
    crumbs.push(crumb(0, "Assets", None, theme, cx));
    for (index, (name, path)) in breadcrumbs.iter().enumerate() {
        crumbs.push(crumb(index + 1, name, Some(path), theme, cx));
    }

    let mut cells: Vec<gpui::AnyElement> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        cells.push(cell(index, entry, selected.as_deref(), theme, cx).into_any_element());
    }

    body()
        .bg(theme.panel)
        .child(
            div()
                .flex_none()
                .h(px(24.0))
                .px_1()
                .flex()
                .items_center()
                .gap_1()
                .border_b_1()
                .border_color(theme.border)
                .text_xs()
                .text_color(theme.text_dim)
                .child("CONTENT BROWSER")
                .children(crumbs),
        )
        .child(
            div()
                .id("content-browser-entries")
                .flex()
                .flex_wrap()
                .content_start()
                .gap_1()
                .p_1()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .when(entries.is_empty(), |grid| {
                    grid.child(placeholder("Empty directory", theme))
                })
                .children(cells),
        )
}

fn crumb(
    index: usize,
    name: &str,
    path: Option<&std::path::PathBuf>,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    let owned = path.cloned();
    let label = SharedString::from(name.to_string());
    let element = div()
        .id(("crumb", index))
        .flex_none()
        .px_1()
        .rounded_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(theme.row_hover))
        .child(label);

    let Some(path) = owned else {
        return element;
    };

    element.on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
        this.editor.content_browser.navigate(&path);
        cx.notify();
    }))
}

fn cell(
    index: usize,
    entry: &Entry,
    selected: Option<&str>,
    theme: &EditorTheme,
    cx: &mut Context<EditorApp>,
) -> impl IntoElement {
    let is_selected = selected == Some(entry.path.as_str());
    let path = entry.path.clone();
    let is_dir = entry.is_dir;
    let hover = theme.row_hover;

    let (bg, text) = if is_selected {
        (theme.accent_muted, theme.accent_foreground)
    } else {
        (with_alpha(theme.panel, 0.0), theme.text)
    };

    let label = if is_dir {
        format!("▸ {}", entry.name)
    } else if entry.extension.is_empty() {
        entry.name.clone()
    } else {
        format!("{} .{}", entry.name, entry.extension)
    };

    div()
        .id(("content-browser-cell", index))
        .flex_none()
        .w(px(CELL_WIDTH))
        .h(px(CELL_HEIGHT))
        .px_1()
        .flex()
        .items_center()
        .rounded_sm()
        .text_sm()
        .cursor_pointer()
        .bg(bg)
        .text_color(text)
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
            let typed_path = std::path::PathBuf::from(&path);
            if is_dir {
                this.editor.content_browser.navigate(&typed_path);
            } else {
                this.editor.content_browser.selected = Some(path);
            }
            cx.notify();
        }))
        .child(div().min_w_0().truncate().child(SharedString::from(label)))
}
