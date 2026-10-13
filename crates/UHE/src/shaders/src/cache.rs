//! Content-addressed shader cache.
//!
//! Keyed on the *source bytes* plus every compiler flag plus the compiler
//! version. All three matter, and the third is the one that is easy to forget:
//! Slang's output can legitimately change between versions, so a key that omits
//! it serves stale SPIR-V after a toolchain bump — a bug that looks like a
//! driver bug and survives every other check.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::request::{CompileRequest, CompiledModule};
use crate::ForgeError;

/// An FNV-1a hash, written out rather than pulled in.
///
/// A cache key needs a hash that cannot change between runs. `DefaultHasher`
/// carries no such promise — it is explicitly not stable across Rust versions —
/// so using it here would silently invalidate, or worse silently *not*
/// invalidate, across toolchain bumps.
#[derive(Clone, Copy)]
struct Fnv1a(u64);

impl Fnv1a {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

impl Fnv1a {
    fn write_str(&mut self, value: &str) {
        // Length-prefixed, so "ab" + "c" and "a" + "bc" cannot collide.
        self.write(&(value.len() as u64).to_le_bytes());
        self.write(value.as_bytes());
    }
}

/// The cache key for one request.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CacheKey(u64);

impl CacheKey {
    /// The source, the flags, and the compiler version.
    ///
    /// Returns `None` when the source cannot be read. That is not an error: a
    /// cache lookup for a file that does not exist should miss, not fail, and
    /// the compile that follows will report the real problem.
    pub fn new(request: &CompileRequest, compiler_version: &str) -> Option<Self> {
        let source = std::fs::read(&request.source).ok()?;
        let mut hash = Fnv1a::new();
        hash.write(&source);
        // Tag the domain, so a future key change cannot read an old layout.
        hash.write_str("forge-cache-v1");
        hash.write_str(compiler_version);
        hash.write_str(&request.source.display().to_string());
        for argument in request.arguments() {
            hash.write_str(&argument);
        }
        Some(Self(hash.finish()))
    }

    /// Two hex digits per byte, fanned out over subdirectories.
    ///
    /// One flat directory with a few thousand shaders is tens of thousands of
    /// entries in a single inode, which ext4 handles badly and every
    /// filesystem handles eventually badly.
    pub fn path(&self, root: &Path) -> PathBuf {
        let hex = format!("{:016x}", self.0);
        root.join(&hex[0..2])
            .join(&hex[2..4])
            .join(format!("{hex}.spv"))
    }
}

/// A cache over a directory of `.spv` files.
#[derive(Clone, Debug)]
pub struct ShaderCache {
    root: PathBuf,
    enabled: bool,
}

impl ShaderCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            enabled: true,
        }
    }

    /// A cache that never hits and never writes.
    ///
    /// What CI and a debug build want: a stale entry is worse than a slow
    /// build, and `assert`-on-everything is what the debug path is for.
    pub fn disabled() -> Self {
        Self {
            root: PathBuf::new(),
            enabled: false,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load(&self, key: &CacheKey) -> Option<CompiledModule> {
        if !self.enabled {
            return None;
        }
        let bytes = std::fs::read(key.path(&self.root)).ok()?;
        let mut module = CompiledModule::from_bytes(&bytes)?;
        module.cached = true;
        Some(module)
    }

    pub fn store(&self, key: &CacheKey, module: &CompiledModule) -> Result<(), ForgeError> {
        if !self.enabled {
            return Ok(());
        }
        let path = key.path(&self.root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write beside the target and rename: a reader that arrives mid-write
        // sees either the old module or the new one, never a half-written one.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = path.with_extension(format!("tmp{}-{sequence}", std::process::id()));
        std::fs::write(&temporary, module.to_bytes())?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), ForgeError> {
        if !self.enabled || !self.root.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}

/// Hash a value the same way the cache does, for tests and for callers that want
/// to know whether two requests would share an entry.
pub fn fingerprint(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
