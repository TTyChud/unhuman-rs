//! Event mapping: winit's events to ours.
//!
//! Its own file because it is a pure translation with no platform calls, so it
//! is testable and it is the only place a winit event *shape* change can break
//! something.

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};

use crate::event::Event;
use crate::keys::{Modifiers, MouseButton};
use crate::winit_keymap::map_key;

/// Scroll units converted from lines to something with a size.
///
/// A wheel notch is a line on Windows and a pixel delta on a trackpad. Handing
/// the raw `LineDelta` to a camera zooms by 1 unit on one platform and by 30 on
/// another, which is the same zoom feeling as different physical mice.
///
/// 3.0 is the conventional conversion: enough that a notch moves the world
/// visibly, small enough that a full scroll of a list is a dozen notches rather
/// than a blur.
const LINES_TO_UNITS: f32 = 3.0;

/// Translate one window event.
///
/// `None` for the events that are not ours. The arm is explicit on purpose: a
/// silently ignored event is how a platform feature goes missing without
/// anything ever saying so, and the list of what is ignored belongs here where
/// it can be read.
pub fn map_window_event(event: &WindowEvent) -> Option<Event> {
    let mapped = match event {
        WindowEvent::CloseRequested => Event::CloseRequested,

        WindowEvent::Resized(PhysicalSize { width, height }) => Event::Resized {
            // A zero-sized window is real: it happens on minimise, and on some
            // window managers mid-maximise. Clamped to 1, because a swapchain
            // cannot be zero-sized and letting it try is a validation error at
            // exactly the moment the user is dragging the window edge.
            width: (*width).max(1),
            height: (*height).max(1),
        },

        WindowEvent::ScaleFactorChanged { scale_factor, .. } => Event::ScaleFactorChanged {
            scale_factor: *scale_factor as f32,
        },

        WindowEvent::Focused(focused) => Event::Focused(*focused),

        WindowEvent::ModifiersChanged(modifiers) => {
            let state = modifiers.state();
            Event::ModifiersChanged(Modifiers {
                shift: state.shift_key(),
                control: state.control_key(),
                alt: state.alt_key(),
                super_key: state.super_key(),
            })
        }

        WindowEvent::KeyboardInput { event, .. } => {
            let modifiers = Modifiers::NONE;
            match event.state {
                ElementState::Pressed => Event::KeyPressed {
                    key: map_key(event.physical_key),
                    // `text` is the character this produced, which is a
                    // different question from which key went down. A text
                    // field wants this; a shortcut wants the key.
                    text: event.text.as_ref().and_then(|text| text.chars().next()),
                    repeat: event.repeat,
                    modifiers,
                },
                ElementState::Released => Event::KeyReleased {
                    key: map_key(event.physical_key),
                    modifiers,
                },
            }
        }

        WindowEvent::CursorMoved { position, .. } => Event::MouseMoved {
            x: position.x,
            y: position.y,
        },

        WindowEvent::MouseInput { state, button, .. } => {
            let button = map_mouse_button(*button);
            match state {
                ElementState::Pressed => Event::MouseButtonPressed {
                    button,
                    modifiers: Modifiers::NONE,
                },
                ElementState::Released => Event::MouseButtonReleased {
                    button,
                    modifiers: Modifiers::NONE,
                },
            }
        }

        WindowEvent::MouseWheel { delta, .. } => {
            let (delta_x, delta_y) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (x * LINES_TO_UNITS, y * LINES_TO_UNITS),
                MouseScrollDelta::PixelDelta(PhysicalPosition { x, y }) => (*x as f32, *y as f32),
            };
            Event::MouseScrolled {
                delta_x,
                delta_y,
                modifiers: Modifiers::NONE,
            }
        }

        _ => return None,
    };
    Some(mapped)
}

fn map_mouse_button(button: winit::event::MouseButton) -> MouseButton {
    match button {
        winit::event::MouseButton::Left => MouseButton::Left,
        winit::event::MouseButton::Right => MouseButton::Right,
        winit::event::MouseButton::Middle => MouseButton::Middle,
        // Back and forward are the standard X11 button numbers 8 and 9. They
        // get their own names rather than collapsing into "other", because
        // paging back through a scene is a real thing to want and an unnamed
        // button cannot be bound.
        winit::event::MouseButton::Back => MouseButton::Other(8),
        winit::event::MouseButton::Forward => MouseButton::Other(9),
        winit::event::MouseButton::Other(code) => MouseButton::Other(code),
    }
}
