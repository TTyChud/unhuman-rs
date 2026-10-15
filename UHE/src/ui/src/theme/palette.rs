//! The twelve authored themes, as data.
//!
//! ## This is a port, and that is the point
//!
//! [`ALL_THEMES`] is the C++'s `kThemes` table (`EditorTheme.cpp:48-61`)
//! transcribed value for value: twelve names, a light flag, an accent, a
//! background, two text colours and a title tint. Those are the numbers a
//! designer chose, and changing them here would throw away the one part of the
//! C++ theme system that was already good.
//!
//! Everything *derived* from them is new, and lives in [`super::derive`]. The
//! split matters: the table is data a person might edit, the derivation is code
//! with a rule in it, and putting them in one file is how you get a bug where
//! somebody "fixed" a theme by editing a formula.
//!
//! ## Five numbers per theme
//!
//! A brand colour, a background, a text colour, a dimmer text colour, and how
//! much brand colour goes into the title bar. Everything else in a [`Theme`]
//! comes out of the derivation, which is what makes twelve themes twelve entries
//! rather than twelve times forty.
//!
//! ## Lookup is forgiving, on purpose
//!
//! [`theme_at`] clamps and [`theme_named`] returns `None`. The index and the
//! name both come from a settings file, and a settings file written by a newer
//! build than this one should not stop the editor from starting with a black
//! screen and no explanation.

use super::contrast::contrast_ratio;
use super::{Rgba, Scheme};

/// One authored theme.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ThemeDef {
    /// Shown in the theme picker.
    pub name: &'static str,
    /// Which branch of the derivation to take.
    pub scheme: Scheme,
    /// The brand colour.
    pub accent: Rgba,
    /// The window background.
    pub background: Rgba,
    /// The label colour.
    pub text: Rgba,
    /// The dimmed label colour, before the legibility push.
    ///
    /// Not guaranteed to survive the derivation: five of the twelve authored
    /// values are below the 3:1 floor on their own theme's input surface, and
    /// [`super::derive`] pulls them towards the text until they are findable.
    pub text_dim: Rgba,
    /// How much accent goes into the focused title bar.
    pub title_tint: f32,
}

impl ThemeDef {
    /// This theme with its accent replaced.
    ///
    /// The C++ needed a second function for this -- `ApplyAccentOverride`, thirty
    /// lines re-writing twenty style slots *after* the first pass had already
    /// written them. Here the accent is an *input* to the derivation rather than
    /// something discovered halfway through it, so an override is the same call
    /// with a different argument. See [`super::derive`] for the rest.
    pub fn with_accent(&self, accent: Rgba) -> ThemeDef {
        ThemeDef { accent, ..*self }
    }

    /// Whether this theme is dark.
    pub fn is_dark(&self) -> bool {
        self.scheme.is_dark()
    }
}

/// The C++'s twelve themes, in its order.
///
/// The order is the order of the combo box in the C++'s appearance panel and the
/// first is that build's default, so a saved index keeps meaning the same thing.
pub const ALL_THEMES: [ThemeDef; 12] = [
    ThemeDef {
        name: "Dark Violet",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.424, 0.388, 1.000),
        background: Rgba::rgb(0.095, 0.095, 0.115),
        text: Rgba::rgb(0.920, 0.920, 0.940),
        text_dim: Rgba::rgb(0.480, 0.480, 0.540),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Midnight",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.260, 0.780, 0.870),
        background: Rgba::rgb(0.070, 0.085, 0.105),
        text: Rgba::rgb(0.880, 0.910, 0.940),
        text_dim: Rgba::rgb(0.430, 0.470, 0.520),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Graphite",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.960, 0.640, 0.240),
        background: Rgba::rgb(0.110, 0.110, 0.110),
        text: Rgba::rgb(0.900, 0.890, 0.870),
        text_dim: Rgba::rgb(0.470, 0.460, 0.445),
        title_tint: 0.06,
    },
    ThemeDef {
        name: "Light",
        scheme: Scheme::Light,
        accent: Rgba::rgb(0.180, 0.420, 0.880),
        background: Rgba::rgb(0.940, 0.942, 0.948),
        text: Rgba::rgb(0.100, 0.105, 0.115),
        text_dim: Rgba::rgb(0.550, 0.560, 0.580),
        title_tint: 0.03,
    },
    ThemeDef {
        name: "Emerald",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.160, 0.780, 0.470),
        background: Rgba::rgb(0.075, 0.105, 0.090),
        text: Rgba::rgb(0.880, 0.930, 0.900),
        text_dim: Rgba::rgb(0.450, 0.520, 0.480),
        title_tint: 0.06,
    },
    ThemeDef {
        name: "Rose",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.940, 0.350, 0.500),
        background: Rgba::rgb(0.105, 0.085, 0.095),
        text: Rgba::rgb(0.930, 0.890, 0.910),
        text_dim: Rgba::rgb(0.520, 0.460, 0.490),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Ocean",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.300, 0.520, 0.960),
        background: Rgba::rgb(0.080, 0.090, 0.115),
        text: Rgba::rgb(0.880, 0.900, 0.940),
        text_dim: Rgba::rgb(0.450, 0.480, 0.540),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Sunset",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.980, 0.480, 0.280),
        background: Rgba::rgb(0.105, 0.085, 0.075),
        text: Rgba::rgb(0.940, 0.900, 0.870),
        text_dim: Rgba::rgb(0.530, 0.470, 0.440),
        title_tint: 0.07,
    },
    ThemeDef {
        name: "Nord",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.460, 0.620, 0.780),
        background: Rgba::rgb(0.180, 0.204, 0.251),
        text: Rgba::rgb(0.847, 0.871, 0.914),
        text_dim: Rgba::rgb(0.480, 0.510, 0.560),
        title_tint: 0.05,
    },
    ThemeDef {
        name: "Dracula",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.745, 0.580, 0.986),
        background: Rgba::rgb(0.133, 0.129, 0.173),
        text: Rgba::rgb(0.968, 0.968, 0.952),
        text_dim: Rgba::rgb(0.550, 0.530, 0.600),
        title_tint: 0.08,
    },
    ThemeDef {
        name: "Paper",
        scheme: Scheme::Light,
        accent: Rgba::rgb(0.720, 0.450, 0.200),
        background: Rgba::rgb(0.965, 0.955, 0.935),
        text: Rgba::rgb(0.150, 0.130, 0.110),
        text_dim: Rgba::rgb(0.550, 0.520, 0.470),
        title_tint: 0.03,
    },
    ThemeDef {
        name: "Carbon",
        scheme: Scheme::Dark,
        accent: Rgba::rgb(0.900, 0.920, 0.960),
        background: Rgba::rgb(0.045, 0.045, 0.048),
        text: Rgba::rgb(0.920, 0.930, 0.950),
        text_dim: Rgba::rgb(0.440, 0.450, 0.470),
        title_tint: 0.05,
    },
];

/// The theme selected when nothing has been chosen: Dark Violet, which is what
/// the C++ defaulted to.
pub const DEFAULT_THEME: ThemeDef = ALL_THEMES[0];

/// Look a theme up by index, clamped.
///
/// Clamped rather than an error: see the module docs.
pub fn theme_at(index: usize) -> &'static ThemeDef {
    &ALL_THEMES[index.min(ALL_THEMES.len() - 1)]
}

/// Look a theme up by name.
pub fn theme_named(name: &str) -> Option<&'static ThemeDef> {
    ALL_THEMES.iter().find(|theme| theme.name == name)
}

/// Near-white. The ink for an accent fill on a dark theme.
pub const INK_LIGHT: Rgba = Rgba::rgb(0.960, 0.960, 0.960);

/// Near-black. The ink for an accent fill on a light theme.
pub const INK_DARK: Rgba = Rgba::rgb(0.080, 0.080, 0.090);

/// The ink for a label drawn on `background`.
///
/// ## What this fixes
///
/// `EditorTheme.cpp:371`:
///
/// ```text
/// lum = 0.2126 * a.x + 0.7152 * a.y + 0.0722 * a.z;   // encoded, not linear
/// return lum > 0.55 ? near-black : near-white;
/// ```
///
/// It thresholded a quantity that is not relative luminance -- see
/// [`super::contrast`] -- which fixed the white-on-white labels the C++ already
/// had on Carbon, a 0.90/0.92/0.96 accent, and left everything else a coin flip.
/// A mid-luminance accent lands near `0.55` either way, and an accent at exactly
/// `0.55` is a different colour from one at `0.5501`.
///
/// Comparing the actual contrast ratio of the two candidate inks has no
/// threshold to get wrong, and it is symmetric, so it does not matter which of
/// the two is brighter.
///
/// ## Why the palette does not use it
///
/// Because the fix is not really *choosing the ink*, it is choosing the fill.
/// [`ThemeDef::derive`] derives [`Token::AccentFill`](super::Token::AccentFill)
/// against the theme's own label, which is why
/// [`Token::TextOnAccent`](super::Token::TextOnAccent) equals
/// [`Token::TextPrimary`](super::Token::TextPrimary) and this function is not on
/// that path any more. It stays public because it is the right answer for the
/// case the derivation cannot cover: a colour from outside the theme, such as an
/// overlay on top of a 3D viewport or a tint chosen by a plugin.
pub fn accent_foreground(background: Rgba) -> Rgba {
    if contrast_ratio(INK_LIGHT, background) >= contrast_ratio(INK_DARK, background) {
        INK_LIGHT
    } else {
        INK_DARK
    }
}

/// The ink for a hue at the luminance of `text`, before the legibility push.
///
/// Exposed because the status colours are the one place a caller may want the hue
/// *without* the push: a graph line, say, where the colour encodes a value and
/// legibility is not the question.
pub fn hue_at_luminance(hue: Rgba, text: Rgba) -> Rgba {
    super::contrast::recolour_at_luminance(hue, super::contrast::relative_luminance(text))
}
