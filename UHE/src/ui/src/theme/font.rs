//! Named font roles, because `Fonts[1]` is not a font.
//!
//! ## The bug being fixed
//!
//! `SceneHierachyPanel.cpp:405`:
//!
//! ```cpp
//! ImFont *boldFont = io.Fonts->Fonts.Size > 1 ? io.Fonts->Fonts[1] : nullptr;
//! ```
//!
//! Index 1 means "whatever the second font that happened to be pushed is". It
//! is bold in the build that pushed Inter SemiBold second and regular in the
//! build that pushed it third, it is null in a build with one font, and the
//! null case is guarded *here* and nowhere else -- every other `PushFont` in
//! that file has to remember. Four component drawers, four chances to get it
//! wrong, and the symptom is a section header that is not bold rather than an
//! error.
//!
//! ## What a role is
//!
//! A [`FontRole`] is a name. [`RoleFont`] is what the name resolves to: a
//! family, a size and a weight. A widget asks for [`FontRole::Strong`] and gets
//! the same font on every platform and in every build, and the resolution is
//! one table here rather than a convention spread across panel files.
//!
//! ## Weights are variation coordinates, not extra font files
//!
//! [`RoleFont::weight`] is a value on the `wght` axis of a variable font -- the
//! C++'s `Inter-VariableFont_opsz,wght.ttf`, which is in the editor's assets and
//! which nothing was using as a variable font. So a role is one face with a
//! coordinate rather than four faces in an atlas, which is what `PushFont` was
//! reaching for.
//!
//! There is a second axis in that file, `opsz`, and it is left alone
//! deliberately: optical sizing changes glyph shapes, so a "12pt" role that
//! silently changed shape between themes would make two themes look like two
//! programs.
//!
//! A role deliberately carries no colour. A font that knows what colour it is
//! has become a style, and a style is a second place to name a colour that does
//! not go through [`super::Token`].

/// Which face a role uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontKind {
    /// Text. The UI's default, and the only kind that appears in a label.
    Sans,
    /// Fixed-width, for numbers being dragged and for anything the user reads
    /// as code. A `DragFloat` is in this role, because a number that changes
    /// width as it changes value is impossible to read at a glance.
    Mono,
    /// The icon font, if the build installed one. Kept as a role so a widget
    /// can ask for it by name; with nothing installed it falls back to
    /// [`FontKind::Sans`] rather than to index 0 of an empty atlas.
    Icon,
}

/// What a piece of text is *for*, which is what determines its font.
///
/// A role is chosen by meaning. `Body` is a label, `Heading` is a section
/// header, `Strong` is the emphasised part of a line, `Value` is a number
/// under the pointer, `Code` is a path or an identifier. Choosing by size
/// instead -- `Font 14` -- is how a UI ends up with four roles that are all
/// 14pt and only one of which is the right choice in any given place.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum FontRole {
    /// A label. The size everything else is measured against.
    Body,
    /// A section header, a panel title.
    Heading,
    /// An emphasised run inside a line: the file name in a tree row.
    Strong,
    /// Secondary text: units, counts, a hint next to a field.
    Caption,
    /// A number being edited, or a column of numbers that must line up.
    Value,
    /// A path, a type name, anything with punctuation a reader will parse.
    Code,
    /// A toolbar glyph.
    Icon,
}

impl FontRole {
    /// Every role, in increasing order of visual weight.
    pub const ALL: [FontRole; 7] = [
        FontRole::Caption,
        FontRole::Body,
        FontRole::Strong,
        FontRole::Heading,
        FontRole::Value,
        FontRole::Code,
        FontRole::Icon,
    ];

    /// The role's name, which is also its stable key in a settings file.
    ///
    /// `Debug` would do, except that the derived name is `Strong` in one place
    /// and `FontRole::Strong` in another depending on how it was formatted.
    pub const fn name(self) -> &'static str {
        match self {
            FontRole::Body => "body",
            FontRole::Heading => "heading",
            FontRole::Strong => "strong",
            FontRole::Caption => "caption",
            FontRole::Value => "value",
            FontRole::Code => "code",
            FontRole::Icon => "icon",
        }
    }

    /// The role a name refers to, or `None`.
    pub fn from_name(name: &str) -> Option<FontRole> {
        FontRole::ALL.into_iter().find(|role| role.name() == name)
    }
}

/// A role resolved to a face, a size and a weight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RoleFont {
    /// Which face.
    pub family: FontKind,
    /// Logical pixels.
    ///
    /// A ratio rather than a point size, because the whole table scales with
    /// the UI zoom and twelve hard-coded sizes would be twelve places to forget
    /// that.
    pub size: f32,
    /// The `wght` coordinate, 1-1000 as the OpenType axis defines it. `400` is
    /// regular, `700` is bold.
    pub weight: u16,
}

impl RoleFont {
    /// A proportional role.
    pub const fn sans(size: f32, weight: u16) -> Self {
        Self {
            family: FontKind::Sans,
            size,
            weight,
        }
    }

    /// A monospaced role.
    pub const fn mono(size: f32, weight: u16) -> Self {
        Self {
            family: FontKind::Mono,
            size,
            weight,
        }
    }
}

/// Every role's font, at one UI scale.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FontRoles {
    /// Multiplier applied to every role's size. The UI zoom.
    pub scale: f32,
}

impl Default for FontRoles {
    /// Inter's own scale, which is what the C++ ran with: 14pt body.
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

impl FontRoles {
    /// The font for a role at the current scale.
    ///
    /// A `match`, not a table, because the numbers belong next to the names that
    /// explain them and a table would have to be kept in step with the enum --
    /// the same duplication that put `Fonts[1]` in a panel file.
    pub fn get(&self, role: FontRole) -> RoleFont {
        // The one place the weight convention is written down: 400 is regular,
        // 600 is the UI's "emphasised", 700 is a header. Not 500, because Inter's
        // 500 is visually indistinguishable from 400 at these sizes and costs a
        // glyph outline set for nothing.
        let font = match role {
            FontRole::Body => RoleFont::sans(14.0, 400),
            FontRole::Heading => RoleFont::sans(16.0, 700),
            FontRole::Strong => RoleFont::sans(14.0, 600),
            FontRole::Caption => RoleFont::sans(12.0, 400),
            FontRole::Value => RoleFont::mono(13.0, 500),
            FontRole::Code => RoleFont::mono(13.0, 400),
            FontRole::Icon => RoleFont::sans(16.0, 400),
        };

        RoleFont {
            size: font.size * self.scale,
            ..font
        }
    }

    /// The roles, for a font installer to walk.
    pub fn iter(&self) -> impl Iterator<Item = (FontRole, RoleFont)> + '_ {
        FontRole::ALL.into_iter().map(|role| (role, self.get(role)))
    }
}
