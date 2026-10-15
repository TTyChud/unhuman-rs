//! Polled input state.
//!
//! ## Why input is a value the pump updates rather than a query
//!
//! The C++ polls the OS: `glfwGetKey`, `glfwGetMouseButton`, called from
//! wherever the value is needed. That has two costs. The second is obvious --
//! a poll per consumer per frame is a call into the windowing system each time.
//! The first is the one that actually bites: it *cannot be tested*, because
//! there is no way to make a key be down.
//!
//! So events arrive, an [`InputState`] is updated by delivering them, and
//! consumers read memory. The same code path runs in a test with no window.

use std::collections::HashSet;

use crate::event::Event;
use crate::keys::{Key, Modifiers, MouseButton};

/// Where the pointer is and what it is doing.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct MouseState {
    /// Pixels from the left edge, and from the top.
    pub position: (f64, f64),
    /// Movement since the previous frame, so a camera can integrate it without
    /// subtracting positions itself and getting the first frame wrong.
    pub delta: (f64, f64),
    /// Scroll since the last pump. Positive `y` is away from the user.
    pub scroll: (f32, f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct MouseButtons {
    down: u8,
    pressed: u8,
    released: u8,
}

/// Everything a frame's consumers can ask about input.
///
/// Two kinds of question live here and they are not interchangeable:
///
/// - **held**: `key_down`, `mouse_down`. True for as long as the key is down.
/// - **this frame**: `key_pressed`, `mouse_pressed`. True for exactly one frame.
///
/// A binding wants `pressed`. A camera wants `down` or a delta. Using `down` for
/// a binding fires it every frame the key is held, which is the single most
/// common input bug; `pressed` is cleared in [`InputState::begin_frame`], which
/// is called by the pump *before* delivery, so "this frame" means "since the
/// last pump" and not "since some arbitrary point".
#[derive(Clone, PartialEq, Debug, Default)]
pub struct InputState {
    keys_down: HashSet<Key>,
    keys_pressed: HashSet<Key>,
    keys_released: HashSet<Key>,
    buttons: [MouseButtons; 8],
    other_buttons_down: HashSet<MouseButton>,
    other_buttons_pressed: HashSet<MouseButton>,
    other_buttons_released: HashSet<MouseButton>,
    mouse: MouseState,
    modifiers: Modifiers,
    text: String,
    focused: bool,
    /// Nesting depth of consumers that want the keyboard for themselves.
    keyboard_captured: u32,
    delivered: usize,
}

impl InputState {
    /// Clears everything that means "since the last frame".
    ///
    /// Called by the pump, before it delivers. Not called by consumers: a
    /// consumer that cleared it would make the answer depend on how many
    /// consumers there are.
    pub fn begin_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.mouse.delta = (0.0, 0.0);
        self.mouse.scroll = (0.0, 0.0);
        self.text.clear();
        self.delivered = 0;
        for buttons in &mut self.buttons {
            buttons.pressed = 0;
            buttons.released = 0;
        }
        self.other_buttons_pressed.clear();
        self.other_buttons_released.clear();
    }

    /// Deliver one event. Called by the pump, not by consumers.
    pub fn apply(&mut self, event: &Event) {
        self.delivered += 1;
        match event {
            Event::KeyPressed {
                key,
                text,
                repeat,
                modifiers,
            } => {
                // A repeat is a held key, not a new press. Treating one as a
                // press is why holding a key can re-trigger an action dozens
                // of times a second.
                if !repeat && !self.keys_down.contains(key) {
                    self.keys_pressed.insert(*key);
                }
                self.keys_down.insert(*key);
                self.modifiers = *modifiers;

                // The character this key produced, which is a different
                // question from which key went down. Suppressed while a
                // consumer owns the keyboard, because a shortcut's accelerator
                // produces text on some layouts and typing it into a field is
                // not what the user meant.
                if let Some(text) = text {
                    if self.keyboard_captured == 0 {
                        self.text.push(*text);
                    }
                }
            }
            Event::KeyReleased { key, modifiers } => {
                self.keys_down.remove(key);
                self.keys_released.insert(*key);
                self.modifiers = *modifiers;
            }
            Event::MouseMoved { x, y } => {
                self.mouse.delta.0 += *x - self.mouse.position.0;
                self.mouse.delta.1 += *y - self.mouse.position.1;
                self.mouse.position = (*x, *y);
            }
            Event::MouseButtonPressed { button, modifiers } => {
                self.set_button(*button, true);
                self.modifiers = *modifiers;
            }
            Event::MouseButtonReleased { button, modifiers } => {
                self.set_button(*button, false);
                self.modifiers = *modifiers;
            }
            Event::MouseScrolled {
                delta_x,
                delta_y,
                modifiers,
            } => {
                self.mouse.scroll.0 += delta_x;
                self.mouse.scroll.1 += delta_y;
                self.modifiers = *modifiers;
            }
            Event::ModifiersChanged(modifiers) => self.modifiers = *modifiers,
            Event::Focused(focused) => self.focused = *focused,
            Event::Resized { .. } | Event::ScaleFactorChanged { .. } => {}
            Event::CloseRequested => {}
        }
    }

    fn set_button(&mut self, button: MouseButton, down: bool) {
        match button {
            MouseButton::Left => self.set_named(0, down),
            MouseButton::Right => self.set_named(1, down),
            MouseButton::Middle => self.set_named(2, down),
            MouseButton::Other(_) => {
                if down {
                    if !self.other_buttons_down.contains(&button) {
                        self.other_buttons_pressed.insert(button);
                    }
                    self.other_buttons_down.insert(button);
                } else {
                    self.other_buttons_down.remove(&button);
                    self.other_buttons_released.insert(button);
                }
            }
        }
    }

    fn set_named(&mut self, index: usize, down: bool) {
        let bit = 1 << index;
        let buttons = &mut self.buttons[index];
        if down {
            if buttons.down & bit == 0 {
                buttons.pressed |= bit;
            }
            buttons.down |= bit;
        } else {
            buttons.down &= !bit;
            buttons.released |= bit;
        }
    }

    fn named(&self, index: usize) -> MouseButtons {
        self.buttons[index]
    }

    /// A key is held right now.
    pub fn key_down(&self, key: Key) -> bool {
        self.keys_down.contains(&key)
    }

    /// A key went down since the last pump.
    pub fn key_pressed(&self, key: Key) -> bool {
        self.keys_pressed.contains(&key)
    }

    /// A key came up since the last pump.
    pub fn key_released(&self, key: Key) -> bool {
        self.keys_released.contains(&key)
    }

    /// Every key currently held. For a debug overlay or a rebinding screen;
    /// a binding should ask about its own key.
    pub fn keys_down(&self) -> impl Iterator<Item = Key> + '_ {
        self.keys_down.iter().copied()
    }

    /// A mouse button is held right now.
    pub fn mouse_down(&self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.named(0).down & 1 != 0,
            MouseButton::Right => self.named(1).down & 1 != 0,
            MouseButton::Middle => self.named(2).down & 1 != 0,
            MouseButton::Other(_) => self.other_buttons_down.contains(&button),
        }
    }

    /// A mouse button went down since the last pump.
    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.named(0).pressed & 1 != 0,
            MouseButton::Right => self.named(1).pressed & 1 != 0,
            MouseButton::Middle => self.named(2).pressed & 1 != 0,
            MouseButton::Other(_) => self.other_buttons_pressed.contains(&button),
        }
    }

    /// A mouse button came up since the last pump.
    pub fn mouse_released(&self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.named(0).released & 1 != 0,
            MouseButton::Right => self.named(1).released & 1 != 0,
            MouseButton::Middle => self.named(2).released & 1 != 0,
            MouseButton::Other(_) => self.other_buttons_released.contains(&button),
        }
    }

    /// The whole mouse state at once.
    pub fn mouse(&self) -> MouseState {
        self.mouse
    }

    /// The modifiers held as of the most recent event.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Whether the window has keyboard focus.
    ///
    /// A frame loop that ignores this processes input aimed at some other
    /// application, which on Linux and macOS means a background window eating
    /// every keypress.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Text typed since the last pump, and cleared by it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Claim the keyboard, so bindings do not fire while typing.
    ///
    /// Depth-counted, because a text field inside a modal that also wants the
    /// keyboard is two claims and needs two releases. The C++ never checked
    /// whether a text field wanted the keyboard, so `Q`, `W`, `E` and `R` fired
    /// while the user was typing and deleted the characters instead.
    pub fn capture_keyboard(&mut self) {
        self.keyboard_captured += 1;
    }

    /// Release a claim taken by [`InputState::capture_keyboard`].
    pub fn release_keyboard(&mut self) {
        self.keyboard_captured = self.keyboard_captured.saturating_sub(1);
    }

    /// True while any consumer holds the keyboard.
    pub fn keyboard_captured(&self) -> bool {
        self.keyboard_captured > 0
    }

    /// A binding should only fire when nothing owns the keyboard.
    pub fn binding_allowed(&self) -> bool {
        self.focused && self.keyboard_captured == 0
    }

    /// How many events were delivered since the last pump.
    pub fn delivered(&self) -> usize {
        self.delivered
    }
}
