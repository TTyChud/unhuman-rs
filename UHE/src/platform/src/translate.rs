//! `winit` → `pings`. The only place the two vocabularies meet.
//!
//! Every key `pings::KeyCode` names is mapped by hand, one arm each, so a
//! missing arm is visible rather than caught by a wildcard. What is left over
//! keeps its raw code instead of being dropped: `pings::KeyCode::Unknown`.

use pings::{Event, KeyCode, MouseButton};
use winit::event::{
    ElementState, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::keyboard::{KeyCode as WinitKeyCode, NativeKeyCode, PhysicalKey};

/// Appends the events one `winit` window event carries. A window event can be
/// more than one `pings::Event`: a character key is a press *and* text.
pub(crate) fn events(event: &WindowEvent, out: &mut Vec<Event>) {
    match event {
        WindowEvent::Resized(size) => out.push(Event::WindowResized {
            width: size.width,
            height: size.height,
        }),
        WindowEvent::CloseRequested => out.push(Event::WindowCloseRequested),
        WindowEvent::Focused(focused) => out.push(Event::WindowFocused(*focused)),
        WindowEvent::KeyboardInput { event, .. } => keyboard(event, out),
        WindowEvent::CursorMoved { position, .. } => out.push(Event::MouseMoved {
            x: position.x as f32,
            y: position.y as f32,
        }),
        WindowEvent::MouseWheel { delta, .. } => {
            let (dx, dy) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                MouseScrollDelta::PixelDelta(position) => (position.x as f32, position.y as f32),
            };
            out.push(Event::MouseScrolled { dx, dy });
        }
        WindowEvent::MouseInput { state, button, .. } => {
            let button = mouse_button(*button);
            out.push(match state {
                ElementState::Pressed => Event::MouseButtonPressed(button),
                ElementState::Released => Event::MouseButtonReleased(button),
            });
        }
        _ => {}
    }
}

fn keyboard(key: &KeyEvent, out: &mut Vec<Event>) {
    let code = key_code(key.physical_key);
    match key.state {
        ElementState::Pressed => {
            out.push(Event::KeyPressed {
                key: code,
                repeat: key.repeat,
            });
            if let Some(text) = &key.text {
                out.push(Event::TextInput(text.to_string()));
            }
        }
        ElementState::Released => out.push(Event::KeyReleased { key: code }),
    }
}

/// The physical key, never the layout's idea of it: WASD must stay where the
/// hand is, whatever the keycaps say.
fn key_code(physical: PhysicalKey) -> KeyCode {
    match physical {
        PhysicalKey::Code(code) => key(code),
        PhysicalKey::Unidentified(native) => KeyCode::Unknown(raw_code(native)),
    }
}

fn raw_code(native: NativeKeyCode) -> u32 {
    match native {
        NativeKeyCode::Android(code) | NativeKeyCode::Xkb(code) => code,
        NativeKeyCode::MacOS(code) | NativeKeyCode::Windows(code) => u32::from(code),
        NativeKeyCode::Unidentified => 0,
    }
}

fn key(code: WinitKeyCode) -> KeyCode {
    use WinitKeyCode as W;

    match code {
        W::Backquote => KeyCode::GraveAccent,
        W::Backslash => KeyCode::Backslash,
        W::BracketLeft => KeyCode::LeftBracket,
        W::BracketRight => KeyCode::RightBracket,
        W::Comma => KeyCode::Comma,
        W::Digit0 => KeyCode::Num0,
        W::Digit1 => KeyCode::Num1,
        W::Digit2 => KeyCode::Num2,
        W::Digit3 => KeyCode::Num3,
        W::Digit4 => KeyCode::Num4,
        W::Digit5 => KeyCode::Num5,
        W::Digit6 => KeyCode::Num6,
        W::Digit7 => KeyCode::Num7,
        W::Digit8 => KeyCode::Num8,
        W::Digit9 => KeyCode::Num9,
        W::Equal => KeyCode::Equal,
        W::KeyA => KeyCode::A,
        W::KeyB => KeyCode::B,
        W::KeyC => KeyCode::C,
        W::KeyD => KeyCode::D,
        W::KeyE => KeyCode::E,
        W::KeyF => KeyCode::F,
        W::KeyG => KeyCode::G,
        W::KeyH => KeyCode::H,
        W::KeyI => KeyCode::I,
        W::KeyJ => KeyCode::J,
        W::KeyK => KeyCode::K,
        W::KeyL => KeyCode::L,
        W::KeyM => KeyCode::M,
        W::KeyN => KeyCode::N,
        W::KeyO => KeyCode::O,
        W::KeyP => KeyCode::P,
        W::KeyQ => KeyCode::Q,
        W::KeyR => KeyCode::R,
        W::KeyS => KeyCode::S,
        W::KeyT => KeyCode::T,
        W::KeyU => KeyCode::U,
        W::KeyV => KeyCode::V,
        W::KeyW => KeyCode::W,
        W::KeyX => KeyCode::X,
        W::KeyY => KeyCode::Y,
        W::KeyZ => KeyCode::Z,
        W::Minus => KeyCode::Minus,
        W::Period => KeyCode::Period,
        W::Quote => KeyCode::Apostrophe,
        W::Semicolon => KeyCode::Semicolon,
        W::Slash => KeyCode::Slash,
        W::AltLeft => KeyCode::LeftAlt,
        W::AltRight => KeyCode::RightAlt,
        W::Backspace => KeyCode::Backspace,
        W::CapsLock => KeyCode::CapsLock,
        W::ContextMenu => KeyCode::Menu,
        W::ControlLeft => KeyCode::LeftControl,
        W::ControlRight => KeyCode::RightControl,
        W::Enter => KeyCode::Enter,
        W::SuperLeft => KeyCode::LeftSuper,
        W::SuperRight => KeyCode::RightSuper,
        W::ShiftLeft => KeyCode::LeftShift,
        W::ShiftRight => KeyCode::RightShift,
        W::Space => KeyCode::Space,
        W::Tab => KeyCode::Tab,
        W::Delete => KeyCode::Delete,
        W::End => KeyCode::End,
        W::Home => KeyCode::Home,
        W::Insert => KeyCode::Insert,
        W::PageDown => KeyCode::PageDown,
        W::PageUp => KeyCode::PageUp,
        W::ArrowDown => KeyCode::Down,
        W::ArrowLeft => KeyCode::Left,
        W::ArrowRight => KeyCode::Right,
        W::ArrowUp => KeyCode::Up,
        W::NumLock => KeyCode::NumLock,
        W::Numpad0 => KeyCode::Keypad0,
        W::Numpad1 => KeyCode::Keypad1,
        W::Numpad2 => KeyCode::Keypad2,
        W::Numpad3 => KeyCode::Keypad3,
        W::Numpad4 => KeyCode::Keypad4,
        W::Numpad5 => KeyCode::Keypad5,
        W::Numpad6 => KeyCode::Keypad6,
        W::Numpad7 => KeyCode::Keypad7,
        W::Numpad8 => KeyCode::Keypad8,
        W::Numpad9 => KeyCode::Keypad9,
        W::NumpadAdd => KeyCode::KeypadAdd,
        W::NumpadDecimal => KeyCode::KeypadDecimal,
        W::NumpadDivide => KeyCode::KeypadDivide,
        W::NumpadEnter => KeyCode::KeypadEnter,
        W::NumpadEqual => KeyCode::KeypadEqual,
        W::NumpadMultiply => KeyCode::KeypadMultiply,
        W::NumpadSubtract => KeyCode::KeypadSubtract,
        W::Escape => KeyCode::Escape,
        W::PrintScreen => KeyCode::PrintScreen,
        W::ScrollLock => KeyCode::ScrollLock,
        W::Pause => KeyCode::Pause,
        W::F1 => KeyCode::F1,
        W::F2 => KeyCode::F2,
        W::F3 => KeyCode::F3,
        W::F4 => KeyCode::F4,
        W::F5 => KeyCode::F5,
        W::F6 => KeyCode::F6,
        W::F7 => KeyCode::F7,
        W::F8 => KeyCode::F8,
        W::F9 => KeyCode::F9,
        W::F10 => KeyCode::F10,
        W::F11 => KeyCode::F11,
        W::F12 => KeyCode::F12,
        W::F13 => KeyCode::F13,
        W::F14 => KeyCode::F14,
        W::F15 => KeyCode::F15,
        W::F16 => KeyCode::F16,
        W::F17 => KeyCode::F17,
        W::F18 => KeyCode::F18,
        W::F19 => KeyCode::F19,
        W::F20 => KeyCode::F20,
        W::F21 => KeyCode::F21,
        W::F22 => KeyCode::F22,
        W::F23 => KeyCode::F23,
        W::F24 => KeyCode::F24,
        W::F25 => KeyCode::F25,
        // Keys `pings` has no name for keep winit's own code: no platform scan
        // code reaches this point, and a key that is bound to nothing is still
        // better than a key that is bound to everything.
        other => KeyCode::Unknown(other as u32),
    }
}

/// GLFW's button numbering, which the C++ handed straight to the event: 3 is
/// back, 4 is forward.
fn mouse_button(button: WinitMouseButton) -> MouseButton {
    match button {
        WinitMouseButton::Left => MouseButton::Left,
        WinitMouseButton::Right => MouseButton::Right,
        WinitMouseButton::Middle => MouseButton::Middle,
        WinitMouseButton::Back => MouseButton::Other(3),
        WinitMouseButton::Forward => MouseButton::Other(4),
        WinitMouseButton::Other(index) => MouseButton::Other(index as u8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_map_is_by_position_not_by_layout() {
        let w = PhysicalKey::Code(WinitKeyCode::KeyW);

        assert_eq!(key_code(w), KeyCode::W);
        assert_eq!(key_code(PhysicalKey::Code(WinitKeyCode::Digit7)), KeyCode::Num7);
        assert_eq!(
            key_code(PhysicalKey::Code(WinitKeyCode::ShiftLeft)),
            KeyCode::LeftShift
        );
        assert_eq!(
            key_code(PhysicalKey::Code(WinitKeyCode::NumpadEnter)),
            KeyCode::KeypadEnter
        );
    }

    #[test]
    fn an_unmapped_key_keeps_its_platform_code() {
        assert_eq!(
            key_code(PhysicalKey::Unidentified(NativeKeyCode::Windows(0x0056))),
            KeyCode::Unknown(0x0056)
        );
        assert_eq!(
            key_code(PhysicalKey::Unidentified(NativeKeyCode::Xkb(94))),
            KeyCode::Unknown(94)
        );
    }

    #[test]
    fn a_key_pings_cannot_name_still_arrives() {
        // winit knows this key; `pings::KeyCode` has no variant for it.
        let code = key_code(PhysicalKey::Code(WinitKeyCode::IntlYen));
        assert!(matches!(code, KeyCode::Unknown(_)));
    }

    #[test]
    fn mouse_buttons_keep_glfws_numbering() {
        assert_eq!(mouse_button(WinitMouseButton::Left), MouseButton::Left);
        assert_eq!(mouse_button(WinitMouseButton::Back), MouseButton::Other(3));
        assert_eq!(
            mouse_button(WinitMouseButton::Other(9)),
            MouseButton::Other(9)
        );
    }
}
