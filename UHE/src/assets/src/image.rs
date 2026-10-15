use crate::error::{Error, Result};

/// A decoded 8-bit RGBA image, raster order, `rgba8.len() == width * height * 4`.
///
/// The C++ handed a `Texture2D` (an uploaded device texture) to anything that
/// wanted pixels, so nothing without a device could look at one. This is the
/// CPU half of `Texture.cpp`: stb/KTX2 decoded bytes and an extent, with the
/// colour space carried as a flag instead of baked into a sampler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
    /// True when the texels are sRGB-encoded and the sampler must decode them.
    /// A property of the *slot* the image is bound to, not of the file, which is
    /// why it is a field the loader sets rather than something the decoder
    /// decides.
    pub srgb: bool,
}

impl Image {
    /// Byte size of an RGBA8 image of this extent.
    pub fn byte_size(&self) -> usize {
        rgba8_size(self.width, self.height)
    }
}

/// Byte size of an RGBA8 image of the given dimensions.
///
/// The counterpart of `RGBA8Size` (`Mipmap.h:56`).
pub const fn rgba8_size(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4
}

/// Decodes PNG/JPEG/BMP/TGA bytes to RGBA8.
///
/// The C++ went through stb_image for every path including KTX2's fallback. One
/// entry point for all four encodings is what makes "decode these bytes" a
/// question with one answer; `image` picks the codec from the bytes themselves,
/// so a `.png` named `.jpg` still decodes.
pub fn decode_image(bytes: &[u8], srgb: bool) -> Result<Image> {
    let decoded = match image::load_from_memory(bytes) {
        Ok(decoded) => decoded,
        // TGA carries no magic number, so the format guesser cannot recognise
        // it; it is the one encoding that has to be named to be decoded.
        Err(guessed) => match image::load_from_memory_with_format(bytes, image::ImageFormat::Tga) {
            Ok(decoded) => decoded,
            Err(_) => return Err(Error::Decode(format!("image: {guessed}"))),
        },
    };
    let rgba = decoded.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Ok(Image {
        width,
        height,
        rgba8: rgba.into_raw(),
        srgb,
    })
}

/// One RGBA8 mip level: `width * height * 4` bytes, raster order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// Half-size copy of an RGBA8 image, by 2x2 box filter.
///
/// Averaging happens in the encoded space, which darkens slightly against a
/// linear-space filter; that is the standard trade for a cheap CPU path and is
/// what the GPU blit chain does too, so a level generated here and one
/// generated on device are the same pixels. Odd source edges duplicate their
/// last texel, matching how the block formats and the KTX2 container size
/// their levels.
///
/// Port of `DownsampleRGBA8` (`Mipmap.cpp:19`).
pub fn downsample_rgba8(source: &[u8], width: u32, height: u32) -> Option<MipLevel> {
    if width == 0 || height == 0 || source.len() < rgba8_size(width, height) {
        return None;
    }

    let dst_width = (width / 2).max(1);
    let dst_height = (height / 2).max(1);
    let mut rgba8 = vec![0u8; rgba8_size(dst_width, dst_height)];

    for (y, row) in rgba8.chunks_exact_mut(dst_width as usize * 4).enumerate() {
        for (x, texel) in row.chunks_exact_mut(4).enumerate() {
            let x = x as u32;
            let y = y as u32;
            let x0 = (x * 2).min(width - 1);
            let y0 = (y * 2).min(height - 1);
            let x1 = (x * 2 + 1).min(width - 1);
            let y1 = (y * 2 + 1).min(height - 1);

            let texel00 = offset(x0, y0, width);
            let texel10 = offset(x1, y0, width);
            let texel01 = offset(x0, y1, width);
            let texel11 = offset(x1, y1, width);

            for (channel, out) in texel.iter_mut().enumerate() {
                *out = average(
                    source[texel00 + channel],
                    source[texel10 + channel],
                    source[texel01 + channel],
                    source[texel11 + channel],
                );
            }
        }
    }

    Some(MipLevel {
        width: dst_width,
        height: dst_height,
        rgba8,
    })
}

/// Mean of four channel values, rounded to nearest as the C++ did
/// (`(sum + 2) / 4`).
fn average(a: u8, b: u8, c: u8, d: u8) -> u8 {
    ((a as u32 + b as u32 + c as u32 + d as u32 + 2) / 4) as u8
}

/// Full chain: level 0 is a copy of the source, then repeated halving until
/// 1x1. A 1x1 source yields exactly one level.
///
/// Port of `GenerateMipmapChainRGBA8` (`Mipmap.cpp:68`). CPU-side and
/// device-free on purpose: the headless harness, editor thumbnails and offline
/// export all need a chain and none of them has a device.
pub fn generate_mipmap_chain_rgba8(source: &[u8], width: u32, height: u32) -> Vec<MipLevel> {
    let mut chain = Vec::new();
    if source.is_empty() || width == 0 || height == 0 || source.len() < rgba8_size(width, height) {
        return chain;
    }

    chain.push(MipLevel {
        width,
        height,
        rgba8: source[..rgba8_size(width, height)].to_vec(),
    });

    let (mut level_width, mut level_height) = (width, height);
    while level_width > 1 || level_height > 1 {
        let previous = chain.last().map(|level| level.rgba8.as_slice());
        let downsampled =
            previous.and_then(|bytes| downsample_rgba8(bytes, level_width, level_height));
        let Some(next) = downsampled else {
            break;
        };
        level_width = next.width;
        level_height = next.height;
        chain.push(next);
    }

    chain
}

/// Byte offset of texel `(x, y)` in a raster-order RGBA8 image.
fn offset(x: u32, y: u32, width: u32) -> usize {
    (y as usize * width as usize + x as usize) * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x4 whose every texel is its own coordinates, so a wrong offset is
    /// visible in the value and not only in the extent.
    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(rgba8_size(width, height));
        for y in 0..height {
            for x in 0..width {
                bytes.extend_from_slice(&[x as u8, y as u8, 255, 255]);
            }
        }
        bytes
    }

    #[test]
    fn downsample_averages_each_2x2_block() {
        let source = gradient(4, 4);
        let half = downsample_rgba8(&source, 4, 4).expect("4x4 downsamples");
        assert_eq!((half.width, half.height), (2, 2));
        // Texel (0, 0) averages source texels (0, 0), (1, 0), (0, 1), (1, 1):
        // x averages 0 and 1 to 0, y averages 0 and 1 to 0.
        assert_eq!(&half.rgba8[0..4], &[0, 0, 255, 255]);
        // Texel (1, 0) averages x in {2, 3} -> 2.
        assert_eq!(&half.rgba8[offset(1, 0, 2)..offset(1, 0, 2) + 4], &[2, 0, 255, 255]);
    }

    #[test]
    fn downsample_rounds_odd_extents_up_and_duplicates_the_edge() {
        let source = gradient(3, 3);
        let half = downsample_rgba8(&source, 3, 3).expect("3x3 downsamples");
        // Odd edges round up, exactly as the KTX2 container sizes them.
        assert_eq!((half.width, half.height), (1, 1));
        // The 2x2 block is source (0,0), (1,0), (0,1), (1,1) - the out-of-range
        // column and row duplicate texel 1 rather than reading past the image.
        assert_eq!(&half.rgba8[0..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn downsample_refuses_a_short_source() {
        assert!(downsample_rgba8(&[0, 0, 0, 0], 4, 4).is_none());
        assert!(downsample_rgba8(&[], 0, 0).is_none());
    }

    #[test]
    fn chain_halves_until_one_by_one() {
        let source = gradient(4, 2);
        let chain = generate_mipmap_chain_rgba8(&source, 4, 2);
        let extents: Vec<(u32, u32)> = chain.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(extents, vec![(4, 2), (2, 1), (1, 1)]);
        assert_eq!(chain[0].rgba8, source);
    }

    #[test]
    fn chain_of_a_one_by_one_is_one_level() {
        let chain = generate_mipmap_chain_rgba8(&[1, 2, 3, 4], 1, 1);
        assert_eq!(chain.len(), 1);
    }
}
