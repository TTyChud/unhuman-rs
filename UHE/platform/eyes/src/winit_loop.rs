//! The event loop, and the frame it drives.
//!
//! Its own file because it is the part of the winit backend that has *state*.
//! `winit_window` owns a window; this owns the loop that window runs in, and
//! keeping them together produced a 500-line file that was half window and half
//! control flow.
//!
//! ## The rule this file enforces
//!
//! Every event the platform sent is applied to the input state *before* the
//! frame callback runs, and the callback is only ever called from
//! [`LoopHandler::step`].
//!
//! The C++ polls input after rendering, so every input arrives one frame late
//! for the entire session. A drag is permanently off by a frame, nobody can say
//! why, and there is no test that could find it -- because the only way to
//! deliver a key in the C++ is to open a window. Here it is a single function
//! with a fixed order, and a test that pushes an event before the loop and
//! asserts the first frame sees it.

use std::cell::Cell;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent as WinitWindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};

use crate::event::Event;
use crate::input::InputState;
use crate::window::{Frame, Window};
use crate::winit_window::WinitWindow;
use crate::Result;

/// State for one run of the event loop.
pub(super) struct LoopHandler<'a> {
    window: &'a mut WinitWindow,
    callback: &'a mut dyn FnMut(&mut dyn Frame) -> Result<()>,
    vsync: bool,
    interval: Duration,
    input: InputState,
    queue: Vec<Event>,
    /// Set when the platform wants a frame.
    ///
    /// A `Cell` because it is written by [`Frame::request_redraw`] from inside
    /// the frame view and read by the loop, and handing the frame view `&mut`
    /// access to the handler would alias `self.window`, which the view already
    /// holds.
    redraw: Cell<bool>,
}

impl<'a> LoopHandler<'a> {
    pub(super) fn new(
        window: &'a mut WinitWindow,
        callback: &'a mut dyn FnMut(&mut dyn Frame) -> Result<()>,
        vsync: bool,
        interval: Duration,
    ) -> Self {
        // Redrawing on the very first `about_to_wait` rather than waiting for
        // the first scheduled tick: a window that shows nothing for one refresh
        // interval after opening looks like a program that did not start.
        Self {
            window,
            callback,
            vsync,
            interval,
            input: InputState::default(),
            queue: Vec::new(),
            redraw: Cell::new(true),
        }
    }

    /// Run one frame: every event the platform sent, then the callback.
    ///
    /// The order *inside this function* is the whole point. See the module
    /// docs.
    fn step(&mut self) -> Result<()> {
        self.input.begin_frame();

        for event in self.queue.drain(..) {
            self.input.apply(&event);
        }

        let mut view = WinitFrame {
            input: &self.input,
            extent: self.window.extent(),
            scale_factor: self.window.scale_factor(),
            vsync: self.vsync,
            redraw: &self.redraw,
            window: self.window,
        };
        (self.callback)(&mut view)
    }

    /// Decide when the next frame happens.
    fn schedule_next(&mut self, event_loop: &ActiveEventLoop) {
        if self.vsync {
            // Rescheduled from *now*, not from the previous deadline. Adding
            // the interval to the old deadline makes a frame that overran its
            // budget run immediately and then again, which is how a loop that
            // has already dropped to 30fps starts running frames it has no time
            // for -- and the frame rate gets worse the busier it gets.
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + self.interval));
        } else {
            // Uncapped. A render loop that a user has asked not to vsync wants
            // to go as fast as it can, and the swapchain is what actually tears
            // frames when it is faster than the display.
            event_loop.set_control_flow(ControlFlow::Poll);
        }
    }

    /// Queue an event for the next frame.
    fn push(&mut self, event: Event) {
        self.queue.push(event);
        self.redraw.set(true);
    }
}

impl ApplicationHandler for LoopHandler<'_> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Nothing to rebuild: there is no GL context to recreate.
        //
        // Recorded because `suspended`/`resumed` is how mobile platforms tear
        // down and hand back a surface, and an engine that handles the first and
        // not the second comes back to a blank screen with no error anywhere.
        self.redraw.set(true);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) {
        // `let ... else`, not a match with an empty arm. An event we do not
        // translate should be a visible no-op rather than an error, but the
        // list of what is not translated belongs in `winit_event` where it can
        // be read and added to.
        let Some(mapped) = crate::winit_event::map_window_event(&event) else {
            return;
        };

        if mapped.is_close() {
            // Latched, not just reported: `should_close()` and a local flag
            // that disagree is how a loop exits its body and keeps running.
            self.window.request_close();
            event_loop.exit();
            return;
        }

        // Geometry is mirrored before the event is queued, so the frame that
        // reacts to the resize sees the new extent rather than the old one.
        self.window.mirror(&mapped);
        self.push(mapped);
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        // Every cause but `Init` means a frame is due: `ResumeTimeReached` and
        // `WaitCancelled` are the two wakeups we asked for, and `Poll` means we
        // were spinning anyway.
        //
        // `ResumeTimeReached` is also how the platform says the app came back
        // from the background. A frame has to happen now rather than at the
        // next scheduled tick, or the first frame after resuming is dropped and
        // the window appears frozen for one interval.
        if !matches!(cause, StartCause::Init) {
            self.redraw.set(true);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // `about_to_wait` is winit's "every queued event has been delivered",
        // which is the only point at which a frame may be drawn. Anything
        // arriving after this belongs to the next frame and is delivered there,
        // which is what "a frame sees the input it had when it started" means
        // in practice.
        if !self.redraw.get() {
            return;
        }
        self.redraw.set(false);

        if let Err(error) = self.step() {
            // A frame that failed ends the program. Carrying on would spin at
            // full speed on a broken frame, producing a log nobody reads
            // instead of one error.
            eprintln!("eyes: a frame failed, shutting down: {error}");
            event_loop.exit();
            return;
        }

        if self.window.should_close() {
            event_loop.exit();
            return;
        }

        self.schedule_next(event_loop);
    }
}

/// What a winit frame sees.
///
/// The geometry is copied rather than read through `&self`, because a frame
/// callback holds `&mut dyn Frame` and the window, and reading the window's
/// mirror at that moment is a borrow of the same thing twice.
struct WinitFrame<'a> {
    window: &'a mut WinitWindow,
    input: &'a InputState,
    extent: (u32, u32),
    scale_factor: f32,
    vsync: bool,
    redraw: &'a Cell<bool>,
}

impl Frame for WinitFrame<'_> {
    fn input(&self) -> &InputState {
        self.input
    }

    fn extent(&self) -> (u32, u32) {
        self.extent
    }

    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    fn vsync(&self) -> bool {
        self.vsync
    }

    fn should_close(&self) -> bool {
        self.window.should_close()
    }

    fn request_redraw(&mut self) {
        self.redraw.set(true);
    }

    fn request_close(&mut self) {
        self.window.request_close();
    }
}
