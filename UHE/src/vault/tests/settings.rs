//! Settings: parsing, and the precedence rules.
//!
//! The tests that matter here are the ones about what happens when a key is
//! *wrong*. A settings file is the one file a person edits by hand, so the
//! failure mode is a typo, and the question is whether a typo is visible.

use vault::settings::{Backend, Settings};

#[test]
fn an_empty_file_is_all_defaults() {
    let settings = Settings::from_toml("").expect("an empty file is valid");
    assert_eq!(settings, Settings::default());
}

#[test]
fn a_partial_file_fills_in_the_rest() {
    let settings = Settings::from_toml(
        r#"
        [graphics]
        vsync = false
        render_scale = 0.75
        "#,
    )
    .expect("a partial file is valid");

    assert!(!settings.graphics.vsync);
    assert_eq!(settings.graphics.render_scale, 0.75);
    // Everything not mentioned keeps its default.
    assert!(settings.graphics.debug_labels);
    assert_eq!(settings.window.width, 1280);
    assert_eq!(settings.assets.search_paths, vec!["assets".to_string()]);
}

#[test]
fn a_misspelled_key_is_an_error_naming_the_key() {
    // The whole argument for `deny_unknown_fields`. Ignoring an unknown key
    // means a typo silently takes the default, and the symptom is a setting
    // that "does nothing" with nothing saying it was never read.
    let error = Settings::from_toml(
        r#"
        [graphics]
        vsnyc = false
        "#,
    )
    .expect_err("a misspelled key must not be ignored");

    let rendered = error.to_string();
    assert!(rendered.contains("vsnyc"), "unhelpful: {rendered}");
}

#[test]
fn to_toml_round_trips_and_documents_itself() {
    let settings = Settings::default();
    let text = settings.to_toml();

    // Every key present, so the file documents itself without external docs.
    for key in ["vsync", "render_scale", "backend", "title", "hot_reload"] {
        assert!(text.contains(key), "`{key}` missing from:\n{text}");
    }
    assert!(text.contains('#'), "and it must carry comments");

    let parsed = Settings::from_toml(&text).expect("our own output must parse");
    assert_eq!(parsed, settings);
}

#[test]
fn the_flag_beats_everything() {
    let mut settings = Settings::default();
    settings.graphics.backend = Backend::Wgpu;

    // A `--backend` flag is the most immediate expression of intent, so it
    // wins over both the environment and the file.
    assert_eq!(
        settings.resolve_backend(Some(Backend::Vulkan)),
        Backend::Vulkan
    );
}

#[test]
fn the_file_is_used_when_nothing_else_is_set() {
    let mut settings = Settings::default();
    settings.graphics.backend = Backend::Vulkan;
    assert_eq!(settings.resolve_backend(None), Backend::Vulkan);
}

#[test]
fn no_backend_anywhere_leaves_the_decision_to_the_caller() {
    let settings = Settings::default();
    assert_eq!(settings.graphics.backend, Backend::Auto);
    // `Auto` is passed through, not resolved here: "the settings did not
    // choose" stays distinguishable from "the settings chose wgpu".
    assert_eq!(settings.resolve_backend(None), Backend::Auto);
}

#[test]
fn backend_names_round_trip() {
    let settings = Settings::from_toml(
        r#"
        [graphics]
        backend = "vulkan"
        "#,
    )
    .expect("vulkan is a valid backend name");
    assert_eq!(settings.graphics.backend, Backend::Vulkan);
}

#[test]
fn an_unknown_backend_is_an_error() {
    // Unlike the environment variable, which is ignored. A file is a decision
    // someone made deliberately; `backend = "vulkank"` is a mistake worth
    // stopping for.
    assert!(Settings::from_toml(
        r#"
        [graphics]
        backend = "vulkank"
        "#,
    )
    .is_err());
}

#[test]
fn hot_reload_defaults_to_on_only_in_debug_builds() {
    // Reloading a shader in a shipped build is a bug, not a feature. But a
    // developer should not have to remember to switch it on.
    assert_eq!(
        Settings::default().assets.hot_reload,
        cfg!(debug_assertions)
    );
}

#[test]
fn a_scene_is_not_settings() {
    // Cheap, but worth pinning: the two formats are separate files and mixing
    // them would mean a scene file could set the backend.
    let scene_like = "schema = 1\nentities = []\n";
    assert!(Settings::from_toml(scene_like).is_err());
}
