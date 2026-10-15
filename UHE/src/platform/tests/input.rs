//! `Input` is the only thing that consumes `pings::Event`, so these are the
//! tests of what a frame can ask about: held state survives, per-frame state
//! does not, and nothing a frame presses is lost when it is released in the
//! same frame.

use eyes::Input;
use pings::{Event, KeyCode, MouseButton};

fn press(key: KeyCode) -> Event {
    Event::KeyPressed { key, repeat: false }
}

fn release(key: KeyCode) -> Event {
    Event::KeyReleased { key }
}

#[test]
fn a_key_is_held_until_it_is_released_and_pressed_for_one_frame() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&press(KeyCode::W));
    assert!(input.is_key_pressed(KeyCode::W));
    assert!(input.is_key_held(KeyCode::W));
    assert!(!input.is_key_released(KeyCode::W));
    input.end_frame();

    input.begin_frame();
    assert!(!input.is_key_pressed(KeyCode::W));
    assert!(input.is_key_held(KeyCode::W));
    input.apply(&release(KeyCode::W));
    assert!(input.is_key_released(KeyCode::W));
    assert!(!input.is_key_held(KeyCode::W));
    input.end_frame();

    input.begin_frame();
    assert!(!input.is_key_pressed(KeyCode::W));
    assert!(!input.is_key_held(KeyCode::W));
    assert!(!input.is_key_released(KeyCode::W));
}

#[test]
fn a_key_pressed_and_released_in_one_frame_is_both() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&press(KeyCode::Space));
    input.apply(&release(KeyCode::Space));

    assert!(input.is_key_pressed(KeyCode::Space));
    assert!(input.is_key_released(KeyCode::Space));
    assert!(!input.is_key_held(KeyCode::Space));
}

#[test]
fn a_repeat_is_a_held_key_not_a_new_press() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&Event::KeyPressed {
        key: KeyCode::A,
        repeat: true,
    });

    assert!(input.is_key_held(KeyCode::A));
    assert!(!input.is_key_pressed(KeyCode::A));
}

#[test]
fn applying_the_same_event_twice_is_one_press() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&press(KeyCode::R));
    input.apply(&press(KeyCode::R));
    input.apply(&Event::MouseButtonPressed(MouseButton::Left));
    input.apply(&Event::MouseButtonPressed(MouseButton::Left));

    assert!(input.is_key_pressed(KeyCode::R));
    assert!(input.is_mouse_pressed(MouseButton::Left));
    input.begin_frame();
    assert!(!input.is_key_pressed(KeyCode::R));
    assert!(!input.is_mouse_pressed(MouseButton::Left));
}

#[test]
fn a_mouse_button_holds_between_frames() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&Event::MouseButtonPressed(MouseButton::Right));
    assert!(input.is_mouse_pressed(MouseButton::Right));
    assert!(input.is_mouse_held(MouseButton::Right));
    input.end_frame();

    input.begin_frame();
    assert!(!input.is_mouse_pressed(MouseButton::Right));
    assert!(input.is_mouse_held(MouseButton::Right));

    input.apply(&Event::MouseButtonReleased(MouseButton::Right));
    assert!(!input.is_mouse_held(MouseButton::Right));
}

#[test]
fn scroll_accumulates_in_a_frame_and_resets_between_them() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&Event::MouseScrolled { dx: 1.0, dy: 2.0 });
    input.apply(&Event::MouseScrolled { dx: 0.5, dy: -1.0 });
    assert_eq!(input.scroll(), (1.5, 1.0));
    input.end_frame();

    input.begin_frame();
    assert_eq!(input.scroll(), (0.0, 0.0));
}

#[test]
fn a_move_updates_the_cursor_and_this_frames_delta() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&Event::MouseMoved { x: 100.0, y: 50.0 });
    assert_eq!(input.cursor(), (100.0, 50.0));

    input.apply(&Event::MouseMoved { x: 110.0, y: 45.0 });
    assert_eq!(input.cursor(), (110.0, 45.0));
    assert_eq!(input.cursor_delta(), (110.0, 45.0));
    input.end_frame();

    input.begin_frame();
    assert_eq!(input.cursor(), (110.0, 45.0));
    assert_eq!(input.cursor_delta(), (0.0, 0.0));

    input.apply(&Event::MouseMoved { x: 112.0, y: 45.0 });
    assert_eq!(input.cursor_delta(), (2.0, 0.0));
}

#[test]
fn an_event_with_no_input_state_leaves_the_state_alone() {
    let mut input = Input::new();

    input.begin_frame();
    input.apply(&press(KeyCode::LeftShift));
    input.apply(&Event::WindowResized {
        width: 800,
        height: 600,
    });
    input.apply(&Event::WindowFocused(false));
    input.apply(&Event::TextInput(String::from("wasd")));
    input.apply(&Event::FrameBegin);
    input.apply(&Event::FrameEnd);

    assert!(input.is_key_held(KeyCode::LeftShift));
    assert!(input.is_key_pressed(KeyCode::LeftShift));
    assert_eq!(input.cursor(), (0.0, 0.0));
    assert_eq!(input.scroll(), (0.0, 0.0));
}
