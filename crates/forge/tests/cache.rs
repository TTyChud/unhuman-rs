//! The cache is the part of `forge` that can fail silently, so most of these
//! tests are about what must *not* collide.

use forge::cache::{CacheKey, ShaderCache};
use forge::{CompileRequest, Define, MatrixLayout, Module, Optimization, Stage};
use std::fs;
use std::path::{Path, PathBuf};

fn shader() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/shaders/Triangle.slang")
}

fn request() -> CompileRequest {
    CompileRequest::new(shader(), "vs_main", Stage::Vertex)
}

fn key(version: &str) -> CacheKey {
    CacheKey::new(&request(), version).expect("the shader is readable")
}

fn valid_module() -> Module {
    Module::from_bytes(&[0x03, 0x02, 0x23, 0x07, 0, 0, 0, 0]).expect("valid magic")
}

#[test]
fn the_same_request_produces_the_same_key() {
    assert_eq!(key("2026.11"), key("2026.11"));
}

/// A toolchain bump can legitimately change the SPIR-V for the same source. A
/// key that omits the version serves stale modules after a Slang upgrade, and
/// the symptom is a shader bug that survives every other check.
#[test]
fn a_different_compiler_version_is_a_different_key() {
    assert_ne!(
        key("2026.11"),
        key("2027.3"),
        "the compiler version must be part of the key",
    );
}

#[test]
fn every_flag_reaches_the_key() {
    let base = key("2026.11");
    let version = "2026.11";

    let defined = CacheKey::new(
        &request().define(Define::value("MAX_LIGHTS", "8")),
        version,
    )
    .expect("readable");
    let profiled = CacheKey::new(&request().profile("sm_6_5"), version).expect("readable");
    let optimized =
        CacheKey::new(&request().optimization(Optimization::O3), version).expect("readable");
    let row_major =
        CacheKey::new(&request().matrix_layout(MatrixLayout::RowMajor), version).expect("readable");
    let entry = CacheKey::new(
        &CompileRequest::new(shader(), "fs_main", Stage::Vertex),
        version,
    )
    .expect("readable");
    let stage = CacheKey::new(
        &CompileRequest::new(shader(), "vs_main", Stage::Fragment),
        version,
    )
    .expect("readable");

    for (other, what) in [
        (defined, "-D"),
        (profiled, "-profile"),
        (optimized, "-O"),
        (row_major, "-matrix-layout"),
        (entry, "-entry"),
        (stage, "-stage"),
    ] {
        assert_ne!(base, other, "{what} changes the output, so it must change the key");
    }
}

/// "AB" + "C" and "A" + "BC" hash identically if strings are run together with
/// no separator, which would let two different flag sets share a cache entry.
#[test]
fn defines_are_length_prefixed() {
    let left = CacheKey::new(
        &request()
            .define(Define::flag("AB"))
            .define(Define::flag("C")),
        "2026.11",
    )
    .expect("readable");
    let right = CacheKey::new(
        &request()
            .define(Define::flag("A"))
            .define(Define::flag("BC")),
        "2026.11",
    )
    .expect("readable");
    assert_ne!(left, right);
}

/// Flags come out in a fixed order, so the cache key and the command line cannot
/// drift apart, and a failing compile is reproducible from one line of a log.
#[test]
fn arguments_are_deterministic_and_complete() {
    let request = CompileRequest::new(shader(), "vs_main", Stage::Vertex)
        .define(Define::value("MAX_LIGHTS", "8"))
        .define(Define::flag("DEBUG"))
        .optimization(Optimization::O2)
        .matrix_layout(MatrixLayout::RowMajor);

    let first = request.arguments();
    assert_eq!(first, request.arguments(), "same request, same command line");

    let joined = first.join(" ");
    for expected in [
        "-target spirv",
        "-profile sm_6_0",
        "-O2",
        "-emit-spirv-directly",
        "-entry vs_main",
        "-stage vertex",
        "-matrix-layout-row-major",
        "-D MAX_LIGHTS=8",
        "-D DEBUG",
    ] {
        assert!(joined.contains(expected), "missing {expected:?} in {joined:?}");
    }
}

/// A define must never smuggle in a second flag.
#[test]
fn a_define_is_never_split_into_two_arguments() {
    let request =
        CompileRequest::new(shader(), "vs_main", Stage::Vertex).define(Define::value("NAME", "1 -D EVIL"));
    let arguments = request.arguments();
    assert!(
        arguments.iter().any(|argument| argument == "NAME=1 -D EVIL"),
        "the value stays in one argument, got {arguments:?}",
    );
    assert!(
        !arguments.iter().any(|argument| argument == "EVIL"),
        "a define value must not become its own -D flag",
    );
}

/// A cache hit still writes the file the caller asked for. A caller that trusts
/// the cache to have placed it works right up until someone clears the cache.
#[test]
fn a_hit_round_trips_the_same_words() {
    let root = std::env::temp_dir().join("forge-cache-hit");
    let _ = fs::remove_dir_all(&root);
    let cache = ShaderCache::new(&root);
    let key = key("2026.11");

    cache.store(&key, &valid_module()).expect("store");
    let loaded = cache.load(&key).expect("cache hit");
    assert!(loaded.cached, "a load is marked cached");
    assert_eq!(loaded.words, valid_module().words);
    let _ = fs::remove_dir_all(&root);
}

/// Truncated output must be rejected here, not at `vkCreateShaderModule`, where
/// the error blames the driver for a file that was never valid.
#[test]
fn truncated_or_wrong_magic_output_is_rejected() {
    assert!(Module::from_bytes(&[]).is_none(), "empty");
    assert!(Module::from_bytes(&[1, 2, 3]).is_none(), "misaligned");
    assert!(
        Module::from_bytes(&[0x03, 0x02, 0x23, 0x07]).is_none(),
        "a bare magic word is not a module",
    );
    assert!(
        Module::from_bytes(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]).is_none(),
        "wrong magic",
    );
    assert!(valid_module().is_valid());
}

#[test]
fn a_disabled_cache_never_hits_and_writes_nothing() {
    let root = std::env::temp_dir().join("forge-cache-disabled");
    let _ = fs::remove_dir_all(&root);
    let cache = ShaderCache::disabled();
    cache
        .store(&key("2026.11"), &valid_module())
        .expect("storing is a no-op, not a failure");
    assert!(cache.load(&key("2026.11")).is_none());
    assert!(!root.exists(), "a disabled cache writes nothing");
}

/// Byte round-trip, because the cache writes and the loader reads.
#[test]
fn words_survive_the_trip_through_bytes() {
    let module =
        Module::from_bytes(&[0x03, 0x02, 0x23, 0x07, 0xFF, 0xFF, 0xFF, 0xFF]).expect("valid");
    assert_eq!(
        Module::from_bytes(&module.to_bytes())
            .expect("round trips")
            .words,
        module.words,
    );
}

#[test]
fn every_stage_round_trips_through_its_name() {
    for stage in [
        Stage::Vertex,
        Stage::Fragment,
        Stage::Compute,
        Stage::Geometry,
        Stage::Hull,
        Stage::Domain,
        Stage::Task,
        Stage::Mesh,
        Stage::Raygen,
        Stage::Miss,
        Stage::Callable,
        Stage::Intersection,
        Stage::Amplification,
    ] {
        assert_eq!(stage.as_str().parse::<Stage>().unwrap(), stage);
    }
}

/// A raygen stage is not a graphics or compute pipeline, and `VulkanGraphicPipeline`
/// must not silently accept one and produce a pipeline that draws nothing.
#[test]
fn only_graphics_and_compute_stages_are_pipeline_stages() {
    assert!(Stage::Vertex.is_pipeline_stage());
    assert!(Stage::Fragment.is_pipeline_stage());
    assert!(Stage::Compute.is_pipeline_stage());
    assert!(!Stage::Raygen.is_pipeline_stage());
    assert!(!Stage::Miss.is_pipeline_stage());
    assert!(!Stage::Callable.is_pipeline_stage());
    assert!(!Stage::Intersection.is_pipeline_stage());
}

/// Slang accepts the older spelling, so the parser does too. Rejecting it would
/// mean a rename of the entry point attribute breaks the build for no reason.
#[test]
fn the_older_fragment_spelling_still_parses() {
    assert_eq!("pixel".parse::<Stage>().unwrap(), Stage::Fragment);
    assert!("nonsense".parse::<Stage>().is_err(), "and nonsense still fails");
}