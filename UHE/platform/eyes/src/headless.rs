//! A window that opens nothing.
//!
//! This is the crate's reason for existing as a trait. Every test of a frame
//! loop, an input binding, a resize path or a swapchain rebuild runs against
//! this, in CI, with no display server and no compositor and no GPU.
//!
//! The C++ has no equivalent, which is why its windowing code is untested: the
//! Vulkan path gets away with it via `VK_EXT_headless_surface`, but there is no
//! headless equivalent for wgpu, and there is no way at all to deliver a
//! keypress without a window.

use std::collections::VecDeque;

use raw_window_handle::RawWindowHandle;

use crate::config::WindowConfig;
use crate::event::Event;
use crate::input::InputState;
use crate::window::{EventPump, Frame, HandleError, PumpOutcome, Window};
use crate::Result;

/// A fixed-size window with no platform behind it.
///
/// Events are pushed by hand and delivered through [`EventPump::pump`] into the
/// same [`InputState`] a real window fills, so a test exercises the code a real
/// frame runs.
#[derive(Debug)]
pub struct HeadlessWindow {
    config: WindowConfig,
    extent: (u32, u32),
    scale_factor: f32,
    vsync: bool,
    close_requested: bool,
    queue: VecDeque<Event>,
    /// Counts calls that actually ran, so a test can tell "set vsync to the
    /// value it already had" from "set vsync was never called".
    vsync_set_count: u32,
}

impl HeadlessWindow {
    /// A headless window with the default config.
    pub fn new() -> Self {
        Self::with_config(WindowConfig::default())
    }

    /// A headless window with a given config. The config's logical size
    /// becomes the extent, since nothing will change it later.
    pub fn with_config(config: WindowConfig) -> Self {
        let (width, height) = config.size.logical();
        Self {
            vsync: config.vsync,
            close_requested: false,
            scale_factor: 1.0,
            extent: (width, height),
            config,
            queue: VecDeque::new(),
            vsync_set_count: 0,
        }
    }

    /// Resize, as the platform would. The event arrives through the queue, so a
    /// test sees it exactly where it would in a real frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.extent = (width.max(1), height.max(1));
        self.queue.push_back(Event::Resized {
            width: self.extent.0,
            height: self.extent.1,
        });
    }

    /// Move to a display with a different scale, as the platform would.
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
        self.queue
            .push_back(Event::ScaleFactorChanged { scale_factor });
    }

    /// Give or take keyboard focus, as the platform would.
    pub fn set_focused(&mut self, focused: bool) {
        self.queue.push_back(Event::Focused(focused));
    }

    /// How many times [`Window::set_vsync`] was called.
    pub fn vsync_set_count(&self) -> u32 {
        self.vsync_set_count
    }
}

impl Default for HeadlessWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl EventPump for HeadlessWindow {
    fn pump(&mut self, input: &mut InputState) -> PumpOutcome {
        // Rule 1 of the pump contract: before delivery. So "pressed since the
        // last pump" stays true for exactly one frame no matter how the caller
        // interleaves pumps and reads.
        input.begin_frame();

        let mut outcome = PumpOutcome::default();
        while let Some(event) = self.queue.pop_front() {
            if event.is_close() {
                // Latched, not just reported. Reporting it from the outcome
                // alone would let `outcome.should_close` and `should_close()`
                // disagree, and a loop that checks one and exits on the other
                // stops.
                self.close_requested = true;
                outcome.should_close = true;
            }
            input.apply(&event);
            outcome.delivered += 1;
        }

        // A close asked for through `request_close` is a close, even though it
        // never became an event.
        outcome.should_close |= self.close_requested;
        outcome
    }

    fn queued(&self) -> usize {
        self.queue.len()
    }
}

impl Window for HeadlessWindow {
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
    }

    fn vsync(&self) -> bool {
        self.vsync
    }

    fn set_vsync(&mut self, enabled: bool) {
        self.vsync = enabled;
        self.vsync_set_count += 1;
    }

    fn request_close(&mut self) {
        self.close_requested = true;
    }

    fn should_close(&self) -> bool {
        self.close_requested
    }

    fn raw_handle(&self) -> std::result::Result<RawWindowHandle, HandleError> {
        Err(HandleError::Headless)
    }

    fn push_event(&mut self, event: Event) {
        self.queue.push_back(event);
    }

    fn run(&mut self, frame: &mut dyn FnMut(&mut dyn Frame) -> Result<()>) -> Result<()> {
        // The input state belongs to the loop, not to the window: a real one is
        // rebuilt from scratch each frame anyway, and `pump` takes it as an
        // argument precisely so a test can supply its own.
        let mut input = InputState::default();

        // A frame limit so a runaway callback cannot hang the test suite. The
        // real window has no such limit because a real window ends when the
        // platform says so.
        const MAX_FRAMES: u64 = 1_000_000;

        for _ in 0..MAX_FRAMES {
            self.pump(&mut input);

            // Checked before the frame, and the frame still runs once more, so
            // a close requested by the frame itself is honoured by the loop
            // rather than needing a second close.
            let closing = self.close_requested;
            let mut view = HeadlessFrame {
                window: self,
                input: &input,
            };
            frame(&mut view)?;

            if self.close_requested {
                break;
            }
            if closing && self.queue.is_empty() {
                break;
            }
        }
        Ok(())
    }
}

/// What a headless frame sees.
///
/// A separate struct rather than implementing [`Frame`] for `HeadlessWindow`
/// because a frame needs the *input* too, and the input is not a field of the
/// window. Borrowing both is the only way to have one.
struct HeadlessFrame<'a> {
    window: &'a mut HeadlessWindow,
    input: &'a InputState,
}

impl Frame for HeadlessFrame<'_> {
    fn input(&self) -> &InputState {
        self.input
    }

    fn extent(&self) -> (u32, u32) {
        self.window.extent()
    }

    fn scale_factor(&self) -> f32 {
        self.window.scale_factor()
    }

    fn vsync(&self) -> bool {
        self.window.vsync()
    }

    fn should_close(&self) -> bool {
        self.window.close_requested
    }

    fn request_redraw(&mut self) {
        // Nothing to request: a headless loop runs as fast as the callback
        // allows, which is what a test wants and what a real window does not.
    }

    fn request_close(&mut self) {
        self.window.request_close();
    }
}
