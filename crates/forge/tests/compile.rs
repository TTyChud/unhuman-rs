//! End-to-end: drive the vendored Slang and check the SPIR-V that comes out.
//!
//! These tests need the compiler. They skip rather than fail when it is absent,
//! because `cargo test` has to work on a machine that has never cloned the C++
//! tree -- but a missing compiler is never silent, the skip says so.

use forge::slangc::{attach_locations, parse_diagnostics, SlangcCompiler};
use forge::cache::ShaderCache;
use forge::{compile_once, CompileRequest, Forge, Slangc, Stage};
use std::path::{Path, PathBuf};

fn compiler() -> Option<SlangcCompiler> {
    let compiler = SlangcCompiler::new(Slangc::default()).ok()?;
    // Construction already ran the binary to read its version, and an empty
    // string means it answered nothing -- so a successful build is not proof
    // that this compiler can actually compile.
    if compiler.version().is_empty() {
        None
    } else {
        Some(compiler)
    }
}

fn shader() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/shaders/Triangle.slang")
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join("forge-e2e");
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory.join(name)
}

/// Diagnostics are the entire value of a shader compiler, so they get parsed
/// rather than dumped.
#[test]
fn a_broken_shader_reports_the_line_it_is_on() {
    let text = "error[E30015]: undefined identifier\n --> /tmp/bad.slang:1:41\n  |\n1 | float4 m() { return nope; }\n";
    let mut diagnostics = parse_diagnostics(text);
    attach_locations(text, &mut diagnostics);

    assert_eq!(diagnostics.len(), 1, "one diagnostic, got {diagnostics:?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.severity, forge::Severity::Error);
    assert_eq!(diagnostic.code.as_deref(), Some("E30015"));
    assert_eq!(diagnostic.line, Some(1));
    assert_eq!(diagnostic.column, Some(41));
    assert_eq!(
        diagnostic.file.as_deref(),
        Some(Path::new("/tmp/bad.slang")),
        "the file points at the shader, not at the compiler",
    );
}

#[test]
fn a_good_shader_compiles_to_valid_spirv() {
    let Some(compiler) = compiler() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let output = scratch("e2e.vert.spv");
    let request = CompileRequest::new(shader(), "vs_main", Stage::Vertex);
    let module = forge::slangc::compile_once(&compiler, &request, &output)
        .expect("Triangle.slang compiles");

    assert!(module.is_valid(), "SPIR-V magic is checked on load");
    assert!(
        module.words.len() > 20,
        "expected real code, got {} words",
        module.words.len(),
    );
    assert!(
        !module.diagnostics.iter().any(|d| d.severity == forge::Severity::Error),
        "no errors, got {:?}",
        module.diagnostics,
    );
}

/// The whole point of `compile_pair`: Vulkan has no linked shader stage, so a
/// vertex/fragment pair is two modules from one source. A helper that returned
/// one module twice would produce a pipeline whose fragment stage is the vertex
/// stage, and it would create successfully.
#[test]
fn a_vertex_fragment_pair_is_two_modules_and_they_differ() {
    let Some(compiler) = compiler() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let forge = Forge::new(compiler, ShaderCache::disabled());
    let output_dir = scratch("pair");
    let (vertex, fragment) = forge
        .compile_pair(&shader(), "vs_main", "fs_main", &output_dir)
        .expect("the pair compiles");

    assert_eq!(vertex.stage, Stage::Vertex);
    assert_eq!(fragment.stage, Stage::Fragment);
    assert_ne!(
        vertex.words, fragment.words,
        "a vertex and a fragment stage are different code",
    );
    assert!(vertex.is_valid() && fragment.is_valid());
}

/// The second compile of an unchanged shader must come from the cache, and the
/// first must not. A cache that never hits is a performance bug; one that always
/// hits is a correctness bug.
#[test]
fn the_second_compile_is_a_cache_hit_and_the_first_is_not() {
    let Some(compiler) = compiler() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let root = std::env::temp_dir().join("forge-e2e-cache");
    let _ = std::fs::remove_dir_all(&root);
    let forge = Forge::new(compiler, ShaderCache::new(&root));
    let request = CompileRequest::new(shader(), "vs_main", Stage::Vertex);

    let first = forge
        .compile(&request, &scratch("e2e-cache.vert.spv"))
        .expect("first compile");
    assert!(!first.cached, "the first compile is a real one");

    let second = forge
        .compile(&request, &scratch("e2e-cache.vert.spv"))
        .expect("second compile");
    assert!(second.cached, "the second comes from the cache");
    assert_eq!(first.words, second.words, "and is identical");

    // The hit still wrote the file the caller asked for.
    assert!(
        scratch("e2e-cache.vert.spv").exists(),
        "a hit writes the requested output, or a caller that trusts the cache works until someone clears it",
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// A source edit must miss. This is the failure that matters most: a cache that
/// keeps serving the previous module after an edit is invisible until someone
/// notices the shader is wrong.
#[test]
fn editing_the_source_misses_the_cache() {
    let Some(compiler) = compiler() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let edited = scratch("edited.slang");
    let original = std::fs::read_to_string(shader()).expect("shader is readable");
    std::fs::write(&edited, original.replace("float3 position", "float3 pos")).expect("write");

    let root = std::env::temp_dir().join("forge-e2e-edit");
    let _ = std::fs::remove_dir_all(&root);
    let forge = Forge::new(compiler, ShaderCache::new(&root));

    let request = CompileRequest::new(&edited, "vs_main", Stage::Vertex);
    let first = forge.compile(&request, &scratch("e2e-edit.vert.spv")).expect("first");
    assert!(!first.cached);

    std::fs::write(&edited, original.replace("input.color", "input.tint")).expect("rewrite");
    let second = forge.compile(&request, &scratch("e2e-edit.vert.spv")).expect("second");
    assert!(!second.cached, "an edited source must not be served from cache");

    let _ = std::fs::remove_dir_all(&root);
}

/// A missing entry point has to fail loudly, with the name in the message.
#[test]
fn a_missing_entry_point_is_an_error_not_a_silent_success() {
    let Some(compiler) = compiler() else {
        eprintln!("skipping: the vendored slangc is not available");
        return;
    };

    let request = CompileRequest::new(shader(), "does_not_exist", Stage::Vertex);
    let error = compile_once(&compiler, &request, &scratch("missing.spv"))
        .expect_err("a missing entry point is an error");
    let text = error.to_string();
    assert!(
        text.contains("does_not_exist"),
        "the message names the entry point that was missing, got {text:?}",
    );
}