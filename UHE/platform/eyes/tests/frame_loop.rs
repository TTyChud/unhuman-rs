//! The frame loop, run headless.
//!
//! The C++ polls input *after* rendering, so every input arrives one frame late
//! for the entire session. Nothing notices, because there is no way to write a
//! test that would notice. These can.

use std::cell::RefCell;

use eyes::{Event, EventPump, Frame, HeadlessWindow, InputState, Key, Window};

/// Runs `window`'s loop and records what each frame saw.
///
/// A `RefCell` because the frame callback gets `&mut dyn Frame`, and the
/// recorder has to be reachable from the closure that owns it.
fn record_frames(window: &mut HeadlessWindow, frames: usize) -> Vec<bool> {
    let seen = RefCell::new(Vec::with_capacity(frames));
    let mut remaining = frames;

    let _ = window.run(&mut |frame: &mut dyn Frame| {
        remaining -= 1;
        seen.borrow_mut().push(frame.input().key_pressed(Key::KeyA));
        if remaining == 0 {
            frame.request_close();
        }
        Ok(())
    });

    seen.into_inner()
}

#[test]
fn a_frame_sees_the_input_that_arrived_before_it() {
    let mut window = HeadlessWindow::new();

    // One event, queued before the loop starts -- as if the platform delivered
    // it while the previous frame was being drawn.
    window.push_event(Event::KeyPressed {
        key: Key::KeyA,
        text: Some('a'),
        repeat: false,
        modifiers: eyes::Modifiers::NONE,
    });

    let seen = record_frames(&mut window, 3);
    assert_eq!(
        seen,
        vec![true, false, false],
        "the first frame must already have the key; a frame loop that polls \
         input after rendering puts it on the second"
    );
}

#[test]
fn the_loop_stops_when_the_frame_asks_it_to() {
    let mut window = HeadlessWindow::new();
    let seen = record_frames(&mut window, 3);
    assert_eq!(seen.len(), 3, "and it stops, rather than spinning");
    assert!(window.should_close());
}

#[test]
fn a_close_event_ends_the_loop_without_asking() {
    let mut window = HeadlessWindow::new();
    window.push_event(Event::CloseRequested);

    let mut frames = 0;
    let _ = window.run(&mut |_frame: &mut dyn Frame| {
        frames += 1;
        Ok(())
    });

    assert!(
        frames <= 2,
        "a close must not leave the loop running: {frames} frames"
    );
}

#[test]
fn the_frame_sees_the_extent_the_resize_produced() {
    let mut window = HeadlessWindow::new();
    window.resize(2560, 1440);

    let mut seen_extent = (0, 0);
    let _ = window.run(&mut |frame: &mut dyn Frame| {
        seen_extent = frame.extent();
        frame.request_close();
        Ok(())
    });

    assert_eq!(
        seen_extent,
        (2560, 1440),
        "a frame must not see the previous frame's extent, or a swapchain \
         rebuilds against a stale size and renders stretched"
    );
}

#[test]
fn a_frame_error_stops_the_loop_and_comes_back_out() {
    let mut window = HeadlessWindow::new();
    let mut frames = 0;

    let result: eyes::Result<()> = window.run(&mut |_frame: &mut dyn Frame| {
        frames += 1;
        Err(eyes::Error::Window("no device".to_string()))
    });

    assert!(result.is_err(), "a broken frame must not be swallowed");
    assert_eq!(frames, 1, "and it must not spin on the failure");
}

#[test]
fn input_state_is_shared_across_frames_not_rebuilt() {
    // A key held across a frame boundary must still be held in the next one.
    // A per-frame `InputState` would make holding a key impossible, which is
    // how "the camera only moves while you tap" happens.
    let mut window = HeadlessWindow::new();
    let mut input = InputState::default();

    window.push_event(Event::KeyPressed {
        key: Key::Shift,
        text: None,
        repeat: false,
        modifiers: eyes::Modifiers::NONE,
    });
    window.pump(&mut input);
    window.pump(&mut input);

    assert!(
        input.key_down(Key::Shift),
        "held across two pumps, so it is still held"
    );
    assert!(
        !input.key_pressed(Key::Shift),
        "but it is no longer a fresh press"
    );
}
