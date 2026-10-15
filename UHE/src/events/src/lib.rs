//! Events and messages. Two separate mechanisms.

pub mod dispatcher;
pub mod event;
pub mod message;
pub mod observer;
pub mod queue;

pub use dispatcher::*;
pub use event::*;
pub use message::*;
pub use observer::*;
pub use queue::*;

// Common aliases.
// Type aliases for frequently used concepts.
pub use std::any::TypeId;
