//! The one function that turns a [`ThemeDef`] into a [`Theme`].
//!
//! ## One rule, one sign
//!
//! The C++ wrote its shades as `light ? 0.050f : 0.045f` about a dozen times,
//! and in four of them got the sign wrong. `MenuBarBg` used
//! `raise(light ? -0.035f : 0.012f)`, where `raise` *already* multiplies by the
//! scheme -- so the light case raised a light theme's surfaces toward white and
//! flattened the menu bar into the window behind it. A token somebody forgot to
//! branch on is a token that is wrong on one scheme, and twelve hand-written
//! branches cannot be reviewed for that.
//!
//! So there is one function, [`lift`], and one branch -- the sign of the amount:
//!
//! ```text
//! lift(+a)   away from the window, toward the text
//! lift(-a)   away from the window, back toward the far side
//! ```
//!
//! On a dark scheme that is white and black; on a light scheme, black and white.
//! Same line of code. Every surface is an amount on that line, and the amounts
//! are ImGui's own numbers, which were tuned.
//!
//! ## The four colours that cannot be placed by taste
//!
//! The C++ placed all four by taste, and each is a real failure in the C++
//! palette that `tests/theme_contrast.rs` now pins:
//!
//! - **Accent fill.** `ButtonActive` was the accent at alpha 0.85, and the ink
//!   for it was chosen by thresholding a mis-computed luminance. On Dark Violet
//!   the resulting label-on-button is 3.43:1 and on Light it is 3.53:1, both
//!   below AA for body text. [`Token::AccentFill`] is the accent pushed towards
//!   the window until the theme's label is legible on it.
//! - **Link.** The accent, used as text: 4.10:1 on Dark Violet's window.
//! - **Dimmed text.** The authored `TextDim` is 2.57:1 on Light's input surface
//!   and 2.86:1 on its window -- invisible next to the 4.5:1 the same theme's
//!   primary text manages.
//! - **Status and axis colours.** The C++ hardcoded a console palette per
//!   scheme (`ComputeConsolePalette`), four numbers with nothing to do with the
//!   theme they are shown in. They are now hue-matched to the theme's text
//!   luminance and then pushed for legibility, so they cannot drift from it.
//!
//! The three row states get the same treatment for the same reason:
//! `HeaderActive` was `Accent(0.55)`, a saturated colour carrying a label, and
//! it is the identical failure one shade later -- 3.11:1 on Carbon.
//!
//! ## Why the ink on an accent fill is the theme's own label
//!
//! The C++ needed a separate `AccentForeground` because it painted labels on a
//! colour it had not derived: the fill was the accent *at an alpha*, so the ink
//! was chosen for one colour and the label ended up on another. Once the fill is
//! derived, there is nothing left for a second ink to solve -- and a label in a
//! different colour from every other label is a second palette, which is the
//! exact disease this crate exists to remove. So [`Token::TextOnAccent`] is
//! [`Token::TextPrimary`], kept as its own slot because it is the result of a
//! different rule and a theme is allowed to diverge from it later.
//!
//! ## Aiming past the bar
//!
//! Every push aims at [`TEXT_TARGET`] / [`NON_TEXT_TARGET`] rather than at the
//! WCAG floor itself. A colour that lands exactly on 4.5:1 makes the suite
//! green or red depending on the last bit of an `f32`, which is not a property
//! anybody wants a palette to have.

use super::contrast::{pushed_until_all, recolour_at_luminance, relative_luminance};
use super::legibility::{NON_TEXT_TARGET, TEXT_TARGET};
use super::{FontRoles, Metrics, Rgba, Scheme, Theme, ThemeDef, Token, TokenSet};

/// The four hues the palette needs meanings for.
///
/// Chromaticities, not colours: [`recolour_at_luminance`] turns each into
/// whatever the theme's text luminance needs, which is what keeps a red meaning
/// "error" in a dark theme and in a light one. The values are the C++'s console
/// palette, so the hues are unchanged.
const HUE_WARN: Rgba = Rgba::rgb(1.00, 0.75, 0.25);
const HUE_ERROR: Rgba = Rgba::rgb(0.96, 0.36, 0.36);
const HUE_CRITICAL: Rgba = Rgba::rgb(0.92, 0.47, 0.96);
const HUE_OK: Rgba = Rgba::rgb(0.45, 0.85, 0.50);

/// The gizmo's three axes.
///
/// The C++ declared these as `const ImVec4 kAxisX` at the top of
/// `SceneHierachyPanel.cpp`, which is a theme system with a one-panel scope: the
/// viewport gizmo could not have used them without including the hierarchy
/// panel's header, and a theme change could not have reached them at all.
const HUE_AXIS_X: Rgba = Rgba::rgb(0.90, 0.22, 0.22);
const HUE_AXIS_Y: Rgba = Rgba::rgb(0.30, 0.85, 0.25);
const HUE_AXIS_Z: Rgba = Rgba::rgb(0.25, 0.45, 0.95);

/// One step away from `base`, in the direction that separates it from the window.
///
/// Positive is toward the text and negative is away from it. On a dark scheme
/// that means white and black; on a light scheme, black and white. One function,
/// one branch, and no per-token `light ?` anywhere else in the crate.
fn lift(scheme: Scheme, base: Rgba, amount: f32) -> Rgba {
    match scheme {
        Scheme::Dark => base.mix(Rgba::rgb(1.0, 1.0, 1.0), amount),
        Scheme::Light => base.mix(Rgba::rgb(0.0, 0.0, 0.0), amount),
    }
}

impl ThemeDef {
    /// Derive every token from this definition.
    pub fn derive(&self) -> Theme {
        let (scheme, bg, accent, text) = (self.scheme, self.background, self.accent, self.text);

        // Surfaces first, because every later colour is measured against them.
        let surface_window = bg;
        let surface_panel = lift(scheme, bg, 0.007);
        let surface_input = lift(scheme, bg, 0.048);
        let surface_input_hover = lift(scheme, bg, 0.082);
        let surface_popup = lift(scheme, bg, 0.025);
        let surface_inactive = lift(scheme, bg, -0.030);
        let surface_title_active = surface_inactive.mix(accent, self.title_tint);
        let viewport_backdrop = bg.mix(text, 0.08);

        // Everything a label can land on. Read once, because the legibility rule
        // takes "where can this be drawn" as a list rather than as one guess.
        let surfaces = [
            surface_window,
            surface_panel,
            surface_popup,
            surface_input,
            surface_title_active,
        ];

        // A colour with a meaning rather than a job: keep its chromaticity, take
        // its brightness from the theme's text, then push it until it is
        // legible. Two steps, because either alone is not enough -- brightness
        // alone cannot fix a saturated hue that clips, and a push alone washes
        // the hue out entirely.
        let meaningful = |hue: Rgba, against: &[Rgba], minimum: f32| {
            let matched = recolour_at_luminance(hue, relative_luminance(text));
            pushed_until_all(against, matched, text, minimum)
        };

        let accent_fill = pushed_until_all(&[text], accent, surface_window, TEXT_TARGET);
        let link = pushed_until_all(&surfaces, accent, text, TEXT_TARGET);
        let border_strong =
            pushed_until_all(&surfaces, lift(scheme, bg, 0.140), text, NON_TEXT_TARGET);
        let text_dim = pushed_until_all(&surfaces, self.text_dim, text, NON_TEXT_TARGET);

        let status_pair = [surface_window, surface_panel];
        let axis_pair = [surface_window];

        let axis_x = meaningful(HUE_AXIS_X, &axis_pair, NON_TEXT_TARGET);
        let axis_y = meaningful(HUE_AXIS_Y, &axis_pair, NON_TEXT_TARGET);
        let axis_z = meaningful(HUE_AXIS_Z, &axis_pair, NON_TEXT_TARGET);

        let colours = TokenSet::empty()
            .with(Token::SurfaceWindow, surface_window)
            .with(Token::SurfacePanel, surface_panel)
            .with(Token::SurfaceInput, surface_input)
            .with(Token::SurfaceInputHover, surface_input_hover)
            .with(Token::SurfacePopup, surface_popup)
            .with(Token::SurfaceInactive, surface_inactive)
            .with(Token::SurfaceTitleActive, surface_title_active)
            .with(Token::ViewportBackdrop, viewport_backdrop)
            .with(Token::Border, bg.mix(lift(scheme, bg, 0.105), 0.70))
            .with(Token::BorderStrong, border_strong)
            .with(Token::Divider, bg.mix(lift(scheme, bg, 0.110), 0.70))
            .with(Token::Scrollbar, lift(scheme, bg, 0.150))
            .with(Token::ScrollbarHover, lift(scheme, bg, 0.215))
            .with(Token::ScrollbarActive, accent)
            .with(Token::TextPrimary, text)
            .with(Token::TextDim, text_dim)
            .with(Token::TextOnAccent, text)
            .with(Token::Link, link)
            .with(Token::Accent, accent)
            .with(Token::AccentFill, accent_fill)
            .with(Token::Button, lift(scheme, bg, 0.080))
            .with(
                Token::ButtonHover,
                lift(scheme, bg, 0.080).mix(accent, 0.20),
            )
            .with(Token::ButtonActive, accent_fill)
            .with(Token::Header, bg.mix(accent, 0.10))
            .with(Token::HeaderHover, bg.mix(accent_fill, 0.35))
            .with(Token::HeaderActive, bg.mix(accent_fill, 0.55))
            .with(Token::Selection, bg.mix(accent_fill, 0.35))
            .with(Token::SelectionInactive, bg.mix(accent_fill, 0.20))
            .with(Token::StatusInfo, text)
            .with(
                Token::StatusWarn,
                meaningful(HUE_WARN, &status_pair, TEXT_TARGET),
            )
            .with(
                Token::StatusError,
                meaningful(HUE_ERROR, &status_pair, TEXT_TARGET),
            )
            .with(
                Token::StatusCritical,
                meaningful(HUE_CRITICAL, &status_pair, TEXT_TARGET),
            )
            .with(
                Token::StatusOk,
                meaningful(HUE_OK, &status_pair, TEXT_TARGET),
            )
            .with(Token::AxisX, axis_x)
            .with(Token::AxisY, axis_y)
            .with(Token::AxisZ, axis_z)
            .with(Token::AxisXHot, lift(scheme, axis_x, 0.30))
            .with(Token::AxisYHot, lift(scheme, axis_y, 0.30))
            .with(Token::AxisZHot, lift(scheme, axis_z, 0.30));

        Theme {
            def: *self,
            scheme,
            colours,
            metrics: Metrics::default(),
            fonts: FontRoles::default(),
        }
    }

    /// This theme with a different accent, fully derived.
    ///
    /// The point of the accent being an *input* rather than something the
    /// derivation discovers half way through: an override is one argument, and
    /// because it still goes through every legibility rule, a user cannot pick an
    /// accent that produces unreadable buttons. The C++'s version of this
    /// (`ApplyAccentOverride`) re-wrote twenty style slots after the fact, so it
    /// could not do that.
    pub fn derive_with_accent(&self, accent: Rgba) -> Theme {
        self.with_accent(accent).derive()
    }
}
