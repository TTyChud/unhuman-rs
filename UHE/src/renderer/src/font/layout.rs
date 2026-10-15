//! Text to positioned quads.
//!
//! The output is in the space the C++ `Renderer2D::DrawString` built
//! (`UHE/src/UHE/Renderer/Renderer2D.cpp:453-560`): x advances rightwards, y
//! downwards, one line height is 1.0, and the origin of the first line is
//! (0, 0) with the baseline on it. Nothing here is a second coordinate
//! convention — the plane bounds and the UVs are the ones the atlas already
//! carries, scaled and translated.

use glam::Vec2;

use crate::font::atlas::Atlas;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions {
    /// Extra space after every glyph, in layout units. The C++ `Kerning`
    /// (`Components.h:74`), which is per-glyph tracking rather than a pair
    /// adjustment — the pair table is applied separately, before the glyph.
    pub kerning: f32,
    /// Extra distance between lines, on top of one line height.
    pub line_spacing: f32,
    /// Break lines at this width when set. A width of zero or less is no wrap.
    pub wrap_width: Option<f32>,
    pub align: Align,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            kerning: 0.0,
            line_spacing: 0.0,
            wrap_width: None,
            align: Align::Left,
        }
    }
}

/// One glyph's quad: bottom-left, bottom-right, top-right, top-left, and the
/// matching atlas coordinates. The order is the C++ one, because the index
/// buffer and the winding follow from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextQuad {
    pub positions: [Vec2; 4],
    pub uv: [Vec2; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub codepoint: u32,
    /// Pen position before this glyph's advance. What a caller measures to.
    pub pen_x: f32,
    pub pen_y: f32,
    pub quad: TextQuad,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub placements: Vec<Placement>,
    /// The box the text was laid out in: the wrap width when there is one, and
    /// the longest line otherwise.
    pub width: f32,
    pub height: f32,
}

/// The pen movement one glyph costs, kerning pairs included. `previous` is the
/// codepoint that preceded it on the same line; a line starts with `None` so a
/// pair never spans a break.
fn step(atlas: &Atlas, scale: f32, tracking: f32, previous: Option<u32>, codepoint: u32) -> f32 {
    let pair = match previous {
        Some(previous) => atlas.kerning(previous, codepoint),
        None => 0.0,
    };
    (atlas.glyph_or_fallback(codepoint).advance + pair) * scale + tracking
}

fn line_width(atlas: &Atlas, scale: f32, tracking: f32, line: &[char]) -> f32 {
    let mut width = 0.0;
    let mut previous = None;
    for &c in line {
        width += step(atlas, scale, tracking, previous, c as u32);
        previous = Some(c as u32);
    }
    width
}

/// Break one logical line at the last space that fits, then at the last glyph
/// that fits when a single word is wider than the line.
fn wrap_line(atlas: &Atlas, scale: f32, tracking: f32, max: f32, line: &[char]) -> Vec<Vec<char>> {
    let mut lines = Vec::new();
    let mut rest: Vec<char> = line.to_vec();

    while !rest.is_empty() {
        if line_width(atlas, scale, tracking, &rest) <= max {
            break;
        }

        // The *last* space whose head fits: breaking at a later space that does
        // not fit would leave the line over the width, which is the whole thing
        // wrapping is for. A space with nothing before it is skipped, so a break
        // cannot emit an empty line.
        let space = rest.iter().enumerate().rev().find_map(|(at, &c)| {
            if c == ' '
                && rest[..at].iter().any(|&c| c != ' ')
                && line_width(atlas, scale, tracking, &rest[..at]) <= max
            {
                Some(at)
            } else {
                None
            }
        });

        let (mut head, tail) = match space {
            Some(at) => (
                rest[..at].to_vec(),
                rest[at + 1..]
                    .iter()
                    .copied()
                    .skip_while(|&c| c == ' ')
                    .collect::<Vec<char>>(),
            ),
            None => {
                let mut fit = rest.len() - 1;
                while fit > 1 && line_width(atlas, scale, tracking, &rest[..fit]) > max {
                    fit -= 1;
                }
                (rest[..fit].to_vec(), rest[fit..].to_vec())
            }
        };

        while head.last() == Some(&' ') {
            head.pop();
        }
        if head.is_empty() {
            break;
        }
        lines.push(head);
        rest = tail;
    }

    if lines.is_empty() || !rest.is_empty() {
        lines.push(rest);
    }
    lines
}

pub fn layout(text: &str, atlas: &Atlas, options: &LayoutOptions) -> Layout {
    let scale = atlas.scale();
    let tracking = options.kerning;
    let line_step = 1.0 + options.line_spacing;
    let wrap = options.wrap_width.filter(|&width| width > 0.0);

    let mut lines: Vec<Vec<char>> = Vec::new();
    for logical in text.split('\n') {
        let chars: Vec<char> = logical.chars().collect();
        match wrap {
            Some(max) => lines.extend(wrap_line(atlas, scale, tracking, max, &chars)),
            None => lines.push(chars),
        }
    }

    let widths: Vec<f32> = lines
        .iter()
        .map(|line| line_width(atlas, scale, tracking, line))
        .collect();
    let box_width = wrap.unwrap_or_else(|| widths.iter().copied().fold(0.0, f32::max));

    let mut placements = Vec::new();
    let mut pen_y = 0.0;
    for (line, &width) in lines.iter().zip(widths.iter()) {
        let mut pen_x = match options.align {
            Align::Left => 0.0,
            Align::Center => (box_width - width) * 0.5,
            Align::Right => box_width - width,
        };
        let mut previous = None;
        for &c in line {
            let codepoint = c as u32;
            let glyph = atlas.glyph_or_fallback(codepoint);
            if let Some(previous) = previous {
                pen_x += atlas.kerning(previous, codepoint) * scale;
            }

            let offset = Vec2::new(pen_x, pen_y);
            let min = glyph.plane_bounds_min * scale + offset;
            let max = glyph.plane_bounds_max * scale + offset;
            placements.push(Placement {
                codepoint,
                pen_x,
                pen_y,
                quad: TextQuad {
                    positions: [
                        Vec2::new(min.x, min.y),
                        Vec2::new(max.x, min.y),
                        Vec2::new(max.x, max.y),
                        Vec2::new(min.x, max.y),
                    ],
                    uv: [
                        Vec2::new(glyph.uv_min.x, glyph.uv_max.y),
                        Vec2::new(glyph.uv_max.x, glyph.uv_max.y),
                        Vec2::new(glyph.uv_max.x, glyph.uv_min.y),
                        Vec2::new(glyph.uv_min.x, glyph.uv_min.y),
                    ],
                },
            });

            pen_x += glyph.advance * scale + tracking;
            previous = Some(codepoint);
        }
        pen_y -= line_step;
    }

    Layout {
        placements,
        width: box_width,
        height: lines.len() as f32 * line_step,
    }
}
