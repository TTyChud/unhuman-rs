//! The window, and the loop that owns the process.
//!
//! `winit` 0.30 runs the event loop itself, so the engine cannot own a `while`
//! loop the way the C++ did (`glfwPollEvents` inside `WindowsWindow::OnUpdate`).
//! A frame is a callback here, and that is the whole shape of this file: an
//! [`ApplicationHandler`] that creates one window on `resumed` and calls the
//! caller's closure once per frame.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pings::Event;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{CursorGrabMode, Window, WindowAttributes, WindowId};

use crate::translate;

/// What the window is created with: the C++ `WindowProps`, plus the vsync flag
/// the C++ kept in its window data and the swapchain is what consumes.
pub struct WindowDesc {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
}

impl Default for WindowDesc {
    fn default() -> Self {
        Self {
            title: String::from("UHE ENGINE"),
            width: 1920,
            height: 1080,
            vsync: false,
        }
    }
}

/// Why the window layer gave up.
#[derive(Debug)]
pub enum WindowError {
    /// The loop could not be created, or failed while running.
    EventLoop(winit::error::EventLoopError),
    /// The window could not be created once the loop resumed.
    Os(winit::error::OsError),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowError::EventLoop(e) => write!(f, "event loop: {e}"),
            WindowError::Os(e) => write!(f, "window: {e}"),
        }
    }
}

impl std::error::Error for WindowError {}

impl From<winit::error::EventLoopError> for WindowError {
    fn from(e: winit::error::EventLoopError) -> Self {
        WindowError::EventLoop(e)
    }
}

impl From<winit::error::OsError> for WindowError {
    fn from(e: winit::error::OsError) -> Self {
        WindowError::Os(e)
    }
}

/// The handful of window operations the engine asks for.
///
/// A trait rather than an inherent impl because `winit::window::Window` is not
/// this crate's type, and re-exporting it rather than wrapping it is what lets
/// the wgpu backend hand the same `Arc<Window>` to `create_surface`. `set_title`
/// and `request_redraw` are already inherent methods on that type and are not
/// repeated here.
pub trait WindowExt {
    /// The window's inner size in physical pixels.
    fn size(&self) -> (u32, u32);

    /// winit's own `scale_factor` returns `f64` and wins method resolution on
    /// the re-exported type; this is the `f32` the engine works in.
    fn scale_factor(&self) -> f32;

    fn request_close(&self);

    /// Hide and grab the cursor, the C++ `SetCursorLocked`, for FPS mouse look.
    fn set_cursor_locked(&self, locked: bool);
}

impl WindowExt for Window {
    fn size(&self) -> (u32, u32) {
        let size = self.inner_size();
        (size.width, size.height)
    }

    fn scale_factor(&self) -> f32 {
        let factor: f64 = Window::scale_factor(self);
        factor as f32
    }

    /// winit 0.30 cannot close a window: the loop is the process, and a window
    /// is destroyed by dropping it. Hiding it is the closest thing the window
    /// has, and `run_windowed` stops once its window is no longer visible.
    fn request_close(&self) {
        self.set_visible(false);
    }

    fn set_cursor_locked(&self, locked: bool) {
        let grabbed = if locked {
            self.set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.set_cursor_grab(CursorGrabMode::Confined))
        } else {
            self.set_cursor_grab(CursorGrabMode::None)
        };
        if let Err(error) = grabbed {
            log::warn!("cursor grab: {error}");
        }
        self.set_cursor_visible(!locked);
    }
}

/// One frame, as the window layer sees it.
///
/// The events are here in order and go no further: the caller pushes them onto
/// the bus, `adam::App::frame` applies them to the one `Input`, and nothing in
/// this crate keeps a second copy of the input state.
pub struct FrameView<'a> {
    pub window: &'a Arc<Window>,
    pub events: &'a [Event],
    pub elapsed: Duration,
}

/// Runs the event loop until the frame closure returns `false`, or the window
/// is gone. Headless never calls this and never constructs a window.
///
/// The loop is `winit`'s, so frames are delivered, not stepped: exactly one per
/// wake, on `RedrawRequested` when the window asked for one, and on
/// `about_to_wait` when it did not, so a platform that never delivers a redraw
/// still advances. A frame ends by asking for the next one.
pub fn run_windowed(
    desc: WindowDesc,
    frame: impl FnMut(&mut FrameView<'_>) -> bool + 'static,
) -> Result<(), WindowError> {
    let event_loop = EventLoop::new()?;
    let mut app = EyesApp {
        desc,
        window: None,
        frame,
        pending: Vec::new(),
        start: Instant::now(),
        error: None,
        frames_this_wake: 0,
    };

    event_loop.run_app(&mut app)?;

    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The descriptor as `winit` wants it, kept out of the loop so the mapping is
/// testable with no display.
fn attributes(desc: &WindowDesc) -> WindowAttributes {
    Window::default_attributes()
        .with_title(desc.title.clone())
        .with_inner_size(PhysicalSize::new(desc.width, desc.height))
}

struct EyesApp<F> {
    /// Kept until `resumed`, which is the first moment a window may exist.
    desc: WindowDesc,
    window: Option<Arc<Window>>,
    frame: F,
    /// Translated events waiting for the next frame.
    pending: Vec<Event>,
    start: Instant,
    error: Option<WindowError>,
    frames_this_wake: u32,
}

impl<F> EyesApp<F> {
    fn start_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        match event_loop.create_window(attributes(&self.desc)) {
            Ok(window) => {
                log::info!(
                    "window {}: {}x{}, vsync {}",
                    self.desc.title,
                    self.desc.width,
                    self.desc.height,
                    self.desc.vsync
                );
                let window = Arc::new(window);
                window.request_redraw();
                self.window = Some(window);
            }
            Err(error) => {
                log::error!("could not create the window: {error}");
                self.error = Some(WindowError::Os(error));
                event_loop.exit();
            }
        }
    }
}

impl<F: FnMut(&mut FrameView<'_>) -> bool> EyesApp<F> {
    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        self.frames_this_wake += 1;

        let Some(window) = self.window.clone() else {
            return;
        };

        let events = std::mem::take(&mut self.pending);
        let elapsed = self.start.elapsed();

        let mut view = FrameView {
            window: &window,
            events: &events,
            elapsed,
        };
        let keep_going = (self.frame)(&mut view);

        if !keep_going || window.is_visible() == Some(false) {
            event_loop.exit();
            return;
        }

        window.request_redraw();
    }
}

impl<F: FnMut(&mut FrameView<'_>) -> bool> ApplicationHandler for EyesApp<F> {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        self.frames_this_wake = 0;
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.start_window(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::RedrawRequested => self.run_frame(event_loop),
            event => translate::events(&event, &mut self.pending),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.frames_this_wake == 0 {
            self.run_frame(event_loop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::Size;

    #[test]
    fn a_descriptors_title_and_size_reach_the_window_attributes() {
        let desc = WindowDesc {
            title: String::from("aimlab"),
            width: 800,
            height: 600,
            vsync: true,
        };
        let expected = PhysicalSize::new(800, 600);

        let attrs = attributes(&desc);

        assert_eq!(attrs.title, "aimlab");
        assert_eq!(attrs.inner_size, Size::Physical(expected));
    }

    #[test]
    fn the_default_descriptor_asks_for_the_c_plus_plus_default_size() {
        let expected = PhysicalSize::new(1920, 1080);

        let attrs = attributes(&WindowDesc::default());

        assert_eq!(attrs.title, "UHE ENGINE");
        assert_eq!(attrs.inner_size, Size::Physical(expected));
    }
}
