//! The editor palette: the C++ `EditorTheme.cpp` table, its persisted
//! selection, and the derived colors panels draw with. The ImGui style it
//! wrote into is a UI toolkit's problem, so only the values survive here.

use std::fmt;
use std::io;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Relative luminance, Rec. 709: what a color on top of this one must
    /// contrast against.
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
}

/// t > 0 toward white, t < 0 toward black.
fn shade(c: Rgba, t: f32) -> Rgba {
    if t >= 0.0 {
        Rgba::new(
            c.r + (1.0 - c.r) * t,
            c.g + (1.0 - c.g) * t,
            c.b + (1.0 - c.b) * t,
            c.a,
        )
    } else {
        Rgba::new(c.r * (1.0 + t), c.g * (1.0 + t), c.b * (1.0 + t), c.a)
    }
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba::new(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a,
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ThemeId {
    DarkViolet,
    Midnight,
    Graphite,
    Light,
    Emerald,
    Rose,
    Ocean,
    Sunset,
    Nord,
    Dracula,
    Paper,
    Carbon,
}

impl ThemeId {
    pub const COUNT: usize = 12;

    pub const ALL: [ThemeId; ThemeId::COUNT] = [
        ThemeId::DarkViolet,
        ThemeId::Midnight,
        ThemeId::Graphite,
        ThemeId::Light,
        ThemeId::Emerald,
        ThemeId::Rose,
        ThemeId::Ocean,
        ThemeId::Sunset,
        ThemeId::Nord,
        ThemeId::Dracula,
        ThemeId::Paper,
        ThemeId::Carbon,
    ];

    pub fn all() -> &'static [ThemeId] {
        &Self::ALL
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(index: usize) -> Option<ThemeId> {
        Self::ALL.get(index).copied()
    }

    pub fn name(self) -> &'static str {
        THEMES[self.index()].name
    }

    pub fn light(self) -> bool {
        THEMES[self.index()].light
    }
}

#[derive(Clone, Copy, Debug)]
struct ThemeDef {
    name: &'static str,
    light: bool,
    accent: Rgba,
    bg: Rgba,
    text: Rgba,
    text_dim: Rgba,
    title_tint: f32,
}

const THEMES: [ThemeDef; ThemeId::COUNT] = [
    ThemeDef {
        name: "Dark Violet",
        light: false,
        accent: Rgba::rgb(0.424, 0.388, 1.000),
        bg: Rgba::rgb(0.095, 0.095, 0.115),
        text: Rgba::rgb(0.920, 0.920, 0.940),
        text_dim: Rgba::rgb(0.480, 0.480, 0.540),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Midnight",
        light: false,
        accent: Rgba::rgb(0.260, 0.780, 0.870),
        bg: Rgba::rgb(0.070, 0.085, 0.105),
        text: Rgba::rgb(0.880, 0.910, 0.940),
        text_dim: Rgba::rgb(0.430, 0.470, 0.520),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Graphite",
        light: false,
        accent: Rgba::rgb(0.960, 0.640, 0.240),
        bg: Rgba::rgb(0.110, 0.110, 0.110),
        text: Rgba::rgb(0.900, 0.890, 0.870),
        text_dim: Rgba::rgb(0.470, 0.460, 0.445),
        title_tint: 0.06,
    },
    ThemeDef {
        name: "Light",
        light: true,
        accent: Rgba::rgb(0.180, 0.420, 0.880),
        bg: Rgba::rgb(0.940, 0.942, 0.948),
        text: Rgba::rgb(0.100, 0.105, 0.115),
        text_dim: Rgba::rgb(0.550, 0.560, 0.580),
        title_tint: 0.03,
    },
    ThemeDef {
        name: "Emerald",
        light: false,
        accent: Rgba::rgb(0.160, 0.780, 0.470),
        bg: Rgba::rgb(0.075, 0.105, 0.090),
        text: Rgba::rgb(0.880, 0.930, 0.900),
        text_dim: Rgba::rgb(0.450, 0.520, 0.480),
        title_tint: 0.06,
    },
    ThemeDef {
        name: "Rose",
        light: false,
        accent: Rgba::rgb(0.940, 0.350, 0.500),
        bg: Rgba::rgb(0.105, 0.085, 0.095),
        text: Rgba::rgb(0.930, 0.890, 0.910),
        text_dim: Rgba::rgb(0.520, 0.460, 0.490),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Ocean",
        light: false,
        accent: Rgba::rgb(0.300, 0.520, 0.960),
        bg: Rgba::rgb(0.080, 0.090, 0.115),
        text: Rgba::rgb(0.880, 0.900, 0.940),
        text_dim: Rgba::rgb(0.450, 0.480, 0.540),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Sunset",
        light: false,
        accent: Rgba::rgb(0.980, 0.480, 0.280),
        bg: Rgba::rgb(0.105, 0.085, 0.075),
        text: Rgba::rgb(0.940, 0.900, 0.870),
        text_dim: Rgba::rgb(0.530, 0.470, 0.440),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Nord",
        light: false,
        accent: Rgba::rgb(0.460, 0.620, 0.780),
        bg: Rgba::rgb(0.180, 0.204, 0.251),
        text: Rgba::rgb(0.847, 0.871, 0.914),
        text_dim: Rgba::rgb(0.480, 0.510, 0.560),
        title_tint: 0.05,
    },
    ThemeDef {
        name: "Dracula",
        light: false,
        accent: Rgba::rgb(0.745, 0.580, 0.986),
        bg: Rgba::rgb(0.133, 0.129, 0.173),
        text: Rgba::rgb(0.968, 0.968, 0.952),
        text_dim: Rgba::rgb(0.550, 0.530, 0.600),
        title_tint: 0.08,
    },
    ThemeDef {
        name: "Paper",
        light: true,
        accent: Rgba::rgb(0.720, 0.450, 0.200),
        bg: Rgba::rgb(0.965, 0.955, 0.935),
        text: Rgba::rgb(0.150, 0.130, 0.110),
        text_dim: Rgba::rgb(0.550, 0.520, 0.470),
        title_tint: 0.03,
    },
    ThemeDef {
        name: "Carbon",
        light: false,
        accent: Rgba::rgb(0.900, 0.920, 0.960),
        bg: Rgba::rgb(0.045, 0.045, 0.048),
        text: Rgba::rgb(0.920, 0.930, 0.950),
        text_dim: Rgba::rgb(0.440, 0.450, 0.470),
        title_tint: 0.05,
    },
];

/// Every value the C++ namespace exposed as an accessor.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_active: Rgba,
    pub accent_muted: Rgba,
    pub accent_foreground: Rgba,
    pub bg: Rgba,
    pub text: Rgba,
    pub text_dim: Rgba,
    pub title_bg: Rgba,
    pub title_tint: f32,
    pub toolbar_bg: Rgba,
    pub console_info: Rgba,
    pub console_warn: Rgba,
    pub console_error: Rgba,
    pub console_critical: Rgba,
    pub light: bool,
}

/// Where the persisted selection lives, beside the asset root. The C++ wrote
/// it next to the process's working directory.
pub const DEFAULT_PATH: &str = "assets/themes/editor_theme.ini";

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Theme {
    pub selected: ThemeId,
    pub custom_enabled: bool,
    pub custom_accent: Rgba,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            selected: ThemeId::DarkViolet,
            custom_enabled: false,
            custom_accent: Rgba::rgb(0.424, 0.388, 1.000),
        }
    }
}

impl Theme {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(&self) -> &'static str {
        self.selected.name()
    }

    pub fn palette(&self) -> Palette {
        let def = &THEMES[self.selected.index()];
        let light = def.light;
        let accent = if self.custom_enabled {
            self.custom_accent
        } else {
            def.accent
        };
        let mut toolbar_bg = mix(
            shade(def.bg, if light { -0.045 } else { 0.010 }),
            accent,
            def.title_tint,
        );
        if self.custom_enabled {
            toolbar_bg = mix(toolbar_bg, accent, 0.35);
        }

        let console = console_palette(light);

        Palette {
            accent,
            accent_hover: accent.with_alpha(0.85),
            accent_active: accent.with_alpha(0.70),
            accent_muted: accent.with_alpha(0.15),
            accent_foreground: if accent.luminance() > 0.55 {
                Rgba::rgb(0.080, 0.080, 0.090)
            } else {
                Rgba::rgb(0.960, 0.960, 0.960)
            },
            bg: def.bg,
            text: def.text,
            text_dim: def.text_dim,
            title_bg: shade(def.bg, if light { -0.060 } else { -0.015 }),
            title_tint: def.title_tint,
            toolbar_bg,
            console_info: console.0,
            console_warn: console.1,
            console_error: console.2,
            console_critical: console.3,
            light,
        }
    }

    /// Missing file is not an error: every editor starts with no settings.
    pub fn load(path: &Path) -> Result<Theme, ThemeError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Theme::new()),
            Err(e) => {
                return Err(ThemeError {
                    message: format!("{}: {e}", path.display()),
                })
            }
        };
        Theme::parse(&text)
    }

    /// The persisted file is three white-space-separated groups: the theme
    /// index, the custom-accent flag, then r g b.
    pub fn parse(text: &str) -> Result<Theme, ThemeError> {
        let mut fields = text.split_whitespace();
        let need = |what: &str| ThemeError {
            message: format!("editor_theme.ini: missing {what}"),
        };
        let bad = |what: &str, token: &str| ThemeError {
            message: format!("editor_theme.ini: {what} is not a number: {token:?}"),
        };

        let index_token = fields.next().ok_or_else(|| need("theme index"))?;
        let index: usize = index_token
            .parse()
            .map_err(|_| bad("theme index", index_token))?;
        let selected = ThemeId::from_index(index).ok_or_else(|| ThemeError {
            message: format!("editor_theme.ini: theme index {index} out of range"),
        })?;

        let enabled_token = fields.next().ok_or_else(|| need("custom accent flag"))?;
        let enabled: i32 = enabled_token
            .parse()
            .map_err(|_| bad("custom accent flag", enabled_token))?;

        let mut channel = || -> Result<f32, ThemeError> {
            let token = fields.next().ok_or_else(|| need("accent channel"))?;
            token.parse().map_err(|_| bad("accent channel", token))
        };
        let r = channel()?;
        let g = channel()?;
        let b = channel()?;

        Ok(Theme {
            selected,
            custom_enabled: enabled != 0,
            custom_accent: Rgba::rgb(r, g, b),
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = format!(
            "{}\n{}\n{} {} {}\n",
            self.selected.index(),
            self.custom_enabled as i32,
            self.custom_accent.r,
            self.custom_accent.g,
            self.custom_accent.b,
        );
        std::fs::write(path, text)
    }

    pub fn set_selected(&mut self, id: ThemeId) {
        self.selected = id;
    }

    pub fn set_custom_accent_enabled(&mut self, enabled: bool) {
        self.custom_enabled = enabled;
    }

    pub fn set_custom_accent(&mut self, rgb: Rgba) {
        self.custom_enabled = true;
        self.custom_accent = Rgba::new(rgb.r, rgb.g, rgb.b, 1.0);
    }
}

fn console_palette(light: bool) -> (Rgba, Rgba, Rgba, Rgba) {
    if light {
        (
            Rgba::rgb(0.120, 0.130, 0.150),
            Rgba::rgb(0.760, 0.550, 0.000),
            Rgba::rgb(0.800, 0.150, 0.150),
            Rgba::rgb(0.640, 0.130, 0.700),
        )
    } else {
        (
            Rgba::rgb(0.920, 0.920, 0.940),
            Rgba::rgb(1.000, 0.750, 0.250),
            Rgba::rgb(0.960, 0.360, 0.360),
            Rgba::rgb(0.920, 0.470, 0.960),
        )
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct ThemeError {
    pub message: String,
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ThemeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_has_a_name_and_a_palette() {
        for (i, id) in ThemeId::all().iter().enumerate() {
            assert_eq!(id.index(), i);
            assert!(!id.name().is_empty());
            let palette = Theme::new().palette();
            assert!(palette.accent.a > 0.0);
        }
    }

    #[test]
    fn custom_accent_overrides_only_the_accent() {
        let mut theme = Theme::new();
        let plain = theme.palette();
        theme.set_custom_accent(Rgba::rgb(0.0, 1.0, 0.0));
        let custom = theme.palette();
        assert_eq!(custom.accent, Rgba::rgb(0.0, 1.0, 0.0));
        assert_eq!(custom.bg, plain.bg);
        assert_eq!(custom.text, plain.text);
        assert_ne!(custom.toolbar_bg, plain.toolbar_bg);
    }

    #[test]
    fn accent_foreground_contrasts() {
        let mut theme = Theme::new();
        theme.set_selected(ThemeId::Carbon);
        let palette = theme.palette();
        assert!(palette.accent_foreground.luminance() < 0.2);

        theme.set_selected(ThemeId::DarkViolet);
        let palette = theme.palette();
        assert!(palette.accent_foreground.luminance() > 0.5);
    }

    #[test]
    fn round_trips_through_text() {
        let theme = Theme {
            selected: ThemeId::Nord,
            custom_enabled: true,
            custom_accent: Rgba::rgb(0.25, 0.5, 0.75),
        };
        let text = format!(
            "{}\n{}\n{} {} {}\n",
            theme.selected.index(),
            theme.custom_enabled as i32,
            theme.custom_accent.r,
            theme.custom_accent.g,
            theme.custom_accent.b,
        );
        let parsed = Theme::parse(&text).unwrap();
        assert_eq!(parsed.selected, ThemeId::Nord);
        assert!(parsed.custom_enabled);
        assert!((parsed.custom_accent.g - 0.5).abs() < 1e-6);
    }

    #[test]
    fn malformed_text_is_an_error() {
        assert!(Theme::parse("").is_err());
        assert!(Theme::parse("hello").is_err());
        assert!(Theme::parse("0 0 0.1 0.2").is_err());
        assert!(Theme::parse("99 0 0.1 0.2 0.3").is_err());
        assert!(Theme::parse("0 0 0.1 0.2 nope").is_err());
    }

    #[test]
    fn missing_file_is_the_defaults() {
        let path = std::env::temp_dir().join("mirror_theme_unit_absent/editor_theme.ini");
        assert_eq!(Theme::load(&path).unwrap(), Theme::new());
    }
}
