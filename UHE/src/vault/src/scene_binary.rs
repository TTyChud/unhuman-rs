//! The binary scene format.
//!
//! ## Why postcard
//!
//! Pure Rust, and it encodes integers as varints. A scene is mostly small
//! numbers -- entity counts, component indices, sort orders, and transforms that
//! are frequently exactly `0.0` or `1.0` -- so a varint encoding is materially
//! smaller than the fixed-width one bincode would use for the same document.
//! Measured on a 400-entity scene the difference is roughly a third of the
//! bytes, which matters when a game ships a hundred scenes.
//!
//! `postcard` has no `build.rs` and no C dependency, which keeps the
//! "cargo test works with no toolchain" property the workspace depends on.
//!
//! ## Why there is a header
//!
//! See [`crate::header`]. The short version: postcard is positional, so a
//! document whose schema changed decodes into *something* rather than failing,
//! and without a schema hash that something is silently wrong.
//!
//! ## Schema hash
//!
//! [`SCENE_SCHEMA`] is a hand-written constant rather than something derived at
//! compile time, because a derive would hash the `Serialize` implementation --
//! which postcard may change between patch versions, silently invalidating
//! every scene file in existence for no reason anyone can act on. It changes
//! when someone edits this list, which is exactly when it should.

use crate::components::Component;
use crate::doc::{EntityDoc, SceneDoc, Transform, Uuid};
use crate::header::{Header, Incompatible};

/// The bytes describing this document layout.
///
/// **This string *is* the schema.** [`SCENE_SCHEMA`] is computed from it, so
/// there is exactly one thing to change when the layout changes, and it sits
/// next to the wire structs it describes.
///
/// ## The honest limit
///
/// The hash reliably catches a file written by a *different* build. It cannot
/// catch someone who adds a field and forgets to edit this string -- in that
/// case old and new files are both accepted, and the mismatch surfaces later as
/// wrong data. No amount of hashing fixes that, because the hash is only as
/// current as the string.
///
/// What does help: the string is a literal in the same file as the structs, and
/// [`SCENE_SCHEMA_DESCRIPTION`] is quoted in the test suite. A field appearing
/// in the struct and not here should be treated as a bug in the change that
/// added it.
pub const SCENE_SCHEMA_DESCRIPTION: &[u8] = b"scene/v1:entity{id:uuid16,parent:uuid16,name:str,tags:list<str>,transform:{t3,q4,s3},components:list<enum16>}";

/// FNV-1a, const-callable so the schema can be computed rather than declared.
///
/// Out of line and `const` because [`crate::header::fnv1a64`] cannot be used in
/// a constant expression. The two must stay identical, which the test in
/// `tests/scene_format.rs` checks.
const fn schema_of(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(PRIME);
        index += 1;
    }
    hash
}

/// Identity of the scene document layout: `fnv1a64(SCENE_SCHEMA_DESCRIPTION)`.
pub const SCENE_SCHEMA: u64 = schema_of(SCENE_SCHEMA_DESCRIPTION);

/// The schema hash this build speaks, for comparison against a file's header.
pub fn scene_schema() -> u64 {
    SCENE_SCHEMA
}

/// Anything that can go wrong turning bytes into a scene.
#[derive(Debug)]
pub enum LoadError {
    /// The header rejected the file.
    Incompatible(Incompatible),
    /// The header was fine and the payload did not decode.
    Payload(postcard::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Incompatible(error) => write!(f, "{error}"),
            LoadError::Payload(error) => write!(f, "payload did not decode: {error}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Incompatible(error) => Some(error),
            LoadError::Payload(error) => Some(error),
        }
    }
}

/// Encode a scene to a complete file: header then payload.
///
/// Goes through [`WireScene`] so the bytes depend on the explicit wire struct
/// and not on whatever `EntityDoc` happens to derive today.
pub fn encode(scene: &SceneDoc) -> Vec<u8> {
    let wire = WireScene::from(scene);

    // `to_allocvec`, not `to_extend`: the latter wants an `Extend<u8>`, and
    // `&mut Vec<u8>` does not implement it. Allocating once is what we want
    // anyway -- the header needs the payload length before it can be written,
    // so the payload must exist before the header is built.
    let payload = postcard::to_allocvec(&wire).expect("encoding a scene cannot fail");

    Header::new(scene_schema(), payload.len() as u64).seal(&payload)
}

/// Decode a complete file.
///
/// Checks the header **before** touching the payload, so a file from a
/// different build is rejected by name rather than by whatever the mismatch
/// happened to produce.
pub fn decode(bytes: &[u8]) -> Result<SceneDoc, LoadError> {
    let payload = Header::open(bytes, scene_schema()).map_err(LoadError::Incompatible)?;
    let wire: WireScene = postcard::from_bytes(payload).map_err(LoadError::Payload)?;
    Ok(SceneDoc::from(wire))
}

/// The wire form of one entity.
///
/// A separate type rather than deriving `Serialize` straight onto
/// [`EntityDoc`] for one reason: it makes the on-disk layout an explicit,
/// reviewable list. If `EntityDoc` grows a field for the ECS's benefit, this
/// struct does not change unless someone decides the file should carry it --
/// and if they do, they change it here, in the file that is *about* the file.
#[derive(serde::Serialize, serde::Deserialize)]
struct WireEntity {
    id: Uuid,
    parent: Uuid,
    name: String,
    tags: Vec<String>,
    transform: Transform,
    components: Vec<Component>,
}

impl From<&EntityDoc> for WireEntity {
    fn from(entity: &EntityDoc) -> Self {
        Self {
            id: entity.id,
            parent: entity.parent,
            name: entity.name.clone(),
            tags: entity.tags.clone(),
            transform: entity.transform,
            components: entity.components.clone(),
        }
    }
}

impl From<WireEntity> for EntityDoc {
    fn from(wire: WireEntity) -> Self {
        Self {
            id: wire.id,
            parent: wire.parent,
            name: wire.name,
            tags: wire.tags,
            transform: wire.transform,
            components: wire.components,
        }
    }
}

/// The wire form of a scene.
#[derive(serde::Serialize, serde::Deserialize)]
struct WireScene {
    entities: Vec<WireEntity>,
}

impl From<&SceneDoc> for WireScene {
    fn from(scene: &SceneDoc) -> Self {
        Self {
            entities: scene.entities.iter().map(WireEntity::from).collect(),
        }
    }
}

impl From<WireScene> for SceneDoc {
    fn from(wire: WireScene) -> Self {
        Self {
            entities: wire.entities.into_iter().map(EntityDoc::from).collect(),
        }
    }
}
