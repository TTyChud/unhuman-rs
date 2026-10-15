//! The winit implementation. The only file that names a windowing library.
//!
//! ## The window owns the loop
//!
//! winit's `EventLoop` is not `Send` -- it holds the platform's connection to
//! the display server -- and on macOS and iOS it must own the main thread
//! outright. So it cannot be moved to a render thread, and a design that
//! spawns the event loop and hands the frame thread a channel does not compile
//! against winit 0.30.
//!
//! Therefore [`Window::run`] hands control to `EventLoop::run_app` and the frame
//! callback is invoked from inside it. That is not a workaround. It is what
//! makes "this frame's input is already in the input state" a structural
//! property rather than a convention a loop has to remember.
//!
//! ## The two C++ bugs this file is built to not reproduce
//!
//! **Vsync is a swapchain property, not a window property.** winit has no API
//! for it -- `present_mode` belongs to the surface. The C++ put `SetVSync` on
//! the window, stored a bool, and never called `glfwSwapInterval`, so vsync was
//! a no-op that read back as enabled: indistinguishable from working, until
//! something measured it. Here the request changes the frame cadence, so
//! [`Window::vsync`] reads back something that is in effect.
//!
//! **Input must arrive before the frame is drawn.** The C++ polls input after
//! rendering, so every input lands a frame late for the whole session. That is
//! an ordering bug in the loop rather than in the input code, which is why the
//! ordering lives in one function here that no caller can reorder.

use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::event_loop::EventLoop;

use crate::config::WindowConfig;
use crate::event::Event;
use crate::input::InputState;
use crate::window::{EventPump, Frame, HandleError, PumpOutcome, Window};
use crate::winit_config::{allow_raw_handle_off_thread, apply_mode, build_attributes};
use crate::{Error, Result};

/// The frame cadence used when vsync is requested and the platform will not
/// report a refresh rate.
///
/// 60 Hz. A wrong-but-plausible number costs a fraction of a frame of latency;
/// spinning costs a whole core.
const DEFAULT_REFRESH: Duration = Duration::from_micros(16_666);

/// A real window.
///
/// Thread-affine: construct it, run it and drop it on one thread. That is
/// winit's requirement, not this crate's, and the reason there is no `Send`
/// bound anywhere in the crate.
pub struct WinitWindow {
    /// `None` before [`Window::run`] and permanently after it returns. An
    /// `EventLoop` is consumed by `run`, so this is a slot rather than a field.
    event_loop: Option<EventLoop<()>>,
    window: winit::window::Window,
    config: WindowConfig,
    vsync: bool,
    /// Mirrored from the platform, because `extent(&self)` cannot query it and
    /// a frame callback is handed `&mut dyn Frame`, not `&mut Window`.
    extent: (u32, u32),
    scale_factor: f32,
    close_requested: bool,
}

impl WinitWindow {
    /// Open a window.
    ///
    /// Fails when there is no windowing system at all -- headless CI, an SSH
    /// session with no X forwarding. That is a different failure from a window
    /// that opened and then misbehaves, and worth telling apart.
    pub fn new(config: WindowConfig) -> Result<Self> {
        let mut builder = EventLoop::builder();
        allow_raw_handle_off_thread(&mut builder);
        let event_loop = builder
            .build()
            .map_err(|error| Error::window(format!("no event loop ({error})")))?;

        // `EventLoop::create_window` rather than `ActiveEventLoop::create_window`,
        // because the latter exists only inside a running loop callback and
        // `new` has to report a failure synchronously. A window that cannot be
        // opened is a startup error, not something discovered three frames in.
        #[allow(deprecated)]
        let window = event_loop
            .create_window(build_attributes(&config))
            .map_err(|error| Error::window(format!("could not create a window ({error})")))?;

        apply_mode(&window, config.mode);

        let physical = window.inner_size();
        Ok(Self {
            vsync: config.vsync,
            extent: (physical.width, physical.height),
            scale_factor: window.scale_factor() as f32,
            event_loop: Some(event_loop),
            window,
            config,
            close_requested: false,
        })
    }

    /// Update the mirrored geometry from an event, before it is delivered.
    ///
    /// Before, so that a frame reading `extent()` sees this frame's resize and
    /// not the previous one -- which is precisely the one-frame-late bug, in a
    /// different costume. A swapchain rebuilt against a stale extent renders
    /// into the old size and stretches.
    pub(super) fn mirror(&mut self, event: &Event) {
        match event {
            Event::Resized { width, height } => self.extent = (*width, *height),
            Event::ScaleFactorChanged { scale_factor } => self.scale_factor = *scale_factor,
            _ => {}
        }
    }

    /// The platform's display refresh rate, used as the frame limit when vsync
    /// is on. `None` when the platform will not say, or says something absurd.
    fn refresh_interval(&self) -> Option<Duration> {
        let monitor = self
            .window
            .primary_monitor()
            .or_else(|| self.window.available_monitors().next())?;
        let rate = monitor.refresh_rate_millihertz()? / 1000;
        // 0 Hz, 1 Hz and 1000 Hz all occur on real hardware. A one-frame-per-
        // second loop is indistinguishable from a hang, so anything outside a
        // plausible range falls back to the default.
        if !(24..=1000).contains(&rate) {
            return None;
        }
        Some(Duration::from_micros(1_000_000 / u64::from(rate)))
    }
}

impl EventPump for WinitWindow {
    /// Not usable on a winit window.
    ///
    /// Events arrive inside `EventLoop::run_app`, so before `run` there is no
    /// queue to drain and during it this window's state belongs to the callback.
    ///
    /// It refuses rather than returning an empty outcome, because a `pump` that
    /// quietly did nothing would let a frame loop that got the order wrong
    /// compile, run, and take no input -- the exact bug this crate exists to
    /// prevent. A silent no-op here would reintroduce it invisibly.
    fn pump(&mut self, _input: &mut InputState) -> PumpOutcome {
        unimplemented!("a winit window delivers events inside Window::run; see EventPump")
    }

    fn queued(&self) -> usize {
        0
    }
}

impl Window for WinitWindow {
    fn config(&self) -> &WindowConfig {
        &self.config
    }

    fn extent(&self) -> (u32, u32) {
        self.extent
    }

    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    fn set_title(&mut self, title: &str) {
        self.config.title = title.to_string();
        self.window.set_title(title);
    }

    fn vsync(&self) -> bool {
        self.vsync
    }

    fn set_vsync(&mut self, enabled: bool) {
        // No winit call exists for this, because vsync belongs to the
        // swapchain's present mode. What this can do is change the frame
        // cadence, so the request is not a bool that does nothing: with vsync
        // off the loop stops rate-limiting and renders as fast as the callback
        // allows. The swapchain reads this when it is next built.
        self.vsync = enabled;
    }

    fn request_close(&mut self) {
        self.close_requested = true;
    }

    fn should_close(&self) -> bool {
        self.close_requested
    }

    fn raw_handle(&self) -> std::result::Result<RawWindowHandle, HandleError> {
        // The `any_thread` flag was set on the event loop when it was built,
        // which is where winit 0.30 takes it; there is no per-call variant.
        //
        // It matters: without it winit refuses a raw handle to any thread but
        // the one that created the window, and a frame callback is not that
        // thread. The failure is a panic inside `create_surface` naming neither
        // winit's rule nor the cause.
        self.window
            .window_handle()
            .map(|handle| handle.as_raw())
            .map_err(|_| HandleError::Platform("winit refused the window handle on this thread"))
    }

    fn push_event(&mut self, _event: Event) {
        // A real window's events come from the platform. Pushing one here would
        // go nowhere, and nothing would say so.
        unimplemented!("a winit window's events come from the platform, not from push_event")
    }

    fn run(&mut self, frame: &mut dyn FnMut(&mut dyn Frame) -> Result<()>) -> Result<()> {
        let event_loop = self.event_loop.take().ok_or(Error::already_run())?;

        // Read before `self` is borrowed by the handler, not after. Two lines
        // of ordering that the compiler enforces, so it cannot rot.
        let vsync = self.vsync;
        let interval = self.refresh_interval().unwrap_or(DEFAULT_REFRESH);

        // The loop's own state lives in `winit_loop`, so this file stays about
        // the window rather than about how it is driven.
        let mut handler = super::winit_loop::LoopHandler::new(self, frame, vsync, interval);

        event_loop
            .run_app(&mut handler)
            .map_err(|error| Error::window(format!("the event loop stopped: {error}")))?;
        Ok(())
    }
}
