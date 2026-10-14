//! What to compile, and what came back.

use std::path::{Path, PathBuf};

use crate::stage::Stage;

/// A `-D` define.
///
/// Split from a plain `String` so a define can never be spelled `-D NAME=value`
/// by accident, which would define a macro with spaces in its name and fail
/// somewhere in the preprocessor with a message that does not mention the flag.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Define {
    pub name: String,
    pub value: Option<String>,
}

impl Define {
    pub fn flag(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: None,
        }
    }

    pub fn value(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: Some(value.into()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Optimization {
    None,
    O1,
    O2,
    O3,
}

impl Optimization {
    fn flag(self) -> String {
        match self {
            Optimization::None => "-O0".to_owned(),
            Optimization::O1 => "-O1".to_owned(),
            Optimization::O2 => "-O2".to_owned(),
            Optimization::O3 => "-O3".to_owned(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatrixLayout {
    RowMajor,
    ColumnMajor,
}

/// One compilation: one source file, one entry point, one stage.
///
/// A request is a value and hashes deterministically, because the cache is keyed
/// on it. Two requests that would produce different SPIR-V must never hash the
/// same, so every flag that reaches the compiler has to be a field here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompileRequest {
    pub source: PathBuf,
    pub entry: String,
    pub stage: Stage,
    /// The `sm_6_0`-style target profile.
    pub profile: String,
    pub defines: Vec<Define>,
    /// `-I` paths: where `#include` and `import` resolve.
    pub include_dirs: Vec<PathBuf>,
    pub optimization: Optimization,
    pub matrix_layout: Option<MatrixLayout>,
    /// Extra module search paths, for `import`.
    pub module_paths: Vec<PathBuf>,
}

impl CompileRequest {
    pub fn new(source: impl Into<PathBuf>, entry: impl Into<String>, stage: Stage) -> Self {
        Self {
            source: source.into(),
            entry: entry.into(),
            stage,
            // The floor the engine targets. Compiling higher hides a missing
            // feature rather than reporting it, so this is deliberately not
            // raised until a shader actually needs it.
            profile: "sm_6_0".to_owned(),
            defines: Vec::new(),
            include_dirs: Vec::new(),
            optimization: Optimization::None,
            matrix_layout: None,
            module_paths: Vec::new(),
        }
    }

    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = profile.into();
        self
    }

    pub fn define(mut self, define: Define) -> Self {
        self.defines.push(define);
        self
    }

    pub fn include(mut self, dir: impl Into<PathBuf>) -> Self {
        self.include_dirs.push(dir.into());
        self
    }

    pub fn module_path(mut self, dir: impl Into<PathBuf>) -> Self {
        self.module_paths.push(dir.into());
        self
    }

    pub fn optimization(mut self, level: Optimization) -> Self {
        self.optimization = level;
        self
    }

    pub fn matrix_layout(mut self, layout: MatrixLayout) -> Self {
        self.matrix_layout = Some(layout);
        self
    }

    /// Every flag passed to the compiler, in a fixed order.
    ///
    /// Order is fixed so the cache key and the command line cannot disagree, and
    /// so a failing compile can be reproduced by copying one line out of a log.
    pub fn arguments(&self) -> Vec<String> {
        let mut args = vec![
            "-target".to_owned(),
            "spirv".to_owned(),
            "-profile".to_owned(),
            self.profile.clone(),
            self.optimization.flag(),
            "-emit-spirv-directly".to_owned(),
            "-entry".to_owned(),
            self.entry.clone(),
            "-stage".to_owned(),
            self.stage.as_str().to_owned(),
        ];
        if let Some(layout) = self.matrix_layout {
            args.push(match layout {
                MatrixLayout::RowMajor => "-matrix-layout-row-major".to_owned(),
                MatrixLayout::ColumnMajor => "-matrix-layout-column-major".to_owned(),
            });
        }
        for dir in &self.include_dirs {
            args.push("-I".to_owned());
            args.push(dir.display().to_string());
        }
        for dir in &self.module_paths {
            args.push("-module-path".to_owned());
            args.push(dir.display().to_string());
        }
        for define in &self.defines {
            args.push("-D".to_owned());
            args.push(match &define.value {
                Some(value) => format!("{}={}", define.name, value),
                None => define.name.clone(),
            });
        }
        args.push(self.source.display().to_string());
        args
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Note,
    Warning,
    Error,
}

/// A compiler message, parsed so a tool can point at a line instead of dumping
/// text at someone.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Slang's error code, e.g. `E30015`. Stable enough to filter on.
    pub code: Option<String>,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

/// Compiled SPIR-V, plus enough provenance to explain it later.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompiledModule {
    pub stage: Stage,
    pub entry: String,
    /// SPIR-V as `u32` words, which is how `ash::vk::ShaderModule` wants it and
    /// how the backend hands it over with no copy.
    pub words: Vec<u32>,
    pub diagnostics: Vec<Diagnostic>,
    /// Where this came from: a cache hit or a real compile.
    pub cached: bool,
}

impl CompiledModule {
    /// The SPIR-V magic number. Checked on every load, because a module that is
    /// truncated or misaligned otherwise fails at `vkCreateShaderModule` with a
    /// error that blames the driver.
    pub const MAGIC: u32 = 0x0723_0203;

    pub fn is_valid(&self) -> bool {
        self.words.first().copied() == Some(Self::MAGIC)
    }

    /// Little-endian bytes, for writing to disk.
    ///
    /// Written out word by word rather than reinterpreting the slice. A
    /// `from_raw_parts` here would be sound but carries an `unsafe` in a crate
    /// that otherwise has none, to save a copy that happens once per shader.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.words.len() * 4);
        for word in &self.words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 || !bytes.len().is_multiple_of(4) {
            return None;
        }
        let (chunks, _) = bytes.as_chunks::<4>();
        let words = chunks
            .iter()
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect::<Vec<u32>>();
        if words.first().copied() != Some(Self::MAGIC) {
            return None;
        }
        Some(Self {
            stage: Stage::Vertex,
            entry: String::new(),
            words,
            diagnostics: Vec::new(),
            cached: false,
        })
    }
}

/// Read a file as a compile request's worth of nothing: a helper for loading
/// checked-in `.spv` next to a source file, which is how tests get a shader
/// without a compiler.
pub fn load_spirv(path: &Path) -> Option<CompiledModule> {
    let bytes = std::fs::read(path).ok()?;
    CompiledModule::from_bytes(&bytes)
}
