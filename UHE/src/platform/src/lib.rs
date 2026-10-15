//! Window and input. `winit` 0.30, re-exported, and nothing else.
//!
//! The C++ had a `Window` interface with one implementation per platform
//! (`Platform/Windows/WindowsWindow.cpp`), an `Input` that polled GLFW
//! (`WindowInput.cpp`), and — on Windows — a file dialog. Nothing in a window is
//! platform code once `winit` exists, so this crate is the whole of what the
//! C++ tree needed two platform directories for.
//!
//! Two things are deliberately absent. There is no global: the loop is a value,
//! the window is `Arc`-shared with whatever draws into it, and the input state
//! is a plain field of `adam::Engine`. And there is no second copy of the input
//! state: `winit` events become `pings::Event`s, the bus carries them to
//! [`Input::apply`], and every query reads what `apply` wrote.
//!
//! Headless never constructs a `Window` and never calls [`run_windowed`].

mod input;
mod translate;
mod window;

pub use input::Input;
pub use window::{run_windowed, FrameView, WindowDesc, WindowError, WindowExt};

/// The window itself, shared. The wgpu backend is handed `Arc<Window>` and calls
/// `Instance::create_surface(window.clone())`, so no backend crate has to name a
/// windowing library — `raw-window-handle` 0.6 comes in with `winit`, and is what
/// wgpu 30 matches.
pub use winit::window::Window;

/// Re-exported for the same reason: a backend that needs to name a handle type
/// takes it from here rather than depending on `winit`.
pub use raw_window_handle;
