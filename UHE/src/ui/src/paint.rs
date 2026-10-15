//! What a widget draws, as data, and the one interface it draws it through.
//!
//! ## Why this file exists at all
//!
//! `egui-wgpu` is welded to wgpu ^30. There is no ash equivalent, so the
//! Vulkan backend currently cannot host the editor's UI at all -- and the plan
//! is wgpu first, egui tessellation through `bifrost` second. That second step
//! is only a contained change if the widgets do not already depend on a
//! renderer, so this file is the seam that makes it one.
//!
//! The split is three layers:
//!
//! | layer | knows about | file |
//! |---|---|---|
//! | widgets | [`Token`] names, geometry | [`crate::widget`], [`crate::gizmo`] |
//! | this file | geometry and tokens, nothing else | `paint.rs` |
//! | renderer | how to turn it into pixels | [`crate::backend`] |
//!
//! A widget cannot say `painter.rect_filled(...)`, because it does not have a
//! `Painter` -- it has a [`PaintSink`]. It cannot name one either, because the
//! trait is this file's and this file does not name egui's painter. Adding
//! egui-glow, hand-rolling tessellation for the Vulkan backend, or recording the
//! whole frame into a frame graph are then three implementations of
//! [`PaintSink`], and `tests/paint.rs` pins the property that matters: the same
//! widget emits the same calls into any two sinks.
//!
//! ## Coordinates are egui's, on purpose
//!
//! [`Vec2`] and [`Rect`] come from `egui`, but only as *arithmetic types* -- no
//! context, no painter, no style. They are the types egui's own tessellated
//! output is expressed in, so when `bifrost` takes over the draw step the
//! geometry does not have to be converted; it is already in the coordinates the
//! tessellator wants. A `glam::Vec2` here would be more consistent with the
//! rest of the engine and would buy one conversion per vertex.

use egui::{pos2, Align, Rect, Vec2};

use crate::theme::{FontRole, Token};

/// Line width and colour for an outline.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stroke {
    /// Width in logical pixels. Zero means "do not stroke", which is different
    /// from a hairline: a zero-width stroke is not a request for a thin one.
    pub width: f32,
    /// The colour, by token. Never a raw colour, so the shape cannot be built
    /// from a number that no theme controls.
    pub colour: Token,
}

impl Stroke {
    /// A stroke of a given width.
    pub const fn new(width: f32, colour: Token) -> Self {
        Self { width, colour }
    }
}

/// Corner rounding for a [`PaintCmd::Rect`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rounding {
    /// Radius of every corner.
    pub radius: f32,
}

impl Rounding {
    /// No rounding.
    pub const NONE: Rounding = Rounding { radius: 0.0 };

    /// A radius of `radius` pixels.
    pub const fn new(radius: f32) -> Self {
        Self { radius }
    }
}

/// One primitive, as data.
///
/// Deliberately a small closed set. The set is the contract with the renderer, so
/// every new renderer has to implement every variant -- which is the pressure
/// that keeps it small. A variant here is a line of code in each backend, so
/// adding one costs more than it looks.
#[derive(Clone, PartialEq, Debug)]
pub enum PaintCmd {
    /// A filled and/or outlined rectangle.
    Rect {
        /// Where.
        bounds: Rect,
        /// Fill, or `None` for no fill.
        fill: Option<Token>,
        /// Outline, or `None`.
        stroke: Option<Stroke>,
        /// Corner rounding.
        rounding: Rounding,
    },
    /// A line segment.
    Line {
        /// One end.
        from: Vec2,
        /// The other end.
        to: Vec2,
        /// How it is drawn.
        stroke: Stroke,
    },
    /// A circle, filled and/or outlined.
    Circle {
        /// The centre.
        center: Vec2,
        /// Radius in logical pixels.
        radius: f32,
        /// Fill, or `None`.
        fill: Option<Token>,
        /// Outline, or `None`.
        stroke: Option<Stroke>,
    },
    /// A closed or open polyline.
    ///
    /// There is no separate "polyline" and "polygon" variant: whether a shape is
    /// filled is a property of the fill token, and two variants would mean two
    /// branches in every backend for one difference.
    Path {
        /// The vertices, in order.
        points: Vec<Vec2>,
        /// Fill, or `None` for an open path.
        fill: Option<Token>,
        /// Outline, or `None`.
        stroke: Option<Stroke>,
        /// Corner rounding for the stroke joins.
        rounding: Rounding,
    },
    /// Text.
    Text {
        /// The anchor.
        position: Vec2,
        /// The string. Owned, because a widget hands over a `String` and copying
        /// it per frame to save 24 bytes is not a trade worth making in the
        /// layer that is supposed to be cheap.
        text: String,
        /// Which font role.
        role: FontRole,
        /// Which colour.
        colour: Token,
        /// How the anchor relates to the text.
        align: Align,
    },
}

/// Where the drawing goes.
///
/// The trait is the seam, so it is deliberately small and deliberately free of
/// any renderer type: a method takes geometry and a [`Token`] and returns
/// nothing. A backend is left with exactly two jobs, resolve the token to a
/// colour and issue the primitive, and neither of them can be done in the wrong
/// place by accident.
pub trait PaintSink {
    /// A filled and/or outlined rectangle.
    fn rect(
        &mut self,
        bounds: Rect,
        fill: Option<Token>,
        stroke: Option<Stroke>,
        rounding: Rounding,
    );

    /// A line segment.
    fn line(&mut self, from: Vec2, to: Vec2, stroke: Stroke);

    /// A circle.
    fn circle(&mut self, center: Vec2, radius: f32, fill: Option<Token>, stroke: Option<Stroke>);

    /// A polyline or polygon.
    fn path(
        &mut self,
        points: Vec<Vec2>,
        fill: Option<Token>,
        stroke: Option<Stroke>,
        rounding: Rounding,
    );

    /// Text at an anchor.
    fn text(&mut self, position: Vec2, text: String, role: FontRole, colour: Token, align: Align);

    /// The rectangle drawing is currently clipped to, if any.
    ///
    /// A sink that cannot clip reports `None`, and a widget that needs clipping
    /// asks for the intersection instead of assuming. Reporting `None` is honest
    /// where silently clipping to the last rectangle is not.
    fn clip(&self) -> Option<Rect> {
        None
    }
}

/// An ordered list of primitives.
///
/// Ordered because draw order is part of the meaning: a gizmo drawn over an axis
/// and not over its own handle is a different gizmo. Not sorted, not batched --
/// a backend that wants to merge two fills can do that with the list it is
/// given, and a layer that reordered it would be making that decision with less
/// information than the backend has.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PaintList {
    commands: Vec<PaintCmd>,
}

impl PaintList {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// The primitives, in draw order.
    pub fn commands(&self) -> &[PaintCmd] {
        &self.commands
    }

    /// How many primitives.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Append one primitive.
    pub fn push(&mut self, command: PaintCmd) {
        self.commands.push(command);
    }

    /// Discard everything.
    pub fn clear(&mut self) {
        self.commands.clear();
    }

    /// Play the whole list into a sink.
    ///
    /// The only path from a list to a renderer, so it is the only place a sink
    /// can be driven -- and it can be driven from a test with a sink that
    /// records instead of draws.
    pub fn play_into(&self, sink: &mut dyn PaintSink) {
        for command in &self.commands {
            match command {
                PaintCmd::Rect {
                    bounds,
                    fill,
                    stroke,
                    rounding,
                } => sink.rect(*bounds, *fill, *stroke, *rounding),
                PaintCmd::Line { from, to, stroke } => sink.line(*from, *to, *stroke),
                PaintCmd::Circle {
                    center,
                    radius,
                    fill,
                    stroke,
                } => sink.circle(*center, *radius, *fill, *stroke),
                PaintCmd::Path {
                    points,
                    fill,
                    stroke,
                    rounding,
                } => sink.path(points.clone(), *fill, *stroke, *rounding),
                PaintCmd::Text {
                    position,
                    text,
                    role,
                    colour,
                    align,
                } => sink.text(*position, text.clone(), *role, *colour, *align),
            }
        }
    }
}

/// A sink that records instead of drawing.
///
/// Not a test helper: it is how a UI is *measured*. Frame cost of an editor
/// panel, "how many primitives does the gizmo cost", "does this widget draw
/// anything when it is collapsed" -- none of those questions can be answered by
/// a `Painter`, and all of them are answered by counting. A test that asserts on
/// a sink of this type is asserting on the widget, not on a pixel.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RecordingSink {
    /// Everything that was drawn, in order.
    pub commands: Vec<PaintCmd>,
    /// The clip rectangle reported by [`PaintSink::clip`], if any.
    pub clip_rect: Option<Rect>,
}

impl RecordingSink {
    /// A sink with no clip.
    pub fn new() -> Self {
        Self::default()
    }

    /// A sink reporting a clip rectangle.
    pub fn clipped_to(clip_rect: Rect) -> Self {
        Self {
            commands: Vec::new(),
            clip_rect: Some(clip_rect),
        }
    }

    /// A sink that has already played a list.
    pub fn of(list: &PaintList) -> Self {
        let mut sink = Self::new();
        list.play_into(&mut sink);
        sink
    }

    /// How many primitives use a given token.
    ///
    /// The check that catches a widget which grew a hardcoded colour: it cannot,
    /// because [`PaintCmd`] has nowhere to put one, but a widget that draws the
    /// wrong *token* is equally invisible in a screenshot and this names it.
    pub fn count_token(&self, token: Token) -> usize {
        self.commands
            .iter()
            .filter(|command| command.tokens().contains(&token))
            .count()
    }
}

impl PaintSink for RecordingSink {
    fn rect(
        &mut self,
        bounds: Rect,
        fill: Option<Token>,
        stroke: Option<Stroke>,
        rounding: Rounding,
    ) {
        self.commands.push(PaintCmd::Rect {
            bounds,
            fill,
            stroke,
            rounding,
        });
    }

    fn line(&mut self, from: Vec2, to: Vec2, stroke: Stroke) {
        self.commands.push(PaintCmd::Line { from, to, stroke });
    }

    fn circle(&mut self, center: Vec2, radius: f32, fill: Option<Token>, stroke: Option<Stroke>) {
        self.commands.push(PaintCmd::Circle {
            center,
            radius,
            fill,
            stroke,
        });
    }

    fn path(
        &mut self,
        points: Vec<Vec2>,
        fill: Option<Token>,
        stroke: Option<Stroke>,
        rounding: Rounding,
    ) {
        self.commands.push(PaintCmd::Path {
            points,
            fill,
            stroke,
            rounding,
        });
    }

    fn text(&mut self, position: Vec2, text: String, role: FontRole, colour: Token, align: Align) {
        self.commands.push(PaintCmd::Text {
            position,
            text,
            role,
            colour,
            align,
        });
    }

    fn clip(&self) -> Option<Rect> {
        self.clip_rect
    }
}

impl PaintCmd {
    /// Every token this primitive refers to.
    ///
    /// A primitive can refer to two: a fill and a stroke. Returning both rather
    /// than "the fill" is what lets [`RecordingSink::count_token`] answer for
    /// outlines as well.
    pub fn tokens(&self) -> Vec<Token> {
        let mut tokens = Vec::new();
        match self {
            PaintCmd::Rect { fill, stroke, .. } => {
                tokens.extend(*fill);
                tokens.extend(stroke.map(|s| s.colour));
            }
            PaintCmd::Line { stroke, .. } => {
                tokens.push(stroke.colour);
            }
            PaintCmd::Path { stroke, fill, .. } => {
                if let Some(stroke) = stroke {
                    tokens.push(stroke.colour);
                }
                tokens.extend(*fill);
            }
            PaintCmd::Circle { fill, stroke, .. } => {
                tokens.extend(*fill);
                tokens.extend(stroke.map(|s| s.colour));
            }
            PaintCmd::Text { colour, .. } => tokens.push(*colour),
        }
        tokens
    }
}

/// The four corners of a rectangle, clockwise from the top left.
///
/// A helper rather than four calls at the use site, because a rectangle drawn as
/// four separate line segments is a rectangle the renderer cannot merge, and the
/// mistake is invisible until the frame time moves.
pub fn corners(rect: Rect) -> [Vec2; 4] {
    [
        pos2(rect.left(), rect.top()).to_vec2(),
        pos2(rect.right(), rect.top()).to_vec2(),
        pos2(rect.right(), rect.bottom()).to_vec2(),
        pos2(rect.left(), rect.bottom()).to_vec2(),
    ]
}
