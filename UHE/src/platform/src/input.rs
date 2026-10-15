//! Input state, fed from the event bus and read by the frame.
//!
//! The C++ `Input` polled GLFW directly (`WindowInput.cpp`), so the window's
//! state and the engine's state were two views of one thing and could disagree
//! — the ImGui layer read `glfwGetKey`, the game read the event callbacks. Here
//! there is one `Input`, `pings::Event`s are the only thing that writes to it,
//! and the window layer never keeps a copy.

use pings::{Event, KeyCode, MouseButton};

/// Held state persists across frames; pressed, released, scroll and cursor
/// delta are per frame.
pub struct Input {
    held_keys: Vec<KeyCode>,
    pressed_keys: Vec<KeyCode>,
    released_keys: Vec<KeyCode>,
    held_mouse: Vec<MouseButton>,
    pressed_mouse: Vec<MouseButton>,
    cursor: (f32, f32),
    cursor_start: (f32, f32),
    scroll: (f32, f32),
}

impl Input {
    pub fn new() -> Self {
        Self {
            held_keys: Vec::new(),
            pressed_keys: Vec::new(),
            released_keys: Vec::new(),
            held_mouse: Vec::new(),
            pressed_mouse: Vec::new(),
            cursor: (0.0, 0.0),
            cursor_start: (0.0, 0.0),
            scroll: (0.0, 0.0),
        }
    }

    pub fn begin_frame(&mut self) {
        self.clear_per_frame();
        self.cursor_start = self.cursor;
    }

    pub fn end_frame(&mut self) {
        self.clear_per_frame();
    }

    /// The only writer. Every event is applied once, so a key pressed and
    /// released inside one frame is both pressed and released in that frame.
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::KeyPressed { key, repeat } => {
                // A repeat is a held key, not a new press: a bound action must
                // not fire again because the key was held down.
                if !*repeat && !self.pressed_keys.contains(key) {
                    self.pressed_keys.push(*key);
                }
                if !self.held_keys.contains(key) {
                    self.held_keys.push(*key);
                }
            }
            Event::KeyReleased { key } => {
                self.held_keys.retain(|held| held != key);
                if !self.released_keys.contains(key) {
                    self.released_keys.push(*key);
                }
            }
            Event::MouseButtonPressed(button) => {
                if !self.pressed_mouse.contains(button) {
                    self.pressed_mouse.push(*button);
                }
                if !self.held_mouse.contains(button) {
                    self.held_mouse.push(*button);
                }
            }
            Event::MouseButtonReleased(button) => {
                self.held_mouse.retain(|held| held != button);
            }
            Event::MouseMoved { x, y } => {
                self.cursor = (*x, *y);
            }
            Event::MouseScrolled { dx, dy } => {
                self.scroll.0 += *dx;
                self.scroll.1 += *dy;
            }
            Event::WindowResized { .. }
            | Event::WindowCloseRequested
            | Event::WindowFocused(_)
            | Event::TextInput(_)
            | Event::FrameBegin
            | Event::FrameEnd => {}
        }
    }

    pub fn is_key_held(&self, key: KeyCode) -> bool {
        self.held_keys.contains(&key)
    }

    pub fn is_key_pressed(&self, key: KeyCode) -> bool {
        self.pressed_keys.contains(&key)
    }

    pub fn is_key_released(&self, key: KeyCode) -> bool {
        self.released_keys.contains(&key)
    }

    pub fn is_mouse_held(&self, button: MouseButton) -> bool {
        self.held_mouse.contains(&button)
    }

    pub fn is_mouse_pressed(&self, button: MouseButton) -> bool {
        self.pressed_mouse.contains(&button)
    }

    /// The position of the last move event, and the same position in window
    /// pixels, which is what a camera reads.
    pub fn cursor(&self) -> (f32, f32) {
        self.cursor
    }

    /// This frame's movement, derived from the cursor rather than accumulated
    /// per event, so a frame with ten move events and a frame with one agree.
    pub fn cursor_delta(&self) -> (f32, f32) {
        (
            self.cursor.0 - self.cursor_start.0,
            self.cursor.1 - self.cursor_start.1,
        )
    }

    pub fn scroll(&self) -> (f32, f32) {
        self.scroll
    }

    fn clear_per_frame(&mut self) {
        self.pressed_keys.clear();
        self.released_keys.clear();
        self.pressed_mouse.clear();
        self.scroll = (0.0, 0.0);
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}
