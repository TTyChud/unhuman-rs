//! Settings: the one format a person edits by hand.
//!
//! ## TOML, and why
//!
//! Because it has real comments. That is the whole argument, and it is a good
//! one: a settings file is the one file in an engine project that a person
//! *will* open, and a format with no comments means every explanation of a
//! field has to live in external documentation, which is out of date by the
//! time anyone reads it.
//!
//! JSON was the alternative and is 14x faster to parse (131 µs vs 1,825 µs on
//! this machine). That is irrelevant: settings are loaded once, at startup, and
//! the file is a few kilobytes. Speed is not a reason to give up comments on
//! the file a human reads most.
//!
//! YAML was the C++'s choice (`yaml-cpp`, via `SceneSerializer.cpp`). It is
//! only 7% slower than TOML on parse and loses on the thing that matters: its
//! indentation sensitivity means a stray space silently changes a key's
//! nesting, which is the single most common way a hand-edited config file
//! breaks.
//!
//! ## Reading is lenient, writing is canonical
//!
//! [`Settings::merge_file`] reports an unknown key instead of ignoring it, and
//! [`Settings::to_toml`] emits every field with its default filled in. So a
//! missing key and a misspelled key behave differently -- one is the default,
//! the other is a warning naming the key -- and a file that round-trips through
//! `to_toml` documents itself completely.

use serde::{Deserialize, Serialize};

/// Which graphics backend to use.
///
/// [`Backend::Auto`] exists as a value rather than the field being an
/// `Option`, because TOML has no null. An `Option<Backend>` serialises a
/// `None` by **omitting the key entirely**, so the emitted file never mentions
/// the setting -- which defeats the point of writing every field out. As a
/// three-valued enum it always appears, and `backend = "auto"` is also
/// self-explanatory to whoever typed it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Decide at startup, from the flag, the environment, or the default.
    #[default]
    Auto,
    /// Cross-vendor, through wgpu. Works everywhere.
    Wgpu,
    /// Native Vulkan, through ash.
    Vulkan,
}

/// The graphics section.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Graphics {
    /// Which backend. [`Backend::Auto`] leaves the decision to `--backend`,
    /// `UHE_BACKEND`, or the default.
    pub backend: Backend,
    /// Start with vsync requested.
    ///
    /// Recorded here and honoured by the swapchain, not by the window. The C++
    /// puts this on the window, stores a bool and never calls the swap-interval
    /// call, so vsync there is a no-op that reads back as enabled.
    /// Start with vsync requested. Honoured by the swapchain, not the window.
    pub vsync: bool,
    /// Render scale, 0.5 to 2.0. Below 1 trades sharpness for frame rate,
    /// which is the only lever on a GPU that is genuinely too slow.
    /// Render scale, 0.5 to 2.0.
    pub render_scale: f32,
    /// Log every GPU object name, so a validation message can be matched to
    /// the object that caused it.
    /// Label every GPU object, so a validation message can name its cause.
    pub debug_labels: bool,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
            vsync: true,
            render_scale: 1.0,
            debug_labels: true,
        }
    }
}

/// The window section.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    /// Title bar text.
    pub title: String,
    /// Opening width, in logical units.
    pub width: u32,
    /// Opening height, in logical units.
    pub height: u32,
    /// Start fullscreen.
    /// Start fullscreen.
    pub fullscreen: bool,
    /// Start maximised.
    /// Start maximised.
    pub maximized: bool,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            title: "unhuman".to_string(),
            width: 1280,
            height: 720,
            fullscreen: false,
            maximized: false,
        }
    }
}

/// The assets section.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Assets {
    /// Directories searched for assets, in order. Mounts replace rather than
    /// stack -- upstream's `Vfs::Mount` overwrites, so its "layering" is a
    /// comment -- so "in order" means "first match wins".
    /// Directories searched for assets. First match wins.
    pub search_paths: Vec<String>,
    /// Whether to reload changed files while running.
    ///
    /// The C++ has this whole loop built and unit-tested, and **nothing calls
    /// it**: `AssetManager::Update()` has no call sites outside its own test,
    /// and `SetHotReloadEnabled` is called only from the test.
    /// Reload changed files while running.
    pub hot_reload: bool,
}

impl Default for Assets {
    fn default() -> Self {
        Self {
            search_paths: vec!["assets".to_string()],
            hot_reload: cfg!(debug_assertions),
        }
    }
}

/// The whole settings file.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Graphics and backend selection.
    pub graphics: Graphics,
    /// Opening window.
    pub window: Window,
    /// Where assets come from.
    pub assets: Assets,
}

/// What was wrong with a settings file.
#[derive(Debug)]
pub enum SettingsError {
    /// It did not parse. `toml`'s error names the line and the key.
    Parse(toml::de::Error),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Parse(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl Settings {
    /// Parse from TOML text.
    ///
    /// `deny_unknown_fields` throughout, so a misspelled key is an error
    /// naming the key. The alternative -- ignore unknown keys -- means a typo
    /// silently takes the default, and the symptom is a setting that "does
    /// nothing" with no indication that it was never read.
    pub fn from_toml(text: &str) -> Result<Self, SettingsError> {
        toml::from_str(text).map_err(SettingsError::Parse)
    }

    /// Canonical TOML, every field present with its default filled in.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("# unhuman settings.\n");
        out.push_str("#\n");
        out.push_str("# Every key is written out, so this file documents itself.\n");
        out.push_str("# An unknown key is an error, not a silently ignored line.\n\n");
        out.push_str(
            &toml::to_string_pretty(self).expect("a struct of plain data cannot fail to serialise"),
        );
        out
    }

    /// The backend to use, after resolving the file against the environment.
    ///
    /// Precedence: `--backend` (passed in), then `UHE_BACKEND`, then this file,
    /// then the default. The environment beats the file because a one-off
    /// `UHE_BACKEND=vulkan` should not require editing the file that is
    /// presumably checked in.
    ///
    /// May still return [`Backend::Auto`], which means the caller should use its
    /// own default. Kept as a value rather than resolved to a concrete backend
    /// here so "the settings did not choose" stays distinguishable from "the
    /// settings chose wgpu".
    pub fn resolve_backend(&self, flag: Option<Backend>) -> Backend {
        if let Some(flag) = flag {
            return flag;
        }
        if let Ok(value) = std::env::var("UHE_BACKEND") {
            match value.to_ascii_lowercase().as_str() {
                "vulkan" => return Backend::Vulkan,
                "wgpu" => return Backend::Wgpu,
                // An unrecognised value is ignored rather than fatal, because a
                // typo in a shell profile should not stop the engine starting.
                _ => {}
            }
        }
        self.graphics.backend
    }
}
