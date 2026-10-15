//! UI. egui + egui-wgpu, backend-neutral.

pub mod capture;
pub mod command_registry;
pub mod gizmo;
pub mod layout;
pub mod paint;
pub mod theme;

pub use eyes;
pub mod undo;

pub use capture::{bindings_allowed, CaptureScope, KeyboardCaptureGuard};
pub use command_registry::{Command, CommandRegistry, Shortcut};
pub use gizmo::{ray_plane_intersection, GizmoMode, Ray};
pub use layout::Layout;
pub use paint::{PaintCmd, PaintList, PaintSink, RecordingSink};
pub use theme::{Rgba, Scheme, Theme, ThemeDef, Token};
pub use undo::{UndoCommand, UndoHistory};
