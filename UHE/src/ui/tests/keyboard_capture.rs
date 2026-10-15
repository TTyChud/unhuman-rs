use mirror::capture::keyboard_capture_suppresses_binding;
use mirror::eyes::{EventPump, HeadlessWindow, InputState, Key, Modifiers, Window};

#[test]
fn capture_suppresses_binding() {
    let mut window = HeadlessWindow::new();
    let mut input = InputState::default();
    window.set_focused(true);
    window.pump(&mut input);
    assert!(!keyboard_capture_suppresses_binding(&input));

    input.capture_keyboard();
    window.push_event(mirror::eyes::Event::KeyPressed {
        key: Key::KeyQ,
        text: Some('q'),
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    window.pump(&mut input);
    assert!(keyboard_capture_suppresses_binding(&input));
    assert_eq!(input.text(), "");
}
