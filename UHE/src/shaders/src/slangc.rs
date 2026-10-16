//! The Slang compiler driver.
//!
//! Wraps the `slangc` binary rather than linking `libslang.so` in process. That
//! is a deliberate first cut, and the reason is the ABI: the Slang C API is
//! made of C++ virtual interfaces (`slang::ISession`, `slang::IModule`), so
//! binding it means binding a C++ vtable layout across a crate boundary, plus
//! libclang as a build-time dependency for `bindgen`. The driver gives us the
//! same compiler, the same diagnostics and the same output today, with none of
//! that risk — and it sits behind [`ShaderCompiler`], so swapping in the FFI
//! later is a new implementation of one trait, not a change to any caller.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::request::{CompileRequest, CompiledModule, Diagnostic, Optimization, Severity};
use crate::ForgeError;

/// Where the vendored `slangc` lives, as a path relative to the *workspace* root.
///
/// The C++ tree already vendors a built Slang distribution
/// (`UHE/vendor/bin/linux/slang/`), and using that exact compiler is the point:
/// a second compiler in the project would be a second set of bugs and a second
/// thing to patch.
///
/// A path relative to the workspace, never to the current directory. `cargo
/// test` runs each test binary with the *crate* directory as its working
/// directory, so a `../UNHUMAN/...` relative to the root resolves to nothing and
/// every shader test silently skips while reporting success.
const VENDORED: &str = "UNHUMAN/UHE/vendor/bin/linux/slang/bin/slangc";

/// Find a usable `slangc`.
///
/// In order: the `SLANGC` environment variable, the vendored copy, then `PATH`.
/// The vendored copy is found by walking up from this crate's manifest
/// directory, because the crate may be built from anywhere in the tree.
pub fn discover() -> Slangc {
    if let Some(path) = std::env::var_os("SLANGC") {
        return Slangc::Path(PathBuf::from(path));
    }
    if let Some(path) = vendored_path() {
        return Slangc::Path(path);
    }
    Slangc::System
}

/// Walk up from this crate looking for the C++ tree.
fn vendored_path() -> Option<PathBuf> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        // `crates/forge` -> workspace root -> the sibling `unhuman`.
        let candidate = ancestor.join(VENDORED);
        if candidate.is_file() {
            return Some(candidate);
        }
        // The C++ tree is a sibling of this workspace, not a child of it.
        let sibling = ancestor.join("..").join(VENDORED);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    None
}

/// Which slangc to run.
///
/// Resolved once at startup rather than per compile: a `which` per shader turns
/// a missing compiler into an N-second hang across a whole asset scan.
#[derive(Clone, Debug)]
pub enum Slangc {
    Path(PathBuf),
    /// Look in `PATH`.
    System,
}

impl Default for Slangc {
    fn default() -> Self {
        discover()
    }
}

impl Slangc {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::Path(path.into())
    }

    fn executable(&self) -> PathBuf {
        match self {
            Slangc::Path(path) => path.clone(),
            Slangc::System => PathBuf::from("slangc"),
        }
    }

    /// The compiler's version string, e.g. `2026.11`.
    ///
    /// Part of the cache key. A cached module built by a different Slang must
    /// not be reused, because the same source can legitimately produce
    /// different SPIR-V across versions — so a key that omits this serves stale
    /// shaders after a toolchain bump, which is close to undebuggable.
    pub fn version(&self) -> Result<String, ForgeError> {
        let output = Command::new(self.executable())
            .arg("-v")
            .output()
            .map_err(|error| ForgeError::Spawn {
                program: self.executable(),
                error,
            })?;
        // slangc prints the version and exits non-zero; that is its convention,
        // not a failure.
        let text = String::from_utf8_lossy(&output.stdout);
        let version = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("unknown")
            .to_owned();
        Ok(version)
    }
}

/// Compile with `slangc`, into a temporary file the caller then reads.
pub struct SlangcCompiler {
    slangc: Slangc,
    version: String,
}

impl SlangcCompiler {
    pub fn new(slangc: Slangc) -> Result<Self, ForgeError> {
        let version = slangc.version()?;
        Ok(Self { slangc, version })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Run the compiler once, writing SPIR-V to `output`.
    ///
    /// Split out from the cache so the cache decides *whether* to call this,
    /// and so a test can force a real compile without touching the cache.
    pub fn compile_to(
        &self,
        request: &CompileRequest,
        output: &Path,
    ) -> Result<Vec<Diagnostic>, ForgeError> {
        // Unique per compilation, not per process. Keying it on the PID means two
        // compiles in one process to the same output name race on the same temp
        // file -- which is exactly what a build system does, and what a
        // multi-threaded asset scan does. One clobbers the other's module and the
        // failure is a wrong shader rather than an error.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = output.with_extension(format!("tmp{}-{sequence}", std::process::id()));

        // Create the output directory rather than reporting a missing one. The
        // compiler's own error for this is `E00004 cannot write output file`,
        // which reads like a permissions problem and sends you looking at the
        // temp directory. The cache path already creates parents; this one
        // should not be the odd case.
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut command = Command::new(self.slangc.executable());
        command.args(request.arguments());
        command.arg("-o").arg(&temporary);

        let result = command.output().map_err(|error| ForgeError::Spawn {
            program: self.slangc.executable(),
            error,
        })?;

        let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
        let diagnostics = parse_diagnostics(&stderr);

        if !result.status.success() {
            // Leave nothing behind: a stale .tmp next to a good .spv is how a
            // later cache read returns the wrong module.
            let _ = std::fs::remove_file(&temporary);
            return Err(ForgeError::Compile {
                request: Box::new(request.clone()),
                diagnostics,
            });
        }

        // Rename without reading: if the module is not valid SPIR-V the caller
        // checks the magic, and a driver bug should not be masked by a
        // successful read here.
        std::fs::rename(&temporary, output)?;
        Ok(diagnostics)
    }
}

/// Parse `slangc`'s diagnostics.
///
/// The format is stable and worth parsing rather than dumping: a caller that
/// gets `file:line:column` back can point at the shader, and one that gets a
/// blob has to grep.
pub fn parse_diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();

        if let Some((severity, rest)) = split_severity(trimmed) {
            let (code, head) = match rest.split_once(']') {
                Some((code, message)) => match code.trim_start_matches('[') {
                    code if code.starts_with('E') || code.starts_with('W') => (
                        Some(code.to_owned()),
                        message.trim_start_matches(':').trim(),
                    ),
                    _ => (None, rest),
                },
                None => (None, rest),
            };
            if !head.is_empty() {
                out.push(Diagnostic {
                    severity,
                    code,
                    file: None,
                    line: None,
                    column: None,
                    message: head.to_owned(),
                });
            }
            continue;
        }

        // Continuation of the previous diagnostic. Slang puts the *name* of the
        // offending identifier on a later line, after a caret:
        //
        //     error[E30015]: undefined identifier
        //      --> a.slang:1:41
        //       1 | ... return nope; }
        //         |                 ^^^^ undefined identifier 'nope'.
        //
        // Taking only the headline produced a diagnostic reading "undefined
        // identifier" with nothing identifying what was undefined -- which is the
        // one thing you need. The echoed source line is skipped; the caret line
        // is not, because that is where the name is.
        let Some(last) = out.last_mut() else {
            continue;
        };
        if trimmed.starts_with("-->") || trimmed.is_empty() {
            continue;
        }
        // `12 | source text` is the echo. Skip the line number, keep the rest.
        let echo = split_source_echo(trimmed);
        if let Some(echo) = echo {
            if echo.contains('^') {
                last.message.push(' ');
                last.message.push_str(echo.trim());
            }
            continue;
        }
        last.message.push(' ');
        last.message.push_str(trimmed);
    }
    out
}

/// `1 | float4 m()` is the echoed source; the part after the pipe is prose
/// attached to the caret run on the same line.
fn split_source_echo(line: &str) -> Option<&str> {
    let (number, rest) = line.split_once('|')?;
    number.trim().parse::<u32>().ok()?;
    Some(rest)
}

fn split_severity(line: &str) -> Option<(Severity, &str)> {
    for (word, severity) in [
        ("error", Severity::Error),
        ("warning", Severity::Warning),
        ("note", Severity::Note),
    ] {
        if let Some(rest) = line.strip_prefix(word) {
            return Some((severity, rest));
        }
    }
    None
}

/// Collect `file:line:column` from the `--> path:1:41` lines that follow a
/// diagnostic, and attach them to it.
///
/// `slangc` emits them on the line after the message, so this walks the whole
/// output rather than parsing line pairs.
pub fn attach_locations(text: &str, diagnostics: &mut [Diagnostic]) {
    let mut pending: Option<usize> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("-->") {
            let location = rest.trim();
            if let Some(index) = pending.take() {
                if let Some((file, line, column)) = parse_location(location) {
                    diagnostics[index].file = Some(file);
                    diagnostics[index].line = Some(line);
                    diagnostics[index].column = Some(column);
                }
            }
            continue;
        }
        if split_severity(trimmed).is_some() {
            pending = diagnostics
                .iter()
                .rposition(|d| d.file.is_none() && d.line.is_none());
        }
    }
}

/// `path:line:column`, split from the right.
///
/// Right-first, and that order matters: taking the last colon as the file
/// boundary first would make `a.slang:1:41` parse as file `a.slang:1`, leave
/// `41` with no line to pair, and drop the whole location -- which is exactly
/// the parse that was there and returned nothing. Splitting right-to-left also
/// survives a Windows drive letter in the path, which a left-to-right split
/// would not.
fn parse_location(text: &str) -> Option<(PathBuf, u32, u32)> {
    let (rest, column) = text.rsplit_once(':')?;
    let (file, line) = rest.rsplit_once(':')?;
    let line = line.parse().ok()?;
    let column = column.parse().ok()?;
    Some((PathBuf::from(file.trim()), line, column))
}

/// Convenience: compile and load in one step, no cache.
pub fn compile_once(
    slangc: &SlangcCompiler,
    request: &CompileRequest,
    output: &Path,
) -> Result<CompiledModule, ForgeError> {
    let diagnostics = slangc.compile_to(request, output)?;
    let bytes = std::fs::read(output)?;
    let mut module = CompiledModule::from_bytes(&bytes).ok_or_else(|| ForgeError::NotSpirv {
        path: output.to_path_buf(),
    })?;
    module.stage = request.stage;
    module.entry = request.entry.clone();
    module.diagnostics = diagnostics;
    Ok(module)
}

/// The default optimisation for a given use.
///
/// Debug builds get none, because `-O0` keeps the SPIR-V close to the source and
/// a bug is then readable in the disassembly. Release gets `-O3`, because that is
/// the only setting where the compiler can fold a constant that the CPU path
/// would have folded anyway.
pub fn default_optimization(release: bool) -> Optimization {
    if release {
        Optimization::O3
    } else {
        Optimization::None
    }
}
