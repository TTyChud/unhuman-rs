//! Events delivered by a window.

use crate::keys::{Key, Modifiers, MouseButton};

/// Something that happened.
///
/// ## Two rules about what belongs here
///
/// **Platform-independent.** No `winit` type appears, and [`Key`] rather than
/// winit's, because a consumer should not have to care which windowing library
/// is underneath.
///
/// **Facts, not meanings.** An event says a key went down. It does not say that
/// means "undo" or "play" -- that is a binding, and a binding belongs to
/// whatever owns the action. The C++ mixed the two, which is how its editor
/// ended up with a `Q` that collided with the translate gizmo.
#[derive(Clone, PartialEq, Debug)]
#[non_exhaustive]
pub enum Event {
    /// The user asked to close the window, or the platform is sending it away.
    ///
    /// A window that cannot be closed is a test that cannot be torn down, and
    /// an editor with no exit is a program with no exit.
    CloseRequested,

    /// The drawable area changed size, in physical pixels.
    ///
    /// Pixels rather than logical units, because this is what a swapchain
    /// rebuilds on and a swapchain wants pixels. Never zero: a window really
    /// does reach zero during a minimise, and a swapchain cannot be zero-sized.
    Resized {
        /// Physical pixels across. At least 1.
        width: u32,
        /// Physical pixels down. At least 1.
        height: u32,
    },

    /// The window moved between displays, or its display's scale changed.
    ///
    /// This is a separate event from [`Event::Resized`] because one does not
    /// imply the other: dragging a window to a different monitor changes the
    /// scale and often not the size, and a swapchain that only watches `Resized`
    /// renders at the wrong resolution afterwards.
    ScaleFactorChanged {
        /// Physical pixels per logical pixel.
        scale_factor: f32,
    },

    /// Keyboard focus arrived or left.
    ///
    /// Input that ignores this is the reason a background window eats every
    /// keypress.
    Focused(bool),

    /// A key went down.
    ///
    /// `key` and `text` are different questions. `key` is which key it was, by
    /// position. `text` is what character that produced, which depends on the
    /// user's layout. A shortcut wants the first, a text field wants the second,
    /// and ctrl-shift-6 is a control character on one layout and a caret on
    /// another -- which is why this is not just one field.
    KeyPressed {
        /// Which key, by position.
        key: Key,
        /// The character produced, if any. `None` for a key that produces none,
        /// such as a modifier.
        text: Option<char>,
        /// Whether the OS considers this an auto-repeat.
        ///
        /// A repeat is a key *held*, not a key pressed again. Treating it as a
        /// new press is why holding a key re-triggers an action dozens of times
        /// a second.
        repeat: bool,
        /// Modifiers held at the time.
        modifiers: Modifiers,
    },

    /// A key came up.
    KeyReleased {
        /// Which key, by position.
        key: Key,
        /// Modifiers held at the time.
        modifiers: Modifiers,
    },

    /// Cursor position in physical pixels, origin top-left.
    MouseMoved {
        /// Pixels from the left edge.
        x: f64,
        /// Pixels from the top edge.
        y: f64,
    },

    /// A mouse button went down.
    MouseButtonPressed {
        /// Which button.
        button: MouseButton,
        /// Modifiers held at the time.
        modifiers: Modifiers,
    },

    /// A mouse button came up.
    MouseButtonReleased {
        /// Which button.
        button: MouseButton,
        /// Modifiers held at the time.
        modifiers: Modifiers,
    },

    /// Scroll wheel movement.
    ///
    /// Positive `delta_y` is away from the user, matching every platform's
    /// natural scroll direction rather than the sign of whatever pixel offset
    /// the driver produced.
    MouseScrolled {
        /// Horizontal scroll.
        delta_x: f32,
        /// Vertical scroll, positive away from the user.
        delta_y: f32,
        /// Modifiers held at the time.
        modifiers: Modifiers,
    },

    /// The modifier set changed with no other event to attach it to.
    ModifiersChanged(Modifiers),
}

impl Event {
    /// Whether the OS is asking the application to end.
    pub fn is_close(&self) -> bool {
        matches!(self, Event::CloseRequested)
    }

    /// Whether this event invalidates a cached hover-pick.
    ///
    /// Deliberately narrow. Only these change *what is under the cursor*, so
    /// only these may force a re-pick. Everything else can leave the pick in
    /// place -- which is the point, because picking on every frame means a GPU
    /// readback on every frame. The C++ does exactly that, once per frame, for
    /// every frame, to answer "what is under the mouse".
    pub fn invalidates_pointer_target(&self) -> bool {
        matches!(
            self,
            Event::MouseMoved { .. }
                | Event::MouseButtonPressed { .. }
                | Event::MouseButtonReleased { .. }
                | Event::Resized { .. }
                | Event::ScaleFactorChanged { .. }
        )
    }
}
