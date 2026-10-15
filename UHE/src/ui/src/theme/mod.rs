//! `mirror`'s theme: semantic colour tokens, sizes, and named font roles.
//!
//! ## Why the tokens live in the library
//!
//! The C++ has two theme systems. `EditorTheme` is the good one -- twelve
//! themes, semantic slots, a Rec.709 check for the accent's foreground -- but it
//! is an *editor-only override* of an engine palette, so a build without the
//! editor silently gets the other one. Two systems that disagree is not two
//! systems, it is a bug with a config file, and which of the two won was decided
//! by the engine's hardcoded one.
//!
//! So the tokens are here, in the library, and there is exactly one of them.
//! `mirror` is the library; the editor is a consumer of it like any other.
//!
//! ## What is in here
//!
//! | file | concern |
//! |---|---|
//! | [`token`] | the vocabulary: [`Token`], and a [`TokenSet`] of colours. |
//! | [`palette`] | the twelve authored themes, as data. |
//! | [`derive`] | the one function that turns a palette entry into a theme. |
//! | [`contrast`] | colour science. Whether two colours can be told apart. |
//! | [`legibility`] | what this UI requires of that, as a checkable table. |
//! | [`font`] | named font roles, so no code says `Fonts[1]`. |
//!
//! [`token`], [`palette`] and [`contrast`] know nothing about any toolkit. That
//! is what lets `tests/theme_contrast.rs` check all twelve themes against the
//! legibility table with no window, no GPU and no display server -- which is
//! the only reason the check can run in CI at all, and therefore the only reason
//! it can be relied on to still hold.
//!
//! ## Opaque, pre-composited, one derivation
//!
//! Three rules, each of which is a bug in the C++ fixed:
//!
//! 1. **A token is the colour that reaches the screen.** Not a colour and an
//!    alpha to be composited by whoever draws it, because then the colour on
//!    screen is a function of the panel rather than a property of the token, and
//!    nothing can be proved about it.
//! 2. **Light and dark differ by one sign.** Every surface is
//!    `lift(scheme, base, amount)` where a positive amount means "toward the
//!    text". The C++ branched per token and got four signs wrong.
//! 3. **A colour that carries text is derived, not chosen.** Four of them could
//!    not be placed by taste -- the accent fill, the link, the dimmed text and
//!    the status colours -- and the C++ placed all four by taste, failing WCAG
//!    AA on six of its twelve themes. See [`derive`].

mod contrast;
mod derive;
mod font;
mod legibility;
mod palette;
mod token;

pub use contrast::{
    contrast_ratio, delinearize, linearize, pushed_until_all, recolour_at_luminance,
    relative_luminance,
};
pub use font::{FontKind, FontRole, FontRoles, RoleFont};
pub use legibility::{checked_pairs, first_failure, Pair, NON_TEXT_CONTRAST, TEXT_CONTRAST};
pub use palette::{
    accent_foreground, hue_at_luminance, theme_at, theme_named, ThemeDef, ALL_THEMES, DEFAULT_THEME,
};
pub use token::{Token, TokenSet};

/// An sRGB colour with straight alpha.
///
/// Channels are **encoded** sRGB, not linear, because that is what a UI toolkit
/// and every texture format in the engine exchange. The one place linear light
/// is required -- deciding whether two colours can be told apart -- is
/// [`contrast`], and it converts on the way in.
///
/// Shading and mixing happen in the encoded space, which is what ImGui does and
/// what a designer expects: `mix(white, black, 0.5)` gives mid grey, and the
/// perceptual midpoint of white and black is darker than that.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgba {
    /// Red, encoded sRGB.
    pub r: f32,
    /// Green, encoded sRGB.
    pub g: f32,
    /// Blue, encoded sRGB.
    pub b: f32,
    /// Alpha. Always `1.0` for a [`Token`] value; see the module docs.
    pub a: f32,
}

impl Rgba {
    /// An opaque colour.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Fully transparent. The value of a slot in a [`TokenSet`] that has not
    /// been derived yet, so a missing token shows as "nothing drawn" rather than
    /// as opaque black over the UI.
    pub const fn transparent() -> Self {
        Self {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }
    }

    /// The same colour at a different alpha.
    ///
    /// For compositing a known colour over a surface the theme already knows (a
    /// tooltip at 90%), prefer deriving a token, which is contrast-testable.
    /// This is for the cases that genuinely are not: a drop shadow over whatever
    /// the OS put behind the window.
    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self { a: alpha, ..self }
    }

    /// Linear interpolation in the encoded space, `t = 0` returning `self`.
    pub fn mix(self, other: Self, t: f32) -> Self {
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// `0xRRGGBB`, for a settings file or a test failure message.
    pub fn to_hex(self) -> String {
        let channel = |v: f32| format!("{:02x}", (v.clamp(0.0, 1.0) * 255.0).round() as u32);
        format!("#{}{}{}", channel(self.r), channel(self.g), channel(self.b))
    }
}

/// Whether a theme is light or dark, and the one thing the derivation branches
/// on.
///
/// There is exactly one branch in the whole of [`derive`], and it is this
/// comparison. The C++ branched per token -- `light ? 0.050f : 0.045f` appears
/// about a dozen times -- which is why its two schemes drifted: a token somebody
/// forgot to branch on is a token that is wrong on one scheme, and there is no
/// way to review twelve hand-written branches for that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    /// Light text on a dark background.
    Dark,
    /// Dark text on a light background.
    Light,
}

impl Scheme {
    /// Whether this is [`Scheme::Dark`].
    pub const fn is_dark(self) -> bool {
        matches!(self, Scheme::Dark)
    }

    /// The other one.
    pub const fn flipped(self) -> Self {
        match self {
            Scheme::Dark => Scheme::Light,
            Scheme::Light => Scheme::Dark,
        }
    }
}

/// How round things are and how big the gaps are.
///
/// The C++'s `ApplyMetrics`, which was identical for all twelve themes: there is
/// no reason for a theme to change the size of a checkbox, so the numbers live
/// here rather than in each theme.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Metrics {
    /// Corner radius of windows and panels.
    pub window_rounding: f32,
    /// Corner radius of text fields, buttons and sliders.
    pub frame_rounding: f32,
    /// Corner radius of the scrollbar grab.
    pub scrollbar_rounding: f32,
    /// Corner radius of a tab.
    pub tab_rounding: f32,
    /// Border width of a window.
    pub window_border: f32,
    /// Border width of a text field.
    pub frame_border: f32,
    /// Padding inside a text field or button.
    pub frame_padding: (f32, f32),
    /// Padding inside a window.
    pub window_padding: f32,
    /// Gap between two widgets.
    pub item_spacing: (f32, f32),
    /// Gap between a widget and the label inside it.
    pub inner_spacing: (f32, f32),
    /// Width of one indent level.
    pub indent_width: f32,
    /// Width of the scrollbar.
    pub scrollbar_width: f32,
    /// Smallest a slider grab can be, in pixels.
    pub grab_min_size: f32,
    /// Width of the separator between two docked panels.
    pub dock_separator: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            window_rounding: 4.0,
            frame_rounding: 4.0,
            scrollbar_rounding: 9.0,
            tab_rounding: 5.0,
            window_border: 1.0,
            frame_border: 0.0,
            frame_padding: (7.0, 4.0),
            window_padding: 8.0,
            item_spacing: (8.0, 5.0),
            inner_spacing: (6.0, 5.0),
            indent_width: 20.0,
            scrollbar_width: 13.0,
            grab_min_size: 9.0,
            dock_separator: 2.0,
        }
    }
}

/// A finished theme: colours, metrics and font roles.
#[derive(Clone, PartialEq, Debug)]
pub struct Theme {
    /// The authored definition this was derived from.
    pub def: ThemeDef,
    /// The light or dark branch.
    pub scheme: Scheme,
    /// Every semantic slot.
    pub colours: TokenSet,
    /// Sizes and rounding.
    pub metrics: Metrics,
    /// Named font roles.
    pub fonts: FontRoles,
}

impl Theme {
    /// The colour bound to a token.
    pub fn colour(&self, token: Token) -> Rgba {
        self.colours[token]
    }

    /// The font for a role.
    pub fn font(&self, role: FontRole) -> RoleFont {
        self.fonts.get(role)
    }

    /// The first of the legibility failures in this theme, or `None`.
    ///
    /// In the library and not only in its test suite, so a tool that generates a
    /// theme can ask the same question `tests/theme_contrast.rs` asks.
    pub fn first_legibility_failure(&self) -> Option<String> {
        first_failure(&self.colours)
    }
}
