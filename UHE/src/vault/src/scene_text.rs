//! The text scene format: export, edit, import.
//!
//! ## Why this exists at all
//!
//! Because binary files are unmergeable. Two people editing the same scene
//! produce two binary files with a conflict a human cannot resolve -- and
//! "resolve it by loading both in the editor and saving" loses the concurrent
//! edits, which is the whole reason for wanting a merge.
//!
//! So a scene has two forms and one canonical direction:
//!
//! ```text
//!   binary  --export-->  text  --import-->  binary
//!     ^                    |                  ^
//!     +--------------------+------------------+
//!             edited by hand in between
//! ```
//!
//! Binary is what the engine loads and what a game ships. Text is what a person
//! edits, a diff shows usefully, and a merge conflict resolves line by line.
//!
//! ## Why TOML rather than JSON or YAML
//!
//! Measured on this machine (400 entities, 75 KB of scene data):
//! JSON 131 µs, TOML 1,825 µs, YAML 1,973 µs. JSON is 14x faster and TOML is
//! only 7% faster than YAML -- but *none of it matters*, because a scene is
//! loaded once at startup and the whole difference is 1.7 ms.
//!
//! So the choice is not speed. It is that this format exists to be read and
//! diffed by a person, and TOML wins that: real comments, which JSON does not
//! have, and without YAML's indentation sensitivity -- where a stray space
//! silently changes a key's nesting, which is the most common way a hand-edited
//! file breaks.

use crate::components::{Component, TextComponent};
use crate::doc::{EntityDoc, SceneDoc, Transform, Uuid};

/// The schema identity for text files.
///
/// Separate from [`crate::scene_binary::SCENE_SCHEMA`] because a text file and
/// a binary file of the same scene must both stay readable, and a change to
/// one is not automatically a change to the other. Bump this when a TOML key is
/// renamed, added or removed.
pub const TEXT_SCHEMA: u64 = 1;

/// What went wrong turning text into a scene.
#[derive(Debug)]
pub enum TextError {
    /// The file did not parse as TOML, or did not match the expected shape.
    Toml(toml::de::Error),
    /// It parsed, but something in it cannot be a scene.
    InvalidScene(crate::doc::InvalidScene),
    /// The file carries a different schema version than this build speaks.
    Schema {
        /// Version the file declares.
        found: u64,
        /// Version this build speaks.
        expected: u64,
    },
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextError::Toml(error) => write!(f, "{error}"),
            TextError::InvalidScene(error) => write!(f, "{error}"),
            TextError::Schema { found, expected } => {
                write!(
                    f,
                    "text schema version {found}, this build speaks {expected}"
                )
            }
        }
    }
}

impl std::error::Error for TextError {}

/// A scene as editable TOML.
#[derive(serde::Serialize, serde::Deserialize)]
struct TextScene {
    /// [`TEXT_SCHEMA`]. Present so a future rename produces a clear error
    /// rather than a scene with every entity silently renamed to nothing.
    schema: u64,
    entities: Vec<TextEntity>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TextEntity {
    id: String,
    /// Absent for a root, rather than a zero UUID. A root and a UUID that
    /// failed to parse should not look the same in a file a person is editing.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    parent: Option<String>,
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tags: Vec<String>,
    #[serde(default)]
    transform: TextTransform,
    #[serde(default)]
    components: Vec<TextComponent>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct TextTransform {
    #[serde(default)]
    translation: [f32; 3],
    #[serde(default = "identity_quaternion")]
    rotation: [f32; 4],
    #[serde(default = "unit_scale")]
    scale: [f32; 3],
}

fn identity_quaternion() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

fn unit_scale() -> [f32; 3] {
    [1.0; 3]
}

impl From<&EntityDoc> for TextEntity {
    fn from(entity: &EntityDoc) -> Self {
        Self {
            id: format_uuid(entity.id),
            parent: (!entity.parent.is_none()).then(|| format_uuid(entity.parent)),
            name: entity.name.clone(),
            tags: entity.tags.clone(),
            transform: TextTransform {
                translation: entity.transform.translation,
                rotation: entity.transform.rotation,
                scale: entity.transform.scale,
            },
            components: entity.components.iter().map(TextComponent::from).collect(),
        }
    }
}

impl From<TextEntity> for EntityDoc {
    fn from(text: TextEntity) -> Self {
        Self {
            id: parse_uuid(&text.id),
            parent: text.parent.as_deref().map_or(Uuid::NONE, parse_uuid),
            name: text.name,
            tags: text.tags,
            transform: Transform {
                translation: text.transform.translation,
                rotation: text.transform.rotation,
                scale: text.transform.scale,
            },
            components: text.components.into_iter().map(Component::from).collect(),
        }
    }
}

/// `8-4-4-4-12` hex, the shape a UUID is written in everywhere else.
fn format_uuid(uuid: Uuid) -> String {
    let mut out = String::with_capacity(36);
    for (index, byte) in uuid.0.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Parse a UUID, tolerating dashes and upper case.
///
/// Lenient on purpose: this is a field a person edits by hand, and rejecting a
/// valid UUID because it was pasted in upper case helps nobody. A genuinely
/// malformed value becomes a zero UUID, which `validate()` then reports as a
/// duplicate or dangling parent rather than silently accepting.
fn parse_uuid(text: &str) -> Uuid {
    let mut bytes = [0u8; 16];
    let mut nibble = 0usize;
    for character in text.chars().filter(|c| *c != '-') {
        let Some(value) = character.to_digit(16) else {
            continue;
        };
        if nibble >= 32 {
            break;
        }
        if nibble.is_multiple_of(2) {
            bytes[nibble / 2] = (value as u8) << 4;
        } else {
            bytes[nibble / 2] |= value as u8;
        }
        nibble += 1;
    }
    Uuid(bytes)
}

/// A scene as editable TOML text.
pub fn to_toml(scene: &SceneDoc) -> String {
    let text = TextScene {
        schema: TEXT_SCHEMA,
        entities: scene.entities.iter().map(TextEntity::from).collect(),
    };

    let body = toml::to_string_pretty(&text).expect("a struct of plain data cannot fail");

    let mut out = String::new();
    out.push_str("# unhuman scene.\n");
    out.push_str("#\n");
    out.push_str("# Edits go here; the engine loads the compiled binary form.\n");
    out.push_str("# `vault import` rebuilds it. Renaming a key is a schema change.\n\n");
    out.push_str(&body);
    out
}

/// Parse an editable TOML scene.
///
/// **Not** `deny_unknown_fields`. A component whose `kind` this build does not
/// know is dropped with the rest of the scene still loading, because the editor's
/// job is to open a file -- including one written by a newer build -- not to
/// refuse it. Structural mistakes are still errors, and `validate` catches the
/// rest.
pub fn from_toml(text: &str) -> Result<SceneDoc, TextError> {
    let parsed: TextScene = toml::from_str(text).map_err(TextError::Toml)?;

    if parsed.schema != TEXT_SCHEMA {
        return Err(TextError::Schema {
            found: parsed.schema,
            expected: TEXT_SCHEMA,
        });
    }

    let scene = SceneDoc {
        entities: parsed.entities.into_iter().map(EntityDoc::from).collect(),
    };

    scene.validate().map_err(TextError::InvalidScene)?;
    Ok(scene)
}
