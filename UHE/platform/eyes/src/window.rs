//! The window traits.
//!
//! ## Why this is a trait and not a struct
//!
//! Two implementations, and the second is the reason. [`crate::HeadlessWindow`]
//! opens no window, creates no event loop and needs no display, so the whole
//! frame loop can be tested in CI. A concrete window type cannot be tested
//! anywhere without a compositor.
//!
//! ## Why there is no `static mut WINDOW`
//!
//! The C++ has `Application` as a singleton with `Input` reaching through it to
//! a raw pointer to the window. Nothing can be constructed in isolation, nothing
//! can be two-at-once, and every test needs a display. So a window is *passed
//! in* -- `&mut dyn Window` alongside `&mut dyn Clock` and `&mut dyn RenderScope`
//! -- and the composition root that owns it is `adam`'s problem.

use raw_window_handle::RawWindowHandle;

use crate::config::WindowConfig;
use crate::event::Event;
use crate::input::InputState;
use crate::Result;

/// Why a raw handle is not available.
///
/// `raw_window_handle::HandleError` has no room for a reason, and "unsupported"
/// is not an answer when the real answer is "there is no window".
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HandleError {
    /// A headless window, which has no platform handle and never will.
    Headless,
    /// The platform refused: no windowing system, or the handle was requested
    /// from a thread the platform does not allow.
    Platform(&'static str),
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandleError::Headless => {
                f.write_str("this window is headless and has no platform handle")
            }
            HandleError::Platform(reason) => {
                write!(f, "the platform refused to give a window handle: {reason}")
            }
        }
    }
}

impl std::error::Error for HandleError {}

/// What one pump call produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PumpOutcome {
    /// Events delivered into the input state.
    pub delivered: usize,
    /// Whether the application should stop after this frame.
    pub should_close: bool,
}

impl PumpOutcome {
    /// Nothing delivered, and the loop should stop.
    pub fn close() -> Self {
        Self {
            delivered: 0,
            should_close: true,
        }
    }
}

/// Something that delivers events into an [`InputState`].
pub trait EventPump {
    /// Drain queued events into `input`.
    ///
    /// ## The contract, and it is two rules not one
    ///
    /// 1. Call [`InputState::begin_frame`] *first*, then deliver every queued
    ///    event, then set `should_close` if a close was requested.
    /// 2. Return, do not block. A pump that waits for the next event has
    ///    turned a render loop into an input loop, and the frame rate becomes
    ///    the keyboard's repeat rate.
    ///
    /// Both are one line each and both are the difference between a testable
    /// input path and a broken one.
    fn pump(&mut self, input: &mut InputState) -> PumpOutcome;

    /// Events queued but not yet delivered, for diagnostics.
    fn queued(&self) -> usize;
}

/// What one frame's callback is given.
///
/// Separate from [`Window`] because the callback needs the *state* of this
/// frame and nothing else -- not `set_title`, not `raw_handle`, not the ability
/// to pump again. A frame that can pump is a frame that can deliver input a
/// second time in the same frame, which is how "key pressed" ends up true
/// twice in one frame.
pub trait Frame {
    /// This frame's input, already filled with everything the platform sent
    /// before this frame began.
    fn input(&self) -> &InputState;
    /// The drawable area for this frame, in physical pixels.
    fn extent(&self) -> (u32, u32);
    /// Physical pixels per logical pixel, as of this frame.
    fn scale_factor(&self) -> f32;
    /// Vsync as requested. The swapchain is what honours it -- see
    /// [`Window::set_vsync`].
    fn vsync(&self) -> bool;
    /// Whether the window is on its way out. The loop still finishes this
    /// frame; a frame callback that checks this can skip the work.
    fn should_close(&self) -> bool;
    /// Ask for another frame.
    ///
    /// Meaningless under `Poll`, and rate-limited by the compositor under
    /// `WaitUntil`, which is why it exists as a separate call rather than as
    /// the only way to get a frame.
    fn request_redraw(&mut self);
    /// Ask to close, from inside a frame.
    fn request_close(&mut self);
}

/// A window.
pub trait Window: EventPump {
    /// The config this window was opened with. Changes to it since then are
    /// not reflected -- the platform is the authority, not this record.
    fn config(&self) -> &WindowConfig;

    /// The drawable area in physical pixels. This is the swapchain's size and
    /// it is never the config's size: the user can resize it, the OS can place
    /// the window on a display with a different scale factor, and the two only
    /// agree on the frame the window opened.
    fn extent(&self) -> (u32, u32);

    /// Physical pixels per logical pixel.
    fn scale_factor(&self) -> f32;

    /// Change the title bar text.
    fn set_title(&mut self, title: &str);

    /// Whether vsync is currently requested.
    ///
    /// ## Why a getter exists
    ///
    /// Because the C++ has `SetVSync` storing a bool and never calling
    /// `glfwSwapInterval`, so vsync was a no-op that *looked* like it worked:
    /// the flag read back true and no frame ever waited. A setter with no
    /// observable effect is indistinguishable from a setter that works, until
    /// something measures it. With a getter, and a test that sets and reads
    /// back, "did this actually do anything" is answerable without a GPU.
    fn vsync(&self) -> bool;

    /// Request vsync, or stop requesting it.
    ///
    /// **This cannot make vsync happen.** Vsync is the swapchain's present
    /// mode; no windowing library exposes it as a window property. What this
    /// records is the *request*, which the swapchain honours when it is next
    /// built, and which the frame loop uses to decide whether to rate-limit
    /// itself.
    ///
    /// The C++ has a `SetVSync` that stores a bool and never calls
    /// `glfwSwapInterval`, so vsync was a no-op that read back as enabled. The
    /// difference here is that [`Window::vsync`] reports something in effect.
    fn set_vsync(&mut self, enabled: bool);

    /// Ask the window to close. Cooperative: the platform may ignore it.
    fn request_close(&mut self);

    /// Whether the window has been asked to close.
    fn should_close(&self) -> bool;

    /// The raw platform handle.
    ///
    /// This is how **one** window serves both backends. winit implements
    /// `HasWindowHandle`, so the same window yields an `ash` surface (via
    /// `ash_window`) and a `wgpu::Surface` (via `SurfaceTarget::Window`), and
    /// `eyes` itself never names a graphics API -- it hands over a handle and
    /// the backend decides what it is.
    ///
    /// `eyes` therefore depends on `raw-window-handle` but not on `ash` or
    /// `wgpu`. Putting a `wgpu::Surface` here would be the same mistake as
    /// naming `vk::` in `bifrost`, one layer earlier.
    ///
    /// ## Windows
    ///
    /// Winit restricts raw handle access to the thread that created the
    /// window. Both backends want the surface on a frame thread, so the
    /// winit implementation calls `window_handle_any_thread()` before
    /// returning this. A backend that forgets fails on Windows only, and only
    /// when the frame thread differs from the creation thread, which is why
    /// that call is inside `eyes` and not at the call site.
    ///
    /// `Err` rather than a dummy handle: a backend handed a fabricated
    /// `RawWindowHandle` would fail later, inside `ash_window` or inside wgpu,
    /// with a message naming neither this crate nor the reason. The C++
    /// headless path works because the headless surface extension is a real
    /// surface; there is no equivalent for wgpu, and this error is the honest
    /// way to say so.
    fn raw_handle(&self) -> std::result::Result<RawWindowHandle, HandleError>;

    /// Deliver one event by hand, without a pump.
    ///
    /// Used by [`crate::HeadlessWindow`] and by tests, and by nothing else: a
    /// real window's events come from the platform.
    fn push_event(&mut self, event: Event);

    /// Run until the window closes, calling `frame` once per frame.
    ///
    /// ## Why the window owns the loop
    ///
    /// Because on the platforms that matter it has to. winit's `EventLoop` is
    /// not `Send` -- it holds the platform's connection -- so it cannot be
    /// moved to a render thread, and on macOS and iOS it must own the main
    /// thread outright. A design that spawns the event loop and hands the frame
    /// thread a channel does not compile against winit 0.30.
    ///
    /// ## What this makes impossible
    ///
    /// **Input arriving a frame late.** The C++ polls input after rendering, so
    /// every input lands one frame late for the entire session -- a drag is
    /// permanently off by a frame and nobody can say why. Here the callback
    /// runs only after this frame's events are in the input state, so "input
    /// this frame" means what it says. That is a structural guarantee, not a
    /// convention for the loop to remember.
    ///
    /// ## The callback
    ///
    /// It borrows everything else it needs -- the GPU, the scene, the clock --
    /// through the closure's captures. `Err` stops the loop and comes back out
    /// of `run`, so a frame that cannot render ends the program rather than
    /// spinning on a broken frame forever.
    fn run(&mut self, frame: &mut dyn FnMut(&mut dyn Frame) -> Result<()>) -> Result<()>;
}
