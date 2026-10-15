//! Colour science: whether two colours can be told apart.
//!
//! ## The Rec.709 function, and what the C++ got wrong about it
//!
//! `EditorTheme.cpp:371` computes the accent's foreground from
//!
//! ```text
//! lum = 0.2126 * r + 0.7152 * g + 0.0722 * b
//! ```
//!
//! on **encoded** sRGB channels, and calls the result relative luminance. It is
//! not. Relative luminance is defined over *linear* light, which is what
//! [`linearize`] produces; the weights are right and the input is not. The error
//! is not subtle -- 0.0393 becomes 0.0031, a factor of twelve -- and it is
//! largest exactly where it matters, in the midtones. A threshold on the wrong
//! function is not a slightly wrong answer, it is a coin flip.
//!
//! Two things follow from that, and both are in this file.
//!
//! **Compare ratios, never threshold a dot product.** [`contrast_ratio`] is the
//! real measure of "can these two be told apart". It needs no threshold, so it
//! cannot be wrong about where the boundary is, and it is symmetric, so it does
//! not matter which of the two colours is brighter. [`pushed_until_all`] bisects
//! on it, which is why no colour in [`super::derive`] is placed by comparing a
//! luminance to a magic number.
//!
//! **The floors belong to a standard, not to us.** WCAG 2.1 AA: 4.5:1 for body
//! text, 3:1 for a control boundary. They live in [`super::legibility`] so the
//! derivation and the test that checks it cannot disagree about what "readable"
//! means, and the derivation aims slightly *past* the floor -- see
//! [`super::derive`] -- so a colour sitting exactly on the bar is not what
//! decides whether the suite is green.

use super::Rgba;

/// The sRGB electro-optical transfer function, encoded to linear.
///
/// The piecewise breakpoint is the one sRGB specifies (`0.04045`), not the
/// `0.03928` in older texts; at 8-bit precision the difference is under one code
/// value either way, and matching the standard costs nothing.
pub fn linearize(channel: f32) -> f32 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse of [`linearize`].
pub fn delinearize(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// Rec.709 relative luminance: `0.2126 R + 0.7152 G + 0.0722 B`, in linear light.
///
/// Alpha is ignored, because a token is opaque and a translucent colour has no
/// luminance until it is composited over something. Composite first, then ask.
pub fn relative_luminance(colour: Rgba) -> f32 {
    0.2126 * linearize(colour.r) + 0.7152 * linearize(colour.g) + 0.0722 * linearize(colour.b)
}

/// The WCAG 2.1 contrast ratio between two opaque colours.
///
/// Always at least `1.0`, and symmetric, so it can be compared against a
/// constant without knowing which of the two is brighter.
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// How long [`pushed_until_all`] bisects.
///
/// Twenty halvings of `1.0` is one part in a million, which is below the
/// precision of an 8-bit channel and far below the precision that matters. The
/// loop is bounded rather than convergent on purpose: it must not be able to
/// spin, because it runs while a theme is being built and a theme is built on
/// the frame the user opened the appearance panel.
const BISECT_STEPS: u32 = 20;

/// The colour nearest `candidate` on the segment towards `toward` that clears
/// `minimum` against **every** background in `against`.
///
/// ## Why one function and not one per token
///
/// Four colours in the palette cannot be placed by taste: the accent fill, the
/// link, the dimmed text and the status colours are all "as saturated as this
/// theme allows, and no less". Expressing that four times would produce four
/// slightly different rules and the guarantee would be gone. Expressing it once
/// makes it a property of the library, and `tests/theme_contrast.rs` becomes a
/// statement about the crate rather than about twelve hand-picked tables.
///
/// ## Why the backgrounds are a list
///
/// A colour's readability depends on what is behind it, and in this UI that
/// varies: a link in a sidebar sits on [`super::Token::SurfacePanel`], which is
/// a hair off [`super::Token::SurfaceWindow`]. Tuning against one background is
/// how a palette ends up with a link that is fine in a window and too faint in a
/// panel. Taking the largest push that satisfies all of them means a colour is
/// legible everywhere the theme allows it to be drawn.
///
/// ## Why bisection
///
/// Contrast is monotone along a straight line between two fixed colours -- the
/// candidate's luminance moves steadily towards `toward`'s -- so the set of
/// usable positions is an interval and the nearest valid one is a bisection. And
/// if even `toward` is not far enough the answer is `toward`, not a panic: a
/// theme that cannot be made legible is still a theme, and refusing to build it
/// would leave the user with a black screen and no explanation.
pub fn pushed_until_all(against: &[Rgba], candidate: Rgba, toward: Rgba, minimum: f32) -> Rgba {
    let mut push = 0.0f32;

    for background in against {
        if contrast_ratio(candidate, *background) >= minimum {
            continue;
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..BISECT_STEPS {
            let mid = (lo + hi) * 0.5;
            if contrast_ratio(candidate.mix(toward, mid), *background) >= minimum {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        // `hi` is the side that passed, so the result is on the legal side of the
        // bar by construction rather than by rounding luck.
        push = push.max(hi);
    }

    candidate.mix(toward, push)
}

/// The colour with the chromaticity of `hue` and the relative luminance of
/// `target`.
///
/// For the status and axis colours, where what the colour *means* is the point
/// -- red means error -- and not how bright it happens to be. Scaling all three
/// linear channels by one factor keeps the hue exactly and moves only the
/// brightness, which is the whole reason to do this in linear space at all.
///
/// Clamping is unavoidable: a saturated hue has a bounded gamut, so a request
/// for a luminance outside it cannot be met. Clamping loses the exact luminance
/// and keeps the hue, which is the half that carries meaning. The caller then
/// hands the result to [`pushed_until_all`] for the final legibility check, so a
/// clamp can cost saturation but can never leave the text unreadable.
pub fn recolour_at_luminance(hue: Rgba, target: f32) -> Rgba {
    let hue_luminance = relative_luminance(hue);
    if hue_luminance <= f32::EPSILON {
        // Black, or close enough: there is no hue to preserve, so give the
        // caller grey rather than a division by a number about to produce
        // infinities.
        let grey = delinearize((target / 3.0).clamp(0.0, 1.0));
        return Rgba::rgb(grey, grey, grey);
    }

    let scale = target / hue_luminance;
    Rgba::rgb(
        delinearize((linearize(hue.r) * scale).clamp(0.0, 1.0)),
        delinearize((linearize(hue.g) * scale).clamp(0.0, 1.0)),
        delinearize((linearize(hue.b) * scale).clamp(0.0, 1.0)),
    )
}
