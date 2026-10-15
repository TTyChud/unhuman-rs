//! `vault` — asset identity, and the file formats.
//!
//! ## Two formats, two jobs
//!
//! | | format | who reads it | why |
//! |---|---|---|---|
//! | scene | **binary** (postcard) | the engine, at startup | fast and small; not mergeable |
//! | scene, editable | **TOML** | a person, and `git merge` | comments, line-oriented diffs |
//! | settings | **TOML** | a person | comments, and it is the file most often opened |
//!
//! The scene exists in both forms on purpose. Binary is unmergeable: two people
//! editing one scene produce a conflict no human can read. So the flow is
//! `binary → export → text → (edit, merge) → import → binary`. See
//! [`scene_text`].
//!
//! ## The header, and why it is not optional
//!
//! Every binary file starts with a magic, a format version, a **schema hash**
//! and a payload length. The schema hash is the one that matters: postcard is
//! positional, so a document whose layout changed decodes into *something*
//! rather than failing. Without the hash that something is silently wrong, and
//! the failure appears later as a scene that loads and renders wrong.
//!
//! ## What this replaces
//!
//! Upstream has no on-disk format beyond yaml-cpp for scenes, and an
//! `AssetID` that is a random `u64` minted per run whose header claims it
//! "survives file moves and renames". It does not, and there is no manifest.
//!
//! ## Not implemented
//!
//! No content-addressed identity yet, no VFS, no loaders. Those are in
//! `docs/PARITY.md` under Assets and are the next pieces; the formats are here
//! first because every loader needs one.

#![deny(missing_docs)]

pub mod components;
pub mod doc;
pub mod header;
pub mod scene_binary;
pub mod scene_text;
pub mod settings;

pub use components::{Component, ComponentKind};
pub use doc::{EntityDoc, InvalidScene, SceneDoc, Transform, Uuid};
pub use header::{Header, Incompatible, MAGIC};
pub use scene_binary::{LoadError, SCENE_SCHEMA};
pub use settings::{Assets, Backend, Graphics, Settings, Window};

/// Save a scene in both forms: the binary the engine loads, and the text a
/// person edits.
///
/// Both, always. Keeping them in step is the point -- a text form that has
/// drifted from its binary is worse than no text form, because it looks
/// authoritative.
pub fn save_scene(
    scene: &SceneDoc,
    binary_path: &std::path::Path,
    text_path: &std::path::Path,
) -> std::io::Result<()> {
    std::fs::write(binary_path, scene_binary::encode(scene))?;
    std::fs::write(text_path, scene_text::to_toml(scene))
}

/// Load a scene, preferring the binary form and falling back to text.
///
/// The fallback is what makes a repository of scenes editable: someone edits
/// `scene.toml`, runs `vault import`, and if they forget, the next load still
/// works. A `binary_path` that does not exist is not an error.
pub fn load_scene(
    binary_path: &std::path::Path,
    text_path: &std::path::Path,
) -> Result<SceneDoc, LoadFailure> {
    match std::fs::read(binary_path) {
        Ok(bytes) => return scene_binary::decode(&bytes).map_err(LoadFailure::Binary),
        // Not an error: the text form may be the only one there is.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(LoadFailure::Io {
                path: binary_path.into(),
                error,
            })
        }
    }

    let text = std::fs::read_to_string(text_path).map_err(|error| LoadFailure::Io {
        path: text_path.into(),
        error,
    })?;
    scene_text::from_toml(&text).map_err(LoadFailure::Text)
}

/// Why a scene did not load.
#[derive(Debug)]
pub enum LoadFailure {
    /// The binary form was rejected -- wrong magic, wrong version, or a schema
    /// this build does not speak.
    Binary(scene_binary::LoadError),
    /// The text form was rejected.
    Text(scene_text::TextError),
    /// Neither form could be read from disk.
    Io {
        /// The file that could not be read.
        path: std::path::PathBuf,
        /// The underlying OS error.
        error: std::io::Error,
    },
}

impl std::fmt::Display for LoadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadFailure::Binary(error) => write!(f, "{error}"),
            LoadFailure::Text(error) => write!(f, "{error}"),
            LoadFailure::Io { path, error } => {
                write!(f, "{}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadFailure::Binary(error) => Some(error),
            LoadFailure::Text(error) => Some(error),
            LoadFailure::Io { error, .. } => Some(error),
        }
    }
}
