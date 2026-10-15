//! Runtime atlas generation from a TTF.
//!
//! This is the half the C++ did in-process (`UHE/src/UHE/Renderer/Font.cpp:96-190`):
//! load the face, take a glyph set, rasterize, pack, and hand back the same
//! record the JSON parser produces. It exists because `fonts/Inter.ttf` ships
//! with the engine and `Font2D::GetDefault()` was "rasterize the default face at
//! startup", not "load a baked atlas".
//!
//! ## This is a coverage atlas, not an MSDF one
//!
//! `ab_glyph` rasterizes glyph *coverage*, and coverage is not a distance field:
//! it has no median-of-three reconstruction and no screen-space range to widen.
//! It is correct at the size it was rasterized at and soft at any other, where
//! an MSDF atlas stays sharp. That is a real difference and it is left visible
//! rather than papered over: `Atlas::is_msdf` is false here and the shader takes
//! its plain-alpha path (`assets/shaders/text.wgsl`). Runtime MSDF generation is
//! msdfgen's edge colouring and distance computation, which needs a C++ library
//! at build time and is not ported; a pre-baked atlas is the MSDF path.
//!
//! The coordinates are the atlas JSON's, not a second convention: plane bounds in
//! em units with y up from the baseline, UVs normalized and y down from the top
//! of the image, `advance` in em units, `line_height` the font's own. `layout`
//! therefore cannot tell which of the two produced the atlas.

use std::collections::HashMap;

use ab_glyph::{Font, FontRef, GlyphId, ScaleFont};
use glam::Vec2;

use crate::font::atlas::{Atlas, Glyph, Metrics};

/// What `Font2D::GetDefault()` rasterized at (`Font.cpp:23`, `genSizePx = 64`).
/// Smaller here because this is coverage, where the size is the resolution.
pub const DEFAULT_PIXEL_SIZE: u32 = 48;

/// The C++ charset: `{0x0020, 0x00FF}` (`Font.cpp:124`).
pub const DEFAULT_FIRST_CODEPOINT: u32 = 0x20;
pub const DEFAULT_LAST_CODEPOINT: u32 = 0xFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeneratorOptions {
    /// Em size in pixels — the rasterization resolution.
    pub pixel_size: u32,
    pub first_codepoint: u32,
    pub last_codepoint: u32,
    /// The packed atlas is exactly this wide; the height is whatever it takes.
    pub atlas_width: u32,
    pub padding: u32,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            pixel_size: DEFAULT_PIXEL_SIZE,
            first_codepoint: DEFAULT_FIRST_CODEPOINT,
            last_codepoint: DEFAULT_LAST_CODEPOINT,
            atlas_width: 512,
            padding: 1,
        }
    }
}

#[derive(Debug)]
pub enum GenerateError {
    Font(ab_glyph::InvalidFont),
    /// `last_codepoint` before `first_codepoint`, or nothing in the range mapped
    /// to a glyph. An atlas with no glyphs has no fallback and cannot be laid out.
    NoGlyphs,
    /// One glyph is wider than the whole atlas: no packing can help.
    GlyphTooLarge {
        codepoint: u32,
        width: u32,
        atlas_width: u32,
    },
}

impl std::fmt::Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerateError::Font(error) => write!(f, "{error}"),
            GenerateError::NoGlyphs => write!(f, "the font has no glyph in the requested range"),
            GenerateError::GlyphTooLarge {
                codepoint,
                width,
                atlas_width,
            } => write!(
                f,
                "glyph {codepoint:#x} is {width}px wide, atlas is {atlas_width}px"
            ),
        }
    }
}

impl std::error::Error for GenerateError {}

impl From<ab_glyph::InvalidFont> for GenerateError {
    fn from(error: ab_glyph::InvalidFont) -> Self {
        GenerateError::Font(error)
    }
}

struct Raster {
    codepoint: u32,
    glyph_id: GlyphId,
    advance: f32,
    plane_min: Vec2,
    plane_max: Vec2,
    width: u32,
    height: u32,
    coverage: Vec<u8>,
}

pub fn generate(ttf: &[u8], options: &GeneratorOptions) -> Result<Atlas, GenerateError> {
    let font = FontRef::try_from_slice(ttf)?;
    let em = options.pixel_size as f32;
    let scaled = font.as_scaled(em);

    let mut rasters = Vec::new();
    if options.last_codepoint >= options.first_codepoint {
        for codepoint in options.first_codepoint..=options.last_codepoint {
            let Some(character) = char::from_u32(codepoint) else {
                continue;
            };
            let glyph_id = font.glyph_id(character);
            // Glyph 0 is `.notdef`: the character is not in this face. The C++
            // `loadCharset` skipped them and reported how many it loaded, so the
            // atlas's size is the count of characters the font actually has.
            if glyph_id.0 == 0 {
                continue;
            }

            let advance = scaled.h_advance(glyph_id) / em;
            let mut raster = Raster {
                codepoint,
                glyph_id,
                advance,
                plane_min: Vec2::ZERO,
                plane_max: Vec2::ZERO,
                width: 0,
                height: 0,
                coverage: Vec::new(),
            };

            // A whitespace glyph has an advance and no outline.
            if let Some(outlined) = font.outline_glyph(glyph_id.with_scale(em)) {
                let bounds = outlined.px_bounds();
                let width = bounds.width().max(0.0) as u32;
                let height = bounds.height().max(0.0) as u32;
                if width > 0 && height > 0 {
                    let mut coverage = vec![0u8; width as usize * height as usize];
                    outlined.draw(|x, y, value| {
                        let index = y as usize * width as usize + x as usize;
                        if index < coverage.len() {
                            coverage[index] = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    });
                    // `px_bounds` is y-down from the baseline, so the glyph's
                    // bottom is `max.y` and its top is `min.y`; the plane space
                    // is y-up, which is the one sign change in this file.
                    raster.plane_min = Vec2::new(bounds.min.x / em, -bounds.max.y / em);
                    raster.plane_max = Vec2::new(bounds.max.x / em, -bounds.min.y / em);
                    raster.width = width;
                    raster.height = height;
                    raster.coverage = coverage;
                }
            }

            rasters.push(raster);
        }
    }
    if rasters.is_empty() {
        return Err(GenerateError::NoGlyphs);
    }

    let (width, height, placements) = pack(&rasters, options)?;

    let mut rgba8 = vec![0u8; width as usize * height as usize * 4];
    let mut glyphs = Vec::with_capacity(rasters.len());
    for (raster, placement) in rasters.iter().zip(placements.iter()) {
        let Some((left, top)) = *placement else {
            // No bitmap: whitespace, or a glyph with an empty outline.
            glyphs.push(Glyph {
                unicode: raster.codepoint,
                advance: raster.advance,
                plane_bounds_min: raster.plane_min,
                plane_bounds_max: raster.plane_max,
                uv_min: Vec2::ZERO,
                uv_max: Vec2::ZERO,
            });
            continue;
        };

        for row in 0..raster.height {
            for column in 0..raster.width {
                let coverage = raster.coverage[(row * raster.width + column) as usize];
                let texel = ((top + row) as usize * width as usize + (left + column) as usize) * 4;
                // Replicated across the channels: the coverage path samples one
                // channel and a reader that picks another gets the same value.
                rgba8[texel] = coverage;
                rgba8[texel + 1] = coverage;
                rgba8[texel + 2] = coverage;
                rgba8[texel + 3] = coverage;
            }
        }

        let u0 = left as f32 / width as f32;
        let v0 = top as f32 / height as f32;
        let u1 = (left + raster.width) as f32 / width as f32;
        let v1 = (top + raster.height) as f32 / height as f32;
        glyphs.push(Glyph {
            unicode: raster.codepoint,
            advance: raster.advance,
            plane_bounds_min: raster.plane_min,
            plane_bounds_max: raster.plane_max,
            uv_min: Vec2::new(u0, v0),
            uv_max: Vec2::new(u1, v1),
        });
    }

    let mut kerning: HashMap<(u32, u32), f32> = HashMap::new();
    for first in &rasters {
        for second in &rasters {
            // The kern table is sparse, so the zero pairs cost a lookup and no
            // entry: the map ends up the size of the pairs that move anything.
            let pair = scaled.kern(first.glyph_id, second.glyph_id);
            if pair != 0.0 {
                kerning.insert((first.codepoint, second.codepoint), pair / em);
            }
        }
    }

    let metrics = Metrics {
        em_size: 1.0,
        line_height: (scaled.ascent() - scaled.descent() + scaled.line_gap()) / em,
        ascender: scaled.ascent() / em,
        descender: scaled.descent() / em,
        // No distance field, so no range. `Atlas::is_msdf` reads this.
        distance_range: 0.0,
    };

    Ok(Atlas::assemble(
        glyphs, kerning, metrics, width, height, rgba8,
    ))
}

/// Shelf packing: tallest first, left to right, a new row when the current one
/// is full (`TightAtlasPacker`'s job in the C++, `Font.cpp:151-153`). No
/// rotation and no repacking — the C++ used a tight packer, and the difference
/// is atlas area, not correctness.
fn pack(
    rasters: &[Raster],
    options: &GeneratorOptions,
) -> Result<(u32, u32, Vec<Option<(u32, u32)>>), GenerateError> {
    let width = options.atlas_width.max(1);
    let padding = options.padding;
    let mut placements = vec![None; rasters.len()];

    let mut order: Vec<usize> = (0..rasters.len()).filter(|&i| rasters[i].height > 0).collect();
    order.sort_by(|&a, &b| {
        rasters[b]
            .height
            .cmp(&rasters[a].height)
            .then(rasters[a].codepoint.cmp(&rasters[b].codepoint))
    });

    let mut left = padding;
    let mut top = padding;
    let mut shelf_height = 0;
    for index in order {
        let (glyph_width, glyph_height) = (rasters[index].width, rasters[index].height);
        if glyph_width + 2 * padding > width {
            return Err(GenerateError::GlyphTooLarge {
                codepoint: rasters[index].codepoint,
                width: glyph_width,
                atlas_width: width,
            });
        }
        if left + glyph_width + padding > width {
            left = padding;
            top += shelf_height + padding;
            shelf_height = 0;
        }
        placements[index] = Some((left, top));
        left += glyph_width + padding;
        shelf_height = shelf_height.max(glyph_height);
    }

    let used_height = top + shelf_height + padding;
    Ok((width, used_height.max(1).next_power_of_two(), placements))
}
