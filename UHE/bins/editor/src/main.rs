//! The editor binary: gpui over `mirror`'s model.
//!
//! This replaces `UHE/src/UHE/ImGui/ImGuiLayer.cpp` and `UHE_EDITOR/src`'s ImGui
//! panels. The model those panels read is `mirror`'s; the engine loop they drove
//! is `adam`'s, advanced from gpui's frame callback (`app::next_frame`). The layout
//! is fixed — a resizable hierarchy on the left, viewport in the middle, inspector
//! on the right, content browser below, toolbar above and status bar under it.
//! Docking and floating windows are not ported.

mod app;
mod panels;
mod theme;
mod viewport;

use gpui::{px, size, App, Bounds, TitlebarOptions, WindowBounds, WindowOptions};

fn main() {
    env_logger::init();

    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("unhuman".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|_| app::EditorApp::new());
                view.update(cx, |this, cx| this.start_frame_loop(window, cx));
                view
            },
        )
        .expect("open the editor window");

        cx.activate(true);
    });
}
