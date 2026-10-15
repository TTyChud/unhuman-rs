//! Platform-independent keys and buttons.
//!
//! ## Why these are not `winit::keyboard::Key`
//!
//! Because `winit` is an implementation detail of this crate. If a key enum
//! re-exported winit's, every consumer -- `mirror`, `sandbox`, a test -- would
//! name a windowing library to ask whether shift is down, and the same seam
//! problem as `chassis` leaking `vk::` would appear one layer up.
//!
//! The mapping from winit lives in exactly one file, `winit_window.rs`.

/// A physical key, identified by position rather than by the character it
/// produces.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
// `missing_docs` is off here and only here. Thirty-six of these variants are
// letters and digits whose identifier is the documentation, and a comment
// restating `KeyB` as "the B key" adds length and no information. Every variant
// whose *name* does not explain its behaviour is documented anyway -- Numpad0
// and PrintScreen and Other all say why they exist. The lint stays armed
// crate-wide, which is where it earns its keep.
#[allow(missing_docs)]
pub enum Key {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,

    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,

    Space,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,

    Left,
    Right,
    Up,
    Down,

    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,

    /// Keypad, with the number spelled out.
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,

    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    Shift,
    Control,
    Alt,
    Super,

    /// Anything this build does not name. Kept so an unknown key still arrives
    /// rather than being dropped, which is how a binding ends up missing with
    /// no way to find out why.
    Other,
}

/// A mouse button.
///
/// `Other` carries the platform's own button number, so a button this build
/// has no name for still arrives as itself. `None` would make it unbindable.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum MouseButton {
    /// The primary button.
    Left,
    /// The secondary button.
    Right,
    /// The wheel button.
    Middle,
    /// A button with no name here, by the platform's number. 8 and 9 are back
    /// and forward on X11 and Wayland.
    Other(u16),
}

/// Modifier keys held during an event.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Modifiers {
    /// Either shift key.
    pub shift: bool,
    /// Either control key. Command on macOS.
    pub control: bool,
    /// Alt on Linux and Windows, Option on macOS. The C++ called this ALT; the
    /// engine's action bindings should too, because "the key next to space".
    pub alt: bool,
    /// The OS or window manager's own modifier, and the reason `Super` exists.
    pub super_key: bool,
}

impl Modifiers {
    /// No modifiers held.
    pub const NONE: Self = Self {
        shift: false,
        control: false,
        alt: false,
        super_key: false,
    };

    /// Shift only.
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };

    /// Control only.
    pub const CONTROL: Self = Self {
        control: true,
        ..Self::NONE
    };

    /// Alt only.
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };

    /// Whether any modifier at all is held.
    pub const fn any(&self) -> bool {
        self.shift || self.control || self.alt || self.super_key
    }
}
