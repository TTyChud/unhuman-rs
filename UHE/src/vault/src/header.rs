//! The file header, and the compatibility gate it exists to enforce.
//!
//! ## Why a header at all
//!
//! Because a binary format without one does not fail when it meets an older
//! reader. It *misreads*. A scene saved by a build that added a field will
//! decode into a document with that field's bytes reinterpreted as whatever
//! came next in the schema -- usually plausible, never correct, and sometimes
//! not even a length that fits in the buffer.
//!
//! So every file starts with four things:
//!
//! | field | why it is there |
//! |---|---|
//! | magic | to reject a file that is not ours before parsing any of it |
//! | format version | to reject a deliberately older or newer encoding |
//! | schema hash | to reject a same-version file whose *meaning* differs |
//! | payload length | so a truncated file is a clean error, not a panic |
//!
//! The schema hash is the one that earns its keep. A format version only
//! changes when someone remembers to bump it; the hash changes the moment a
//! field's type or position does, whether or not anyone noticed.
//!
//! ## What this replaces
//!
//! Upstream has no equivalent. Its `AssetID` is a random UUID minted per run
//! (`UUID.cpp:22-28`) whose header claims it "survives file moves and renames"
//! -- it does not, and `grep -rn manifest` across the engine returns nothing.
//! It has no on-disk format at all beyond yaml-cpp for scenes.

use std::fmt;

/// Four bytes, chosen so a text file or a PNG cannot be mistaken for one.
pub const MAGIC: [u8; 4] = *b"UHV1";

/// Bump when the **encoding** changes: a different postcard configuration, a
/// different field order, a different container.
///
/// Bumping this is almost always the wrong fix. Prefer letting the schema hash
/// catch the change, so the file is rejected with a message naming the
/// difference rather than with "wrong version".
pub const FORMAT_VERSION: u32 = 1;

/// How a file was rejected.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Incompatible {
    /// The magic does not match, so this is not one of our files.
    NotOurs {
        /// The first four bytes we actually read.
        found: [u8; 4],
    },
    /// Our file, but an encoding version this build does not speak.
    FormatVersion {
        /// What the file declares.
        found: u32,
        /// What this build understands.
        expected: u32,
    },
    /// Our file, same format, but a different document schema.
    ///
    /// The case that matters. A field was added, removed, renamed or
    /// retyped. The file is readable in principle and wrong in practice, and
    /// that is exactly the combination that produces a scene which loads
    /// without error and renders wrong.
    Schema {
        /// Schema hash recorded in the file.
        found: u64,
        /// Schema hash this build produces.
        expected: u64,
    },
    /// The declared payload length does not match what is in the file.
    Truncated {
        /// Bytes the header promised.
        declared: u64,
        /// Bytes actually present.
        present: u64,
    },
}

impl fmt::Display for Incompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Incompatible::NotOurs { found } => write!(
                f,
                "not an engine file: magic is {:?}, expected {:?}",
                String::from_utf8_lossy(found),
                String::from_utf8_lossy(&MAGIC)
            ),
            Incompatible::FormatVersion { found, expected } => {
                write!(f, "format version {found}, this build speaks {expected}")
            }
            Incompatible::Schema { found, expected } => write!(
                f,
                "schema hash {found:#018x}, this build produces {expected:#018x}. \
                 The file is from a build whose document layout differs -- a field was \
                 added, removed or retyped. Export it to text from the matching build \
                 (`vault export`) and re-import here."
            ),
            Incompatible::Truncated { declared, present } => write!(
                f,
                "file is truncated: header declares {declared} payload bytes, found {present}"
            ),
        }
    }
}

impl std::error::Error for Incompatible {}

/// The header every vault file starts with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Header {
    /// Encoding version. See [`FORMAT_VERSION`].
    pub format_version: u32,
    /// Identity of the document layout. See [`Header::open`].
    pub schema_hash: u64,
    /// Payload bytes that follow, so truncation is a clean error.
    pub payload_len: u64,
}

impl Header {
    /// Build a header for a payload of this build's schema.
    pub fn new(schema_hash: u64, payload_len: u64) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            schema_hash,
            payload_len,
        }
    }

    /// Wrap a payload, producing a complete file.
    pub fn seal(&self, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&self.format_version.to_le_bytes());
        out.extend_from_slice(&self.schema_hash.to_le_bytes());
        out.extend_from_slice(&self.payload_len.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Read and check a header, returning the payload.
    ///
    /// Checks in a fixed order -- magic, version, schema, length -- so the
    /// error names the *first* thing wrong with the file rather than whatever
    /// a garbage parse produced.
    pub fn open(bytes: &[u8], expected_schema: u64) -> Result<&[u8], Incompatible> {
        if bytes.len() < HEADER_LEN {
            // Too short to even hold a magic. Reported as truncation rather
            // than as "not ours", because an empty file is more likely a
            // half-written one than something else entirely.
            return Err(Incompatible::Truncated {
                declared: HEADER_LEN as u64,
                present: bytes.len() as u64,
            });
        }

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        if magic != MAGIC {
            return Err(Incompatible::NotOurs { found: magic });
        }

        let format_version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
        if format_version != FORMAT_VERSION {
            return Err(Incompatible::FormatVersion {
                found: format_version,
                expected: FORMAT_VERSION,
            });
        }

        let schema_hash = u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
        if schema_hash != expected_schema {
            return Err(Incompatible::Schema {
                found: schema_hash,
                expected: expected_schema,
            });
        }

        let payload_len = u64::from_le_bytes(bytes[16..24].try_into().expect("8 bytes"));
        let present = (bytes.len() - HEADER_LEN) as u64;
        if payload_len != present {
            return Err(Incompatible::Truncated {
                declared: payload_len,
                present,
            });
        }

        Ok(&bytes[HEADER_LEN..])
    }
}

/// 4 magic + 4 version + 8 schema + 8 length.
pub const HEADER_LEN: usize = 24;

/// FNV-1a over a slice.
///
/// Chosen for being four lines and obviously correct rather than for being
/// fast. This runs once per file write, over a hash *description* rather than
/// over the file, so its speed is irrelevant -- what matters is that it cannot
/// accidentally change between builds, which is why it is spelled out here
/// instead of pulled from a crate whose internals may move.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}
