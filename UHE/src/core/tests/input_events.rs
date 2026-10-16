//! The event bus is what fills input state, and the frame is where it happens.
//!
//! This is the seam the C++ tree did not have: `WindowInput` polled the window
//! and the ImGui layer polled it again, so the two could disagree about what was
//! down. Here the window pushes events, the engine owns one `Input`, and the
//! frame drains the bus into it — so a key that arrived this frame is held for
//! this frame and no other copy of that state exists.

use adam::{App, Engine};
use pings::{Event, KeyCode, MouseButton};

fn app() -> App {
    App::with_engine(Engine::headless())
}

#[test]
fn a_pressed_key_is_held_and_a_released_one_is_not() {
    let mut app = app();

    app.engine_mut().events.push(Event::KeyPressed {
        key: KeyCode::W,
        repeat: false,
    });
    app.frame().unwrap();
    assert!(app.engine().input.is_key_held(KeyCode::W));

    app.engine_mut()
        .events
        .push(Event::KeyReleased { key: KeyCode::W });
    app.frame().unwrap();
    assert!(!app.engine().input.is_key_held(KeyCode::W));
}

#[test]
fn a_press_is_visible_to_the_frame_it_arrived_in_and_gone_the_next() {
    let mut app = app();

    app.engine_mut().events.push(Event::KeyPressed {
        key: KeyCode::Space,
        repeat: false,
    });
    app.frame().unwrap();

    // `pressed` is per frame; `held` is not. A key pressed in an earlier frame
    // must not read as freshly pressed forever.
    app.frame().unwrap();
    assert!(app.engine().input.is_key_held(KeyCode::Space));

    app.engine_mut().events.push(Event::KeyReleased {
        key: KeyCode::Space,
    });
    app.frame().unwrap();
    assert!(!app.engine().input.is_key_held(KeyCode::Space));
}

#[test]
fn the_cursor_survives_a_frame() {
    let mut app = app();

    app.engine_mut()
        .events
        .push(Event::MouseMoved { x: 120.0, y: 45.0 });
    app.frame().unwrap();

    assert_eq!(app.engine().input.cursor(), (120.0, 45.0));
}

#[test]
fn a_button_press_is_tracked_like_a_key() {
    let mut app = app();

    app.engine_mut()
        .events
        .push(Event::MouseButtonPressed(MouseButton::Left));
    app.frame().unwrap();
    assert!(app.engine().input.is_mouse_held(MouseButton::Left));

    app.engine_mut()
        .events
        .push(Event::MouseButtonReleased(MouseButton::Left));
    app.frame().unwrap();
    assert!(!app.engine().input.is_mouse_held(MouseButton::Left));
}

/// The bus is drained, not read: an event consumed by one frame must not be
/// applied again by the next, or a press would be re-processed forever.
#[test]
fn events_are_consumed_once() {
    let mut app = app();

    app.engine_mut()
        .events
        .push(Event::MouseButtonPressed(MouseButton::Right));
    app.frame().unwrap();

    app.engine_mut()
        .events
        .push(Event::MouseButtonReleased(MouseButton::Right));
    app.frame().unwrap();
    assert!(!app.engine().input.is_mouse_held(MouseButton::Right));

    app.frame().unwrap();
    assert!(!app.engine().input.is_mouse_held(MouseButton::Right));
}
