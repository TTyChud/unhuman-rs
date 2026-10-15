//! The atlas half of a font: metrics, glyph quads and kerning.
//!
//! Two producers write this type and neither owns it. `msdf-atlas-gen` bakes an
//! atlas ahead of time and writes a JSON next to the PNG it packs, which
//! [`parse`](Atlas::parse) reads; [`crate::font::generate`] rasterizes a TTF at
//! startup, which is what the C++ did in-process. Both end at
//! [`assemble`](Atlas::assemble), so `layout` cannot tell which one produced the
//! atlas — the coordinate convention is one, not two.
//!
//! The JSON field names are the ones `msdf-atlas-gen` emits, unchanged, so an
//! atlas the C++ editor produced loads without being re-baked.

use std::collections::HashMap;
use std::fmt;

use glam::Vec2;
use vault::Image;

/// One glyph. The counterpart of `FontGlyph` (`UHE/src/UHE/Renderer/Font.h:16`).
///
/// `plane_bounds_*` is the quad in em units — what the JSON calls `planeBounds`
/// — and `uv_*` is the same quad in the atlas, normalized, y down from the top
/// of the image. The two are not in the same scale: `layout` divides the plane
/// box by one line height, because that is how the C++ normalized a line to 1.0
/// (`Renderer2D.cpp:463`) and a second unit would be a second source of truth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub unicode: u32,
    pub advance: f32,
    pub plane_bounds_min: Vec2,
    pub plane_bounds_max: Vec2,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
}

/// What the glyphs are relative to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub em_size: f32,
    pub line_height: f32,
    pub ascender: f32,
    pub descender: f32,
    /// How far the signed distance field spreads, in pixels. Zero means the
    /// atlas is not a distance field at all — see [`Atlas::is_msdf`].
    pub distance_range: f32,
}

/// A parsed atlas: geometry, kerning, and the decoded image once it is attached.
#[derive(Clone, Debug)]
pub struct Atlas {
    glyphs: Vec<Glyph>,
    index: HashMap<u32, usize>,
    kerning: HashMap<(u32, u32), f32>,
    pub metrics: Metrics,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Debug)]
pub enum AtlasError {
    Json(serde_json::Error),
    /// An atlas with no glyphs has no fallback and cannot be laid out.
    NoGlyphs,
    ImageSize {
        atlas_width: u32,
        atlas_height: u32,
        image_width: u32,
        image_height: u32,
    },
    ImageBytes {
        expected: usize,
        found: usize,
    },
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtlasError::Json(error) => write!(f, "malformed atlas json: {error}"),
            AtlasError::NoGlyphs => write!(f, "atlas has no glyphs"),
            AtlasError::ImageSize {
                atlas_width,
                atlas_height,
                image_width,
                image_height,
            } => write!(
                f,
                "atlas image is {image_width}x{image_height}, json says {atlas_width}x{atlas_height}"
            ),
            AtlasError::ImageBytes { expected, found } => {
                write!(f, "atlas image is {found} bytes, expected {expected}")
            }
        }
    }
}

impl std::error::Error for AtlasError {}

impl From<serde_json::Error> for AtlasError {
    fn from(error: serde_json::Error) -> Self {
        AtlasError::Json(error)
    }
}

impl Atlas {
    /// Parse the geometry. The image is attached separately, because a caller
    /// laying text out to measure it needs no pixels.
    pub fn parse(json: &str) -> Result<Atlas, AtlasError> {
        let raw: RawAtlas = serde_json::from_str(json)?;
        if raw.glyphs.is_empty() {
            return Err(AtlasError::NoGlyphs);
        }

        let width = raw.atlas.width;
        let height = raw.atlas.height;
        let (fw, fh) = (width as f32, height as f32);

        let mut glyphs = Vec::with_capacity(raw.glyphs.len());
        for glyph in &raw.glyphs {
            let plane = glyph.plane_bounds.unwrap_or_default();
            let atlas = glyph.atlas_bounds.unwrap_or_default();
            // A whitespace glyph has bounds in neither space; zero-sized is what
            // `getQuadPlaneBounds` gave the C++ for it, and its advance is still
            // real, so it is kept rather than dropped.
            glyphs.push(Glyph {
                unicode: glyph.unicode,
                advance: glyph.advance,
                plane_bounds_min: Vec2::new(plane.left, plane.bottom),
                plane_bounds_max: Vec2::new(plane.right, plane.top),
                uv_min: Vec2::new(atlas.left / fw, atlas.top / fh),
                uv_max: Vec2::new(atlas.right / fw, atlas.bottom / fh),
            });
        }

        let mut kerning = HashMap::with_capacity(raw.kerning.len());
        for pair in &raw.kerning {
            kerning.insert((pair.unicode1, pair.unicode2), pair.advance);
        }

        Ok(Atlas::assemble(
            glyphs,
            kerning,
            Metrics {
                em_size: raw.metrics.em_size,
                line_height: raw.metrics.line_height,
                ascender: raw.metrics.ascender,
                descender: raw.metrics.descender,
                distance_range: raw.atlas.distance_range.unwrap_or(DEFAULT_DISTANCE_RANGE),
            },
            width,
            height,
            Vec::new(),
        ))
    }

    /// The one way an `Atlas` is built, so the glyph table and its lookup cannot
    /// disagree: two producers and a hand-built struct would be three chances to
    /// forget to index a glyph.
    pub(crate) fn assemble(
        glyphs: Vec<Glyph>,
        kerning: HashMap<(u32, u32), f32>,
        metrics: Metrics,
        width: u32,
        height: u32,
        rgba8: Vec<u8>,
    ) -> Atlas {
        let mut index = HashMap::with_capacity(glyphs.len());
        for (position, glyph) in glyphs.iter().enumerate() {
            index.entry(glyph.unicode).or_insert(position);
        }
        Atlas {
            glyphs,
            index,
            kerning,
            metrics,
            width,
            height,
            rgba8,
        }
    }

    /// Attach the decoded atlas image. Rejects a mismatch rather than sampling
    /// the wrong texels, which looks like a font bug and is not one.
    pub fn set_image(&mut self, image: &Image) -> Result<(), AtlasError> {
        if image.width != self.width || image.height != self.height {
            return Err(AtlasError::ImageSize {
                atlas_width: self.width,
                atlas_height: self.height,
                image_width: image.width,
                image_height: image.height,
            });
        }
        let expected = self.width as usize * self.height as usize * 4;
        if image.rgba8.len() != expected {
            return Err(AtlasError::ImageBytes {
                expected,
                found: image.rgba8.len(),
            });
        }
        self.rgba8 = image.rgba8.clone();
        Ok(())
    }

    pub fn from_json_and_image(json: &str, image: &Image) -> Result<Atlas, AtlasError> {
        let mut atlas = Atlas::parse(json)?;
        atlas.set_image(image)?;
        Ok(atlas)
    }

    pub fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    pub fn glyph(&self, codepoint: u32) -> Option<&Glyph> {
        self.index
            .get(&codepoint)
            .map(|&position| &self.glyphs[position])
    }

    /// The glyph a codepoint that is not in the atlas is drawn with: the first
    /// entry, not the lowest codepoint. "First" is what the atlas says, and it is
    /// stable across two atlases of the same face.
    pub fn fallback(&self) -> &Glyph {
        &self.glyphs[0]
    }

    /// Exact glyph, or the fallback. The C++ dropped an unknown character
    /// (`Renderer2D.cpp:504-506`), which silently shortens the run; a wrong glyph
    /// in the right place is what a caller can act on.
    pub fn glyph_or_fallback(&self, codepoint: u32) -> &Glyph {
        self.glyph(codepoint).unwrap_or_else(|| self.fallback())
    }

    pub fn kerning(&self, first: u32, second: u32) -> f32 {
        self.kerning.get(&(first, second)).copied().unwrap_or(0.0)
    }

    /// Em units to layout units. One line height is 1.0, exactly as the C++
    /// `DrawString` scaled (`Renderer2D.cpp:463`).
    pub fn scale(&self) -> f32 {
        if self.metrics.line_height > 0.0 {
            1.0 / self.metrics.line_height
        } else {
            1.0
        }
    }

    /// True when the atlas is a signed distance field, which is what the shader's
    /// screen-space range path needs. A coverage atlas is not, and says so.
    pub fn is_msdf(&self) -> bool {
        self.metrics.distance_range > 0.0
    }
}

/// What `Font.cpp:23` passed to the packer, and what the shader assumed when the
/// JSON did not carry it (`Text.slang` hardcoded `float2(2.0, 2.0)`).
const DEFAULT_DISTANCE_RANGE: f32 = 2.0;

#[derive(serde::Deserialize)]
struct RawAtlas {
    atlas: RawAtlasInfo,
    metrics: RawMetrics,
    glyphs: Vec<RawGlyph>,
    #[serde(default)]
    kerning: Vec<RawKerning>,
}

#[derive(serde::Deserialize)]
struct RawAtlasInfo {
    #[serde(default, rename = "distanceRange")]
    distance_range: Option<f32>,
    width: u32,
    height: u32,
}

#[derive(serde::Deserialize)]
struct RawMetrics {
    #[serde(default, rename = "emSize")]
    em_size: f32,
    #[serde(default, rename = "lineHeight")]
    line_height: f32,
    #[serde(default)]
    ascender: f32,
    #[serde(default)]
    descender: f32,
}

#[derive(serde::Deserialize)]
struct RawGlyph {
    unicode: u32,
    #[serde(default)]
    advance: f32,
    #[serde(default, rename = "planeBounds")]
    plane_bounds: Option<RawBounds>,
    #[serde(default, rename = "atlasBounds")]
    atlas_bounds: Option<RawBounds>,
}

#[derive(serde::Deserialize, Default, Clone, Copy)]
struct RawBounds {
    #[serde(default)]
    left: f32,
    #[serde(default)]
    bottom: f32,
    #[serde(default)]
    right: f32,
    #[serde(default)]
    top: f32,
}

#[derive(serde::Deserialize)]
struct RawKerning {
    unicode1: u32,
    unicode2: u32,
    #[serde(default)]
    advance: f32,
}
