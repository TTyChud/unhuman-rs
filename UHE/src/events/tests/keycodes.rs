use std::collections::HashSet;

use pings::{KeyCode, MouseButton};

/// Every name `UHE/src/UHE/Core/keyCodes.h` declares, in its order, with the
/// variant it became. The two renames: `D0`..`D9` are `Num0`..`Num9`, and the
/// `KP*` keypad keys are `Keypad*`.
const KEY_CODES: &[(&str, KeyCode)] = &[
    ("Space", KeyCode::Space),
    ("Apostrophe", KeyCode::Apostrophe),
    ("Comma", KeyCode::Comma),
    ("Minus", KeyCode::Minus),
    ("Period", KeyCode::Period),
    ("Slash", KeyCode::Slash),
    ("D0", KeyCode::Num0),
    ("D1", KeyCode::Num1),
    ("D2", KeyCode::Num2),
    ("D3", KeyCode::Num3),
    ("D4", KeyCode::Num4),
    ("D5", KeyCode::Num5),
    ("D6", KeyCode::Num6),
    ("D7", KeyCode::Num7),
    ("D8", KeyCode::Num8),
    ("D9", KeyCode::Num9),
    ("Semicolon", KeyCode::Semicolon),
    ("Equal", KeyCode::Equal),
    ("A", KeyCode::A),
    ("B", KeyCode::B),
    ("C", KeyCode::C),
    ("D", KeyCode::D),
    ("E", KeyCode::E),
    ("F", KeyCode::F),
    ("G", KeyCode::G),
    ("H", KeyCode::H),
    ("I", KeyCode::I),
    ("J", KeyCode::J),
    ("K", KeyCode::K),
    ("L", KeyCode::L),
    ("M", KeyCode::M),
    ("N", KeyCode::N),
    ("O", KeyCode::O),
    ("P", KeyCode::P),
    ("Q", KeyCode::Q),
    ("R", KeyCode::R),
    ("S", KeyCode::S),
    ("T", KeyCode::T),
    ("U", KeyCode::U),
    ("V", KeyCode::V),
    ("W", KeyCode::W),
    ("X", KeyCode::X),
    ("Y", KeyCode::Y),
    ("Z", KeyCode::Z),
    ("LeftBracket", KeyCode::LeftBracket),
    ("Backslash", KeyCode::Backslash),
    ("RightBracket", KeyCode::RightBracket),
    ("GraveAccent", KeyCode::GraveAccent),
    ("Escape", KeyCode::Escape),
    ("Enter", KeyCode::Enter),
    ("Tab", KeyCode::Tab),
    ("Backspace", KeyCode::Backspace),
    ("Insert", KeyCode::Insert),
    ("Delete", KeyCode::Delete),
    ("Right", KeyCode::Right),
    ("Left", KeyCode::Left),
    ("Down", KeyCode::Down),
    ("Up", KeyCode::Up),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("CapsLock", KeyCode::CapsLock),
    ("ScrollLock", KeyCode::ScrollLock),
    ("NumLock", KeyCode::NumLock),
    ("PrintScreen", KeyCode::PrintScreen),
    ("Pause", KeyCode::Pause),
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("F12", KeyCode::F12),
    ("F13", KeyCode::F13),
    ("F14", KeyCode::F14),
    ("F15", KeyCode::F15),
    ("F16", KeyCode::F16),
    ("F17", KeyCode::F17),
    ("F18", KeyCode::F18),
    ("F19", KeyCode::F19),
    ("F20", KeyCode::F20),
    ("F21", KeyCode::F21),
    ("F22", KeyCode::F22),
    ("F23", KeyCode::F23),
    ("F24", KeyCode::F24),
    ("F25", KeyCode::F25),
    ("KP0", KeyCode::Keypad0),
    ("KP1", KeyCode::Keypad1),
    ("KP2", KeyCode::Keypad2),
    ("KP3", KeyCode::Keypad3),
    ("KP4", KeyCode::Keypad4),
    ("KP5", KeyCode::Keypad5),
    ("KP6", KeyCode::Keypad6),
    ("KP7", KeyCode::Keypad7),
    ("KP8", KeyCode::Keypad8),
    ("KP9", KeyCode::Keypad9),
    ("KPDecimal", KeyCode::KeypadDecimal),
    ("KPDivide", KeyCode::KeypadDivide),
    ("KPMultiply", KeyCode::KeypadMultiply),
    ("KPSubtract", KeyCode::KeypadSubtract),
    ("KPAdd", KeyCode::KeypadAdd),
    ("KPEnter", KeyCode::KeypadEnter),
    ("KPEqual", KeyCode::KeypadEqual),
    ("LeftShift", KeyCode::LeftShift),
    ("LeftControl", KeyCode::LeftControl),
    ("LeftAlt", KeyCode::LeftAlt),
    ("LeftSuper", KeyCode::LeftSuper),
    ("RightShift", KeyCode::RightShift),
    ("RightControl", KeyCode::RightControl),
    ("RightAlt", KeyCode::RightAlt),
    ("RightSuper", KeyCode::RightSuper),
    ("Menu", KeyCode::Menu),
];

#[test]
fn every_key_the_cpp_names_maps_to_one_variant() {
    let mut seen = HashSet::new();
    for (name, key) in KEY_CODES {
        assert!(seen.insert(*key), "{name} maps to a variant already listed");
    }
    assert_eq!(seen.len(), 118);
}

#[test]
fn unknown_round_trips_the_raw_code() {
    let raw = 0xDEAD_BEEF;
    match KeyCode::Unknown(raw) {
        KeyCode::Unknown(code) => assert_eq!(code, raw),
        other => panic!("Unknown did not round-trip: {other:?}"),
    }

    assert_ne!(KeyCode::Unknown(u32::from(b'W')), KeyCode::W);
    assert_eq!(KeyCode::Unknown(raw), KeyCode::Unknown(raw));
}

#[test]
fn mouse_buttons_keep_the_cpp_spelling_and_other_keeps_the_raw_number() {
    assert_ne!(MouseButton::Left, MouseButton::Right);
    assert_ne!(MouseButton::Left, MouseButton::Middle);
    assert_eq!(MouseButton::Other(6), MouseButton::Other(6));
    assert_ne!(MouseButton::Other(6), MouseButton::Other(7));
}
