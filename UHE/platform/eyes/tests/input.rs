//! Input state: the tests that run with no display, no GPU and no compositor.
//!
//! This is the file that justifies the crate. Every assertion below is about
//! behaviour the C++ could not be tested on at all, because the C++ polls the OS
//! and there is no way to make a key be down without a window.

use eyes::keys::Modifiers;
use eyes::{EventPump, HeadlessWindow, InputState, Key, MouseButton, Window};

/// A key-down with no character, which is what a modifier or a shortcut sends.
fn key_down(key: Key) -> eyes::Event {
    eyes::Event::KeyPressed {
        key,
        text: None,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

/// A key-down that also produced `letter`.
fn key_types(key: Key, letter: char) -> eyes::Event {
    eyes::Event::KeyPressed {
        key,
        text: Some(letter),
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn press(input: &mut InputState, window: &mut HeadlessWindow, key: Key) {
    window.push_event(key_down(key));
    window.pump(input);
}

#[test]
fn a_key_is_held_until_it_is_released() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());

    press(&mut input, &mut window, Key::KeyW);
    assert!(input.key_down(Key::KeyW));
    assert!(input.key_pressed(Key::KeyW));

    // Pumped again with nothing queued: held, but no longer a fresh press.
    // This is the distinction that stops a binding firing once per frame the
    // key is held.
    window.pump(&mut input);
    assert!(input.key_down(Key::KeyW));
    assert!(!input.key_pressed(Key::KeyW), "held is not pressed");

    window.push_event(eyes::Event::KeyReleased {
        key: Key::KeyW,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);
    assert!(!input.key_down(Key::KeyW));
    assert!(input.key_released(Key::KeyW));
}

#[test]
fn an_auto_repeat_is_not_a_press() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());

    window.push_event(key_down(Key::Space));
    window.pump(&mut input);
    assert!(input.key_pressed(Key::Space));

    // What the OS sends while the key is held.
    window.push_event(eyes::Event::KeyPressed {
        key: Key::Space,
        text: Some(' '),
        repeat: true,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);
    assert!(
        !input.key_pressed(Key::Space),
        "a repeat must not read as a new press"
    );
    assert!(input.key_down(Key::Space), "but it is still held");
}

#[test]
fn pressed_is_true_for_exactly_one_frame() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());
    // Queued, not delivered: `press` would consume the press itself and the
    // frame under test would never see it.
    window.push_event(key_down(Key::Escape));

    let mut frames_pressed = 0;
    for _ in 0..10 {
        window.pump(&mut input);
        if input.key_pressed(Key::Escape) {
            frames_pressed += 1;
        }
    }
    assert_eq!(frames_pressed, 1);
}

#[test]
fn text_accumulates_and_clears() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());

    window.push_event(key_types(Key::KeyH, 'h'));
    window.push_event(key_types(Key::KeyI, 'i'));
    window.pump(&mut input);
    assert_eq!(input.text(), "hi");

    // Cleared by the next pump, so a text field reads this frame's characters
    // and not everything typed since the program started.
    window.pump(&mut input);
    assert_eq!(input.text(), "");
}

#[test]
fn typing_suppresses_bindings() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());
    window.set_focused(true);
    window.pump(&mut input);
    assert!(input.binding_allowed());

    // A text field claims the keyboard.
    input.capture_keyboard();
    window.push_event(eyes::Event::KeyPressed {
        key: Key::KeyQ,
        text: Some('q'),
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);

    assert!(
        !input.binding_allowed(),
        "Q must not fire as a binding while a text field owns the keyboard"
    );
    assert_eq!(input.text(), "", "and the character must not be typed");

    input.release_keyboard();
    window.push_event(eyes::Event::KeyPressed {
        key: Key::KeyQ,
        text: Some('q'),
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);
    assert!(input.binding_allowed());
    assert_eq!(input.text(), "q");
}

#[test]
fn claims_are_depth_counted() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());
    window.set_focused(true);
    window.pump(&mut input);

    // A text field inside a modal that also wants the keyboard.
    input.capture_keyboard();
    input.capture_keyboard();
    assert!(input.keyboard_captured());

    input.release_keyboard();
    assert!(
        input.keyboard_captured(),
        "one release must not drop a claim someone else still holds"
    );
    input.release_keyboard();
    assert!(!input.keyboard_captured());

    // An unbalanced release saturates rather than wrapping.
    input.release_keyboard();
    assert!(!input.keyboard_captured());
    let _ = window;
}

#[test]
fn an_unfocused_window_takes_no_bindings() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());
    assert!(
        !input.focused(),
        "a new window does not have focus by default"
    );
    assert!(!input.binding_allowed());
    let _ = window.pump(&mut input);
}

#[test]
fn mouse_delta_accumulates_within_a_frame() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());

    window.push_event(eyes::Event::MouseMoved { x: 10.0, y: 20.0 });
    window.push_event(eyes::Event::MouseMoved { x: 15.0, y: 20.0 });
    window.pump(&mut input);

    assert_eq!(input.mouse().position, (15.0, 20.0));
    // Displacement from where the frame started, not per-move: two moves in one
    // frame sum to the same total as one. A camera integrating this gets the
    // same result either way; a camera subtracting positions itself gets the
    // first frame wrong, which is what this avoids.
    assert_eq!(input.mouse().delta, (15.0, 20.0));

    window.pump(&mut input);
    assert_eq!(input.mouse().delta, (0.0, 0.0), "delta is per frame");
    assert_eq!(input.mouse().position, (15.0, 20.0), "position is not");
}

#[test]
fn mouse_buttons_distinguish_held_from_pressed() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());

    window.push_event(eyes::Event::MouseButtonPressed {
        button: MouseButton::Left,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);
    assert!(input.mouse_down(MouseButton::Left));
    assert!(input.mouse_pressed(MouseButton::Left));

    window.pump(&mut input);
    assert!(input.mouse_down(MouseButton::Left));
    assert!(!input.mouse_pressed(MouseButton::Left));

    window.push_event(eyes::Event::MouseButtonReleased {
        button: MouseButton::Left,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);
    assert!(!input.mouse_down(MouseButton::Left));
    assert!(input.mouse_released(MouseButton::Left));
}

#[test]
fn scroll_accumulates_and_clears() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());

    for _ in 0..3 {
        window.push_event(eyes::Event::MouseScrolled {
            delta_x: 0.0,
            delta_y: 3.0,
            modifiers: Modifiers::NONE,
        });
    }
    window.pump(&mut input);
    assert_eq!(input.mouse().scroll, (0.0, 9.0));

    window.pump(&mut input);
    assert_eq!(input.mouse().scroll, (0.0, 0.0));
}

#[test]
fn an_unnamed_mouse_button_is_still_a_button() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());
    let back = MouseButton::Other(8);

    window.push_event(eyes::Event::MouseButtonPressed {
        button: back,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);

    // Dropped, it would be unbindable, and there would be nothing to report.
    assert!(input.mouse_down(back));
    assert!(input.mouse_pressed(back));
}

#[test]
fn a_resize_arrives_as_an_event_and_moves_the_extent() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());
    let before = window.extent();

    window.resize(1920, 1080);
    let outcome = window.pump(&mut input);

    assert_eq!(outcome.delivered, 1);
    assert_eq!(window.extent(), (1920, 1080));
    assert_ne!(window.extent(), before, "the extent must actually move");
}

#[test]
fn a_zero_sized_window_is_clamped() {
    let (mut window, _input) = (HeadlessWindow::new(), InputState::default());
    // What a minimise reports. A swapchain cannot be zero-sized.
    window.resize(0, 0);
    assert_eq!(window.extent(), (1, 1));
}

#[test]
fn a_close_request_ends_the_loop() {
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());

    window.push_event(eyes::Event::CloseRequested);
    let outcome = window.pump(&mut input);
    assert!(outcome.should_close);
    assert!(window.should_close());

    // And from the other direction.
    let (mut window, mut input) = (HeadlessWindow::new(), InputState::default());
    window.request_close();
    assert!(window.pump(&mut input).should_close);
}

#[test]
fn set_vsync_actually_changes_something() {
    let mut window = HeadlessWindow::new();
    let initial = window.vsync();

    window.set_vsync(!initial);
    assert_ne!(
        window.vsync(),
        initial,
        "the C++ SetVSync stored a bool and read back as enabled"
    );
    assert_eq!(window.vsync_set_count(), 1, "and the call must have run");
}

#[test]
fn a_headless_window_has_no_platform_handle() {
    let window = HeadlessWindow::new();
    // Not a fabricated handle: a backend handed one would fail later, inside a
    // graphics API, naming neither this crate nor the reason.
    let error = window.raw_handle().expect_err("headless has no handle");
    assert_eq!(error, eyes::HandleError::Headless);
    assert!(!error.to_string().is_empty());
}
