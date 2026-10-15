//! Animation, scripts and text.

use serde::{Deserialize, Serialize};

/// A 2D sprite animation: which frames, how fast, and what to do at the end.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SpriteAnimation {
    /// Frame indices, in play order.
    pub frames: Vec<u32>,
    /// Frames per second.
    /// Frames advanced per second.
    pub fps: f32,
    /// Whether to return to the first frame after the last.
    /// Whether playback wraps.
    pub looping: bool,
}

/// Text drawn with the engine's font atlas.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Text {
    /// The string itself.
    pub body: String,
    /// Cap height in world units.
    pub size: f32,
    /// Linear RGBA.
    pub color: [f32; 4],
}

/// A value a script reads at init.
///
/// Its own three-variant enum rather than a general JSON value, because that is
/// all a property table needs and a general value would pull a JSON crate into
/// `vault` for a feature most scripts never use.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Property {
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// A double.
    Float(f64),
    /// A string.
    Text(String),
}

/// A script attached to an entity.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct NativeScript {
    /// Script name, resolved through the VFS. Not a path, so a script can move
    /// between folders without rewriting every scene that uses it.
    /// Script name, resolved through the VFS.
    pub name: String,
    /// Named values the script reads at init.
    /// Named values the script reads at init.
    pub properties: Vec<(String, Property)>,
}

/// Animation playback: which clip, at what speed, blended with what.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Animator {
    /// Clip currently playing.
    pub clip: String,
    /// Playback rate multiplier.
    pub speed: f32,
    /// Whether playback wraps at the end of the clip.
    pub looping: bool,
    /// Clip currently fading out, if any.
    /// Clip currently fading out, if any.
    pub fading_from: Option<String>,
    /// 0 = fully on `fading_from`, 1 = fully on `clip`.
    /// 0 = fully on `fading_from`, 1 = fully on `clip`.
    pub blend: f32,
}
