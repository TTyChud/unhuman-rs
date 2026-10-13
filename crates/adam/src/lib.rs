//! Base engine. Owns the composition root and nothing else.
//!
//! The one structural rule this crate enforces: **plugins hold behavior,
//! `Engine` holds data**. A plugin owns its own private state and receives
//! `&mut Engine` per hook, so `adam` never depends on a plugin crate and a
//! plugin never reaches into another plugin's state.

pub mod app;
pub mod engine;
pub mod error;
pub mod plugin;
pub mod time;

pub use app::App;
pub use engine::Engine;
pub use error::{Error, Result};
pub use plugin::{FrameCtx, Plugin};
pub use time::Time;

pub use chassis;
pub use grind;