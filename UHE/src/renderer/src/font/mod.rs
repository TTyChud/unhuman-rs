//! Fonts.
//!
//! Three pieces, split by what can fail and what can be tested without a device:
//! [`atlas`] is the file format, [`generate`] rasterizes a face at startup, and
//! [`layout`] is pure arithmetic over a loaded atlas.

pub mod atlas;
pub mod generate;
pub mod layout;

pub use atlas::{Atlas, AtlasError, Glyph, Metrics};
pub use generate::{GenerateError, GeneratorOptions};
pub use layout::{Align, Layout, LayoutOptions, Placement, TextQuad};

/// `Font2D::GetDefault()` (`UHE/src/UHE/Renderer/Font.cpp:220-227`) is the face
/// the engine ships, resolved through `vault` rather than the filesystem, so it
/// travels with the asset root like every other asset.
pub const DEFAULT_FONT_PATH: &str = "assets/fonts/Inter.ttf";
