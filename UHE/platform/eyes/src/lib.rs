//! `eyes` — window, surface handle, input.
//!
//! The only crate in the workspace that names a windowing system, and the only
//! one that knows what a window *is*. Everything above it takes `&mut dyn
//! Window` and hands its frame callback a `&mut dyn Frame`.
//!
//! ## What is here
//!
//! | module | concern |
//! |---|---|
//! | [`config`] | window settings. Data only, no windowing types. |
//! | [`keys`] | platform-independent keys, buttons, modifiers. |
//! | [`event`] | what happened. Facts, not meanings. |
//! | [`input`] | polled state, updated by delivering events. |
//! | [`window`] | the traits a frame loop depends on. |
//! | [`headless`] | a window that opens nothing, for tests. |
//! | [`winit_window`] | the real window. |
//! | [`winit_loop`] | the event loop it runs in. |
//! | [`winit_config`] | size units, fullscreen modes, the thread rule. |
//! | [`winit_event`] | winit events to ours. A pure table. |
//! | [`winit_keymap`] | winit key codes to ours. A pure table. |
//!
//! Splitting those out of [`winit_window`] is not tidiness. The two pure tables
//! hold no platform calls, so they are the only part of the winit backend that
//! can be tested without a window; the loop is separated because it is the half
//! that owns state, and folding it back in produced one file that was half a
//! window and half a control-flow machine.
//!
//! ## The four decisions worth knowing
//!
//! **It depends on `raw-window-handle`, not on `ash` or `wgpu`.** One window
//! serves both backends, because winit hands out a raw platform handle and each
//! backend builds its own surface from it. Putting a `wgpu::Surface` here would
//! be the same mistake as naming `vk::` in `bifrost`, one layer earlier -- and
//! it would mean `eyes` could not be built for the backend that does not want
//! it.
//!
//! **It depends on `winit` 0.30 and `raw-window-handle` 0.6 together.** That
//! pairing is load-bearing: winit 0.30 implements rwh_06 and wgpu 30 asks for
//! rwh_06, so one version bridges all three. A mismatch fails deep inside
//! `create_surface`, naming neither cause.
//!
//! **The window owns the frame loop, not the other way round.** winit's
//! `EventLoop` is not `Send` and on macOS must own the main thread, so it
//! cannot be moved to a render thread. [`Window::run`] therefore hands control
//! to winit and invokes the frame callback from inside it.
//!
//! **`raw_handle` returns `Result`, not a handle.** A backend handed a
//! fabricated handle fails later, inside a graphics API, with a message naming
//! neither this crate nor the reason. [`HeadlessWindow`] returns
//! [`HandleError::Headless`] and says so.
//!
//! ## The frame loop
//!
//! ```no_run
//! use eyes::{Frame, Window};
//!
//! fn run(mut window: Box<dyn Window>) -> eyes::Result<()> {
//!     window.run(&mut |frame: &mut dyn Frame| {
//!         let input = frame.input();
//!         if input.binding_allowed() && input.key_pressed(eyes::Key::Escape) {
//!             frame.request_close();
//!         }
//!         // ... record and submit using frame.extent() ...
//!         Ok(())
//!     })
//! }
//! ```
//!
//! Note what is *not* there: no call to pump input, and no separate render call.
//! Events have already been delivered into `frame.input()` by the time the
//! callback runs. The C++ polls input after rendering, so every input arrives a
//! frame late for the entire session; making the ordering impossible to get
//! wrong beats documenting it.
//!
//! ## Not implemented
//!
//! No clipboard, no cursor shapes, no multi-monitor placement, no fullscreen
//! mode switching after creation. The C++ has a `SetVSync` that does nothing
//! (vsync is a swapchain property and winit has no API for it), a
//! `SetWindowSize` that ignores the scale factor, and input polled one frame
//! late. Those are the bugs this crate exists to not have.

#![deny(missing_docs)]

pub mod config;
pub mod event;
pub mod headless;
pub mod input;
pub mod keys;
pub mod window;
pub mod winit_config;
pub mod winit_event;
pub mod winit_keymap;
pub mod winit_loop;
pub mod winit_window;

pub use config::{WindowConfig, WindowMode, WindowSize};
pub use event::Event;
pub use headless::HeadlessWindow;
pub use input::{InputState, MouseState};
pub use keys::{Key, Modifiers, MouseButton};
pub use window::{EventPump, Frame, HandleError, PumpOutcome, Window};
pub use winit_window::WinitWindow;

/// `Result` for anything in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything that can go wrong in `eyes`.
#[derive(Debug)]
pub enum Error {
    /// The platform refused to open a window or start an event loop. The
    /// message is kept because "no display server" and "permission denied" have
    /// opposite fixes, and winit reports both as an opaque error string.
    Window(String),
    /// [`Window::run`] was called twice. The event loop is consumed by the first
    /// call and cannot be run again, so this is reported rather than ignored --
    /// a silently ignored second `run` would leave a program that renders one
    /// frame and then appears to hang.
    AlreadyRun,
}

impl Error {
    pub(crate) fn window(message: String) -> Self {
        Error::Window(message)
    }

    pub(crate) fn already_run() -> Self {
        Error::AlreadyRun
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Window(message) => write!(f, "{message}"),
            Error::AlreadyRun => {
                f.write_str("this window's event loop has already been run; run() consumes it")
            }
        }
    }
}

impl std::error::Error for Error {}
