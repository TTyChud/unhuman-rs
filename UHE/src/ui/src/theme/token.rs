//! Semantic colour tokens: the vocabulary.
//!
//! ## Why a token and not a colour
//!
//! The C++ has two theme systems. `EditorTheme` is the good one -- 12 themes,
//! semantic slots, a Rec.709 check for the accent's foreground -- but it is an
//! *editor-only override* of an engine palette, so a build without the editor
//! silently gets the other one. Two systems that disagree is not two systems, it
//! is a bug with a config file.
//!
//! So the tokens are here, in the library, and there is exactly one of them. A
//! widget asks for [`Token::HeaderHover`], never for `(0.24, 0.31, 0.62)`. The
//! moment a number appears in a widget the theme has stopped working: that
//! widget will be unreadable in one of the twelve themes and nobody will find
//! out which until somebody opens it.
//!
//! ## Why tokens are opaque
//!
//! Every token is the colour that actually reaches the screen, already
//! composited. The C++ wrote `ImGuiCol_TextSelectedBg = Accent(0.35)` and left
//! the compositing to whatever happened to be underneath, which means the colour
//! on screen depends on the panel it is in -- and means there is no way to ask
//! whether the text on it is readable, because the answer is a function of the
//! background rather than a property of the token.
//!
//! [`Token::Selection`] is therefore `mix(surface, accent, 0.35)`, computed.
//! [`super::contrast::pushed_until_all`] exists to compute the ones that cannot
//! be computed by mixing.
//!
//! Translucency is still available, by [`Rgba::with_alpha`], at the site that
//! composites. That is the honest place for it: a shadow over an unknown OS
//! backdrop genuinely has no single resulting colour, and no contrast test can
//! be written for a colour that does not exist yet.

use std::ops::{Index, IndexMut};

use super::Rgba;

/// A semantic colour slot.
///
/// Named for what the colour *is for*, never for what it looks like. There is no
/// `Token::Bluish`; there is [`Token::Link`] and [`Token::StatusError`],
/// because those are the two things a person needs to know about a colour to use
/// it correctly, and a name is the only documentation that travels with a pixel.
///
/// The list is the union of what ImGui's style array carried in the C++ and what
/// the editor drew by hand on top of it. The hand-drawn ones -- the three gizmo
/// axes, the viewport backdrop -- are in here for the same reason the rest are:
/// they were `const ImVec4 kAxisX` at the top of a panel file, which is a theme
/// system with a one-panel scope.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
#[non_exhaustive]
pub enum Token {
    // ---- surfaces, back to front -------------------------------------
    /// The window behind everything.
    SurfaceWindow,
    /// A panel sitting on the window. A hair above it.
    SurfacePanel,
    /// A text field, a slider rail, a combo box. The surface a label sits on
    /// most often, so it is the one every text-bearing token is checked against.
    SurfaceInput,
    /// The same field under the pointer.
    SurfaceInputHover,
    /// A menu, a tooltip, a context menu. Floats above the window, so it goes
    /// *toward the text* rather than away from the window.
    SurfacePopup,
    /// An inactive title bar, and the dimmed rest of the tree behind a modal.
    SurfaceInactive,
    /// A focused title bar, and the floating toolbar. Carries the accent as a
    /// tint, which is how a window says it has focus without a glow.
    SurfaceTitleActive,
    /// The area behind the 3D viewport when it is not focused.
    ViewportBackdrop,

    // ---- lines --------------------------------------------------------
    /// A decorative hairline: a separator between rows, the border of a panel.
    Border,
    /// Every control outline and the focus ring. Held to the WCAG 1.4.11
    /// non-text floor, because these are the borders that mean "there is a
    /// control here".
    BorderStrong,
    /// A rule between groups of rows.
    Divider,
    /// The scrollbar grab at rest.
    Scrollbar,
    /// The scrollbar grab under the pointer.
    ScrollbarHover,
    /// The scrollbar grab while dragging.
    ScrollbarActive,

    // ---- text ---------------------------------------------------------
    /// A label. The only ink in the theme.
    TextPrimary,
    /// A label for something the user cannot do right now.
    TextDim,
    /// The ink for a label drawn on an accent fill. The same colour as
    /// [`Token::TextPrimary`], kept as its own slot because the rule that
    /// produces it is a different rule; see [`super::derive`].
    TextOnAccent,
    /// A hyperlink, or anything else that navigates.
    Link,

    // ---- accent -------------------------------------------------------
    /// The accent as decoration: a checkmark, a slider grab, a focus ring, the
    /// outline of a drop target. Never carries a label -- see
    /// [`Token::AccentFill`].
    Accent,
    /// The accent as a *fill*: a pressed button, an active row. Derived so the
    /// theme's label is legible on it.
    AccentFill,
    /// A button at rest.
    Button,
    /// A button under the pointer.
    ButtonHover,
    /// A button being pressed. The same colour as [`Token::AccentFill`],
    /// because a pressed button is a filled accent.
    ButtonActive,

    // ---- rows and selection -------------------------------------------
    /// A selectable tree or list row at rest.
    Header,
    /// A selectable row under the pointer.
    HeaderHover,
    /// A selectable row being pressed.
    HeaderActive,
    /// The background of a selected row.
    Selection,
    /// The background of selected text inside a text field.
    SelectionInactive,

    // ---- status -------------------------------------------------------
    /// An informational log line.
    StatusInfo,
    /// A log line worth reading.
    StatusWarn,
    /// A log line naming a failure.
    StatusError,
    /// A log line naming a failure that is not recoverable.
    StatusCritical,
    /// A log line naming a success.
    StatusOk,

    // ---- gizmo --------------------------------------------------------
    /// The X axis at rest.
    AxisX,
    /// The Y axis at rest.
    AxisY,
    /// The Z axis at rest.
    AxisZ,
    /// The X axis under the pointer.
    AxisXHot,
    /// The Y axis under the pointer.
    AxisYHot,
    /// The Z axis under the pointer.
    AxisZHot,
}

impl Token {
    /// Every token, in declaration order.
    ///
    /// The legibility table in [`super::legibility`] is walked by the test, so a
    /// token with no checked pair is a gap to notice rather than a silent pass.
    pub const ALL: [Token; 39] = [
        Token::SurfaceWindow,
        Token::SurfacePanel,
        Token::SurfaceInput,
        Token::SurfaceInputHover,
        Token::SurfacePopup,
        Token::SurfaceInactive,
        Token::SurfaceTitleActive,
        Token::ViewportBackdrop,
        Token::Border,
        Token::BorderStrong,
        Token::Divider,
        Token::Scrollbar,
        Token::ScrollbarHover,
        Token::ScrollbarActive,
        Token::TextPrimary,
        Token::TextDim,
        Token::TextOnAccent,
        Token::Link,
        Token::Accent,
        Token::AccentFill,
        Token::Button,
        Token::ButtonHover,
        Token::ButtonActive,
        Token::Header,
        Token::HeaderHover,
        Token::HeaderActive,
        Token::Selection,
        Token::SelectionInactive,
        Token::StatusInfo,
        Token::StatusWarn,
        Token::StatusError,
        Token::StatusCritical,
        Token::StatusOk,
        Token::AxisX,
        Token::AxisY,
        Token::AxisZ,
        Token::AxisXHot,
        Token::AxisYHot,
        Token::AxisZHot,
    ];

    /// How many tokens there are.
    pub const COUNT: usize = Token::ALL.len();

    /// Whether this token is a surface -- something a foreground can be drawn
    /// on.
    ///
    /// A predicate rather than a list of the surface tokens in two places,
    /// because the legibility table's whole argument is that "a foreground lands
    /// on a surface", and a table that could fall out of step with this one would
    /// make that argument decorative.
    pub const fn is_surface(self) -> bool {
        matches!(
            self,
            Token::SurfaceWindow
                | Token::SurfacePanel
                | Token::SurfaceInput
                | Token::SurfaceInputHover
                | Token::SurfacePopup
                | Token::SurfaceTitleActive
                | Token::ViewportBackdrop
        )
    }

    /// The token's name.
    ///
    /// `Debug` on a fieldless enum already prints the variant name; this exists
    /// so a caller asking for a name gets a name, and so the *settings file*
    /// format has something stable to write that is not a derived `Debug` string.
    pub const fn name(self) -> &'static str {
        // A `match` with no wildcard, so adding a variant is a compile error
        // rather than a token that silently prints something else.
        match self {
            Token::SurfaceWindow => "SurfaceWindow",
            Token::SurfacePanel => "SurfacePanel",
            Token::SurfaceInput => "SurfaceInput",
            Token::SurfaceInputHover => "SurfaceInputHover",
            Token::SurfacePopup => "SurfacePopup",
            Token::SurfaceInactive => "SurfaceInactive",
            Token::SurfaceTitleActive => "SurfaceTitleActive",
            Token::ViewportBackdrop => "ViewportBackdrop",
            Token::Border => "Border",
            Token::BorderStrong => "BorderStrong",
            Token::Divider => "Divider",
            Token::Scrollbar => "Scrollbar",
            Token::ScrollbarHover => "ScrollbarHover",
            Token::ScrollbarActive => "ScrollbarActive",
            Token::TextPrimary => "TextPrimary",
            Token::TextDim => "TextDim",
            Token::TextOnAccent => "TextOnAccent",
            Token::Link => "Link",
            Token::Accent => "Accent",
            Token::AccentFill => "AccentFill",
            Token::Button => "Button",
            Token::ButtonHover => "ButtonHover",
            Token::ButtonActive => "ButtonActive",
            Token::Header => "Header",
            Token::HeaderHover => "HeaderHover",
            Token::HeaderActive => "HeaderActive",
            Token::Selection => "Selection",
            Token::SelectionInactive => "SelectionInactive",
            Token::StatusInfo => "StatusInfo",
            Token::StatusWarn => "StatusWarn",
            Token::StatusError => "StatusError",
            Token::StatusCritical => "StatusCritical",
            Token::StatusOk => "StatusOk",
            Token::AxisX => "AxisX",
            Token::AxisY => "AxisY",
            Token::AxisZ => "AxisZ",
            Token::AxisXHot => "AxisXHot",
            Token::AxisYHot => "AxisYHot",
            Token::AxisZHot => "AxisZHot",
        }
    }

    /// A token's name, or `None`.
    ///
    /// For a settings file written by a newer build than this one: an unknown
    /// token is a token to skip, not a reason to refuse to start.
    pub fn from_name(name: &str) -> Option<Token> {
        Token::ALL.into_iter().find(|token| token.name() == name)
    }

    /// The slot index, used to index a [`TokenSet`].
    ///
    /// `repr(u8)` plus `as` is safe here because [`TokenSet`] is sized from
    /// [`Token::ALL`], so no variant's value can exceed the array.
    const fn slot(self) -> usize {
        self as usize
    }
}

/// A complete set of token colours.
#[derive(Clone, PartialEq, Debug)]
pub struct TokenSet {
    colours: [Rgba; Token::COUNT],
}

impl TokenSet {
    /// An empty set, every slot transparent.
    ///
    /// A derived set never has a hole in it. This exists so a missing token
    /// shows as "nothing drawn" rather than as opaque black over the UI, and so
    /// a test can prove one.
    pub fn empty() -> Self {
        Self {
            colours: [Rgba::transparent(); Token::COUNT],
        }
    }

    /// The colour bound to a token. Transparent for a token nothing bound.
    pub fn colour(&self, token: Token) -> Rgba {
        self.colours[token.slot()]
    }

    /// Builder form of the index assignment, for a derivation that reads as a
    /// table.
    pub fn with(mut self, token: Token, colour: Rgba) -> Self {
        self[token] = colour;
        self
    }

    /// Every `(token, colour)` in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Token, Rgba)> + '_ {
        Token::ALL.into_iter().zip(self.colours.iter().copied())
    }

    /// Whether two sets agree on every token.
    ///
    /// So a test can say "these two themes differ" without caring *where*, and
    /// so the custom-accent path has something to assert on besides a value.
    pub fn same_as(&self, other: &TokenSet) -> bool {
        self == other
    }
}

impl Default for TokenSet {
    fn default() -> Self {
        Self::empty()
    }
}

impl Index<Token> for TokenSet {
    type Output = Rgba;

    fn index(&self, token: Token) -> &Rgba {
        &self.colours[token.slot()]
    }
}

impl IndexMut<Token> for TokenSet {
    fn index_mut(&mut self, token: Token) -> &mut Rgba {
        &mut self.colours[token.slot()]
    }
}
