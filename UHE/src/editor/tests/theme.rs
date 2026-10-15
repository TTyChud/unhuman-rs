//! The theme's persisted file: the real editor's, a missing one, and a broken
//! one.

use std::path::PathBuf;

use mirror::theme::{Rgba, Theme, ThemeId};

/// The real editor's file, checked in with the other fixtures.
fn reference_theme_file() -> PathBuf {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root")
        .to_path_buf();
    workspace.join("assets/themes/editor_theme.ini")
}

fn scratch(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("mirror_theme_{name}.ini"));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn the_real_editor_theme_parses_to_three_floats() {
    let path = reference_theme_file();
    assert!(path.is_file(), "missing fixture: {}", path.display());

    let theme = Theme::load(&path).expect("the shipped editor_theme.ini parses");
    assert_eq!(theme.selected, ThemeId::Emerald);
    assert!(!theme.custom_enabled);

    let accent = theme.custom_accent;
    for channel in [accent.r, accent.g, accent.b] {
        assert!((0.0..=1.0).contains(&channel), "{channel} out of range");
    }
    assert!((accent.r - 0.424).abs() < 1e-5);
    assert!((accent.g - 0.388).abs() < 1e-5);
    assert!((accent.b - 1.0).abs() < 1e-5);

    let palette = theme.palette();
    // No custom accent in the file, so the palette is Emerald's own.
    assert_eq!(palette.accent, Rgba::rgb(0.160, 0.780, 0.470));
    assert!(palette.accent_foreground.a > 0.0);
    assert!(palette.toolbar_bg.a > 0.0);
    assert_ne!(palette.console_error, palette.console_info);
}

#[test]
fn a_missing_file_is_the_defaults() {
    let path = std::env::temp_dir().join("mirror_theme_absent/editor_theme.ini");
    let theme = Theme::load(&path).expect("missing settings are not an error");
    assert_eq!(theme, Theme::new());
    assert_eq!(theme.selected, ThemeId::DarkViolet);
    assert!(!theme.custom_enabled);
}

#[test]
fn a_malformed_file_is_an_error() {
    for (name, contents) in [
        ("empty", ""),
        ("words", "violet midnight\nnot a number\n"),
        ("short", "0\n1\n"),
        ("index", "42\n0\n0.1 0.2 0.3\n"),
        ("channel", "0\n0\n0.1 0.2\n"),
    ] {
        let path = scratch(name, contents);
        assert!(
            Theme::load(&path).is_err(),
            "{name} should not have parsed: {contents:?}"
        );
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn a_custom_accent_round_trips_through_the_file() {
    let path = scratch("round_trip", "");
    let theme = Theme {
        selected: ThemeId::Graphite,
        custom_enabled: true,
        custom_accent: Rgba::rgb(0.25, 0.5, 0.75),
    };
    theme.save(&path).unwrap();
    let loaded = Theme::load(&path).unwrap();
    assert_eq!(loaded.selected, ThemeId::Graphite);
    assert!(loaded.custom_enabled);
    assert!((loaded.custom_accent.b - 0.75).abs() < 1e-6);
    let _ = std::fs::remove_file(path);
}

#[test]
fn the_gallery_lists_every_theme() {
    assert_eq!(ThemeId::all().len(), ThemeId::COUNT);
    for id in ThemeId::all() {
        assert_eq!(ThemeId::from_index(id.index()), Some(*id));
        assert!(!id.name().is_empty());
    }
    assert_eq!(ThemeId::from_index(ThemeId::COUNT), None);
}
