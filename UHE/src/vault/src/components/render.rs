//! Components that describe something drawn.

use serde::{Deserialize, Serialize};

/// A sprite: a 2D texture plus how to draw it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Sprite {
    /// Asset path, resolved through the VFS. Not an index -- an index into what?
    pub texture: String,
    /// Drawn width in world units.
    pub width: f32,
    /// Drawn height in world units.
    pub height: f32,
    /// Draw order within the same layer. Higher is nearer.
    pub sort_order: i32,
    /// Multiplied into the texture's colour.
    pub tint: [f32; 4],
}

/// An orthographic or perspective 2D camera.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Camera2d {
    /// Whether `size` is an orthographic extent or a vertical FOV.
    pub orthographic: bool,
    /// Vertical extent for orthographic, vertical FOV in radians otherwise.
    /// Cap height in world units.
    pub size: f32,
    /// Near clip distance.
    pub near: f32,
    /// Far clip distance.
    pub far: f32,
    /// Clear colour when this camera draws.
    /// Clear colour, as RGBA.
    pub clear: [f32; 4],
}

/// A 3D model instance.
///
/// `path` rather than a reference to a `Mesh`, because the document is data and
/// the mesh arrives when `vault` resolves it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Model3d {
    /// Asset path, resolved through the VFS.
    pub path: String,
    /// Index into the loaded model's materials, or `None` for every material.
    /// Index into the loaded model's materials, or `None` for every material.
    pub material_override: Option<u32>,
    /// Model node to instantiate. `None` means the model's default scene.
    /// Model node to instantiate, or `None` for the default scene.
    pub node: Option<String>,
    /// Whether the file's own animation clips should play.
    /// Whether the model's own clips start playing.
    pub auto_play: bool,
}
