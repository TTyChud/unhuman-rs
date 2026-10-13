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

use crate::request::{
    CompileRequest, CompiledModule, Diagnostic, Optimization, Severity,
};
use crate::ForgeError;

/// Where the vendored `slangc` lives, relative to this repository.
///
/// The C++ tree already vendors a built Slang distribution
/// (`UHE/vendor/bin/linux/slang/`), and using that exact compiler is the point:
/// a second compiler in the project would be a second set of bugs and a second
/// thing to patch.
const VENDORED: &str = "../unhuman/UHE/vendor/bin/linux/slang/bin/slangc";

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
        Self::Path(PathBuf::from(VENDORED))
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
        let temporary = output.with_extension(format!("tmp{}", std::process::id()));
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
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        // `error[E30015]: undefined identifier`
        let Some((severity, rest)) = split_severity(trimmed) else {
            continue;
        };
        // `rest` is still `[E30015]: undefined identifier` -- the bracket comes
        // before the code, not after the severity word.
        let (code, message) = match rest.split_once(']') {
            Some((code, message)) => match code.trim_start_matches('[') {
                code if code.starts_with('E') || code.starts_with('W') => {
                    (Some(code.to_owned()), message.trim_start_matches(':').trim())
                }
                _ => (None, rest),
            },
            None => (None, rest),
        };
        if message.is_empty() {
            continue;
        }
        out.push(Diagnostic {
            severity,
            code,
            file: None,
            line: None,
            column: None,
            message: message.to_owned(),
        });
    }
    out
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
