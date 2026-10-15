//! Events: the bus, the key and mouse codes, the event enum.
//!
//! Ported from `UHE/src/UHE/Events/`, `UHE/src/UHE/Core/keyCodes.h`,
//! `mouseButtonCodes.h` and the dispatch in `Core/Application.cpp`.

mod bus;
mod event;
mod key;

pub use bus::{EventBus, Subscription};
pub use event::Event;
pub use key::{KeyCode, MouseButton};
