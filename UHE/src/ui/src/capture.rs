//! Bridging egui capture state to `eyes`' keyboard claim.

use eyes::InputState;

/// RAII guard releasing the keyboard claim taken by [`capture_keyboard`].
pub struct KeyboardCaptureGuard<'a> {
    input: &'a mut InputState,
}

impl<'a> KeyboardCaptureGuard<'a> {
    /// Create a guard, claiming the keyboard immediately.
    pub fn capture(input: &'a mut InputState) -> Self {
        input.capture_keyboard();
        Self { input }
    }
}

impl Drop for KeyboardCaptureGuard<'_> {
    fn drop(&mut self) {
        self.input.release_keyboard();
    }
}

/// Claim the keyboard while `guard` lives.
pub fn capture_keyboard(input: &mut InputState) -> KeyboardCaptureGuard<'_> {
    KeyboardCaptureGuard::capture(input)
}

/// Expose capture scope helper.
pub struct CaptureScope {
    keyboard: bool,
}

impl CaptureScope {
    /// Keyboard-only capture.
    pub fn keyboard() -> Self {
        Self { keyboard: true }
    }

    /// Apply the capture to `input`.
    pub fn apply(&self, input: &mut InputState) {
        if self.keyboard {
            input.capture_keyboard();
        }
    }
}

/// Whether bindings may fire.
pub fn bindings_allowed(input: &InputState) -> bool {
    input.binding_allowed()
}

/// Whether a captured keyboard suppresses shortcut bindings.
pub fn keyboard_capture_suppresses_binding(input: &InputState) -> bool {
    !input.binding_allowed()
}
