//! What this UI requires of a colour, as a table a test can read.
//!
//! ## Why requirements are data
//!
//! [`super::contrast`] can tell you whether two colours are distinguishable. It
//! cannot tell you whether any particular pair matters. "Is this link colour
//! readable" is only a question once you have said which colour is the ink and
//! which is the paper, and the list of the pairs that occur is a fact about the
//! UI rather than about colour theory.
//!
//! So the pairs are a table. That has three consequences worth having:
//!
//! - **The check is total.** Every theme is checked against every pair, so a new
//!   theme cannot ship with an unreadable state; only a new *pair* can go
//!   unchecked, and adding one is a deliberate act in a 40-line table.
//! - **The failure message can say why.** Each row carries the WCAG clause it
//!   comes from, so a failure reports a rule rather than two numbers and a
//!   guess.
//! - **The floors are named once.** [`TEXT_CONTRAST`] and
//!   [`NON_TEXT_CONTRAST`] are used by both the table and
//!   [`super::derive`], so the derivation and its test cannot disagree about
//!   what "readable" means -- which is the whole way this kind of check usually
//!   gets defeated, by the test asserting 4.5 while the code aims for 3.

use super::{contrast_ratio, Rgba, Token, TokenSet};

/// 4.5:1. The WCAG 2.1 AA floor for body text (clause 1.4.3).
pub const TEXT_CONTRAST: f32 = 4.5;

/// 3:1. The WCAG 2.1 AA floor for a control boundary or a focus ring (1.4.11).
pub const NON_TEXT_CONTRAST: f32 = 3.0;

/// What the palette *aims* for, so a derived colour is never exactly on the bar.
///
/// A test asserting `>= 4.5` on a value the derivation produced to be exactly
/// `4.5` is a test that passes or fails on the last bit of an `f32`. The gap is
/// invisible to a person and large enough to be reliable.
pub const TEXT_TARGET: f32 = 4.6;

/// [`NON_TEXT_CONTRAST`] with the same margin.
pub const NON_TEXT_TARGET: f32 = 3.1;

/// One foreground-on-background requirement.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pair {
    /// What is being drawn.
    pub foreground: Token,
    /// What it is drawn on.
    pub background: Token,
    /// The floor for this kind of pair.
    pub minimum: f32,
    /// The clause the floor comes from, carried so a failure message can name a
    /// rule instead of printing two numbers.
    pub why: &'static str,
}

/// A body-text pair.
const fn body(foreground: Token, background: Token) -> Pair {
    Pair {
        foreground,
        background,
        minimum: TEXT_CONTRAST,
        why: "WCAG 2.1 AA 1.4.3, body text",
    }
}

/// A pair checked against the non-text floor.
///
/// Also used for disabled text, because WCAG 1.4.3 explicitly exempts inactive
/// components; 3:1 is then the honest bar for "you can tell it is there".
const fn ui(foreground: Token, background: Token) -> Pair {
    Pair {
        foreground,
        background,
        minimum: NON_TEXT_CONTRAST,
        why: "WCAG 2.1 AA 1.4.11, non-text boundary",
    }
}

/// The pairs the test and [`first_failure`] check.
///
/// A table rather than a rule, because "every token against every surface" is
/// 43 x 7 and most of those never occur -- a label is never drawn in
/// [`Token::AxisX`]. Listing the pairs that occur keeps it short enough to read,
/// and a token added later has to be *added here* to be checked.
const PAIRS: &[Pair] = &[
    // Body text on every surface it can land on.
    body(Token::TextPrimary, Token::SurfaceWindow),
    body(Token::TextPrimary, Token::SurfacePanel),
    body(Token::TextPrimary, Token::SurfacePopup),
    body(Token::TextPrimary, Token::SurfaceInput),
    body(Token::TextPrimary, Token::SurfaceTitleActive),
    // ... and on the faces a label is drawn over.
    body(Token::TextPrimary, Token::Button),
    body(Token::TextPrimary, Token::ButtonHover),
    body(Token::TextPrimary, Token::Header),
    body(Token::TextPrimary, Token::HeaderHover),
    body(Token::TextPrimary, Token::HeaderActive),
    body(Token::TextPrimary, Token::Selection),
    // The one place the ink has to work against a colour that is neither a
    // surface nor a neutral: a label on an accent fill.
    body(Token::TextOnAccent, Token::AccentFill),
    body(Token::TextOnAccent, Token::ButtonActive),
    // A link is text.
    body(Token::Link, Token::SurfaceWindow),
    body(Token::Link, Token::SurfacePanel),
    body(Token::Link, Token::SurfacePopup),
    body(Token::Link, Token::SurfaceInput),
    body(Token::Link, Token::SurfaceTitleActive),
    // Disabled and secondary text.
    ui(Token::TextDim, Token::SurfaceWindow),
    ui(Token::TextDim, Token::SurfacePanel),
    ui(Token::TextDim, Token::SurfacePopup),
    ui(Token::TextDim, Token::SurfaceInput),
    // Console lines, in a window and in a panel.
    body(Token::StatusInfo, Token::SurfaceWindow),
    body(Token::StatusInfo, Token::SurfacePanel),
    body(Token::StatusWarn, Token::SurfaceWindow),
    body(Token::StatusWarn, Token::SurfacePanel),
    body(Token::StatusError, Token::SurfaceWindow),
    body(Token::StatusError, Token::SurfacePanel),
    body(Token::StatusCritical, Token::SurfaceWindow),
    body(Token::StatusCritical, Token::SurfacePanel),
    body(Token::StatusOk, Token::SurfaceWindow),
    body(Token::StatusOk, Token::SurfacePanel),
    // Meaningful boundaries: the outline of every control and the focus ring.
    ui(Token::BorderStrong, Token::SurfaceWindow),
    ui(Token::BorderStrong, Token::SurfaceInput),
    // The gizmo axes. The only coloured thing drawn over a 3D viewport, so they
    // have to be legible against the window behind it as well.
    ui(Token::AxisX, Token::SurfaceWindow),
    ui(Token::AxisY, Token::SurfaceWindow),
    ui(Token::AxisZ, Token::SurfaceWindow),
];

/// Every foreground-on-background pair the UI actually draws.
pub fn checked_pairs() -> &'static [Pair] {
    PAIRS
}

/// Check one token set against [`checked_pairs`].
///
/// Returns a description of the first failure, or `None`.
pub fn first_failure(colours: &TokenSet) -> Option<String> {
    first_failure_with(colours, PAIRS)
}

/// Check a token set against a specific list of pairs.
///
/// The parameter exists so a test can prove the *checker* works by handing it a
/// pair that must fail -- otherwise a checker that returned `None` always would
/// pass every theme, and a green suite would mean nothing.
pub fn first_failure_with(colours: &TokenSet, pairs: &[Pair]) -> Option<String> {
    for pair in pairs {
        let ratio = contrast_ratio(colours[pair.foreground], colours[pair.background]);
        if ratio < pair.minimum {
            return Some(format!(
                "{} ({}) on {} is {:.2}:1, below the {:.1}:1 floor",
                pair.foreground.name(),
                pair.why,
                pair.background.name(),
                ratio,
                pair.minimum,
            ));
        }
    }
    None
}

/// The measured ratio for one pair.
///
/// For a test that wants to report the worst case in a theme rather than the
/// first failure, which is a much more useful thing to read.
#[allow(dead_code)]
pub fn ratio_of(colours: &TokenSet, pair: &Pair) -> f32 {
    contrast_ratio(colours[pair.foreground], colours[pair.background])
}

/// The pair with the smallest margin in a token set, or `None` for an empty
/// table.
///
/// Margin rather than ratio, because a pair needing only 3:1 and reaching 3.2
/// is fine and a pair needing 4.5 and reaching 4.6 is not: the margin is what
/// says whether the theme has room left.
#[allow(dead_code)]
pub fn tightest(colours: &TokenSet, pairs: &[Pair]) -> Option<(Pair, f32)> {
    pairs
        .iter()
        .map(|pair| (*pair, ratio_of(colours, pair) - pair.minimum))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// An opaque black, for the one place a theme is composited over something it
/// does not know: a drop shadow over the OS backdrop.
///
/// Not a token, because it is not a semantic colour -- it is the absence of
/// light, and every other absence of light in this crate is expressed as a
/// surface plus an amount.
#[allow(dead_code)]
pub const SHADOW: Rgba = Rgba::rgb(0.0, 0.0, 0.0);
