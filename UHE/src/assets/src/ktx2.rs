//! KTX2 container parsing.
//!
//! Port of `UHE/src/UHE/Renderer/KTX2.{h,cpp}`, minus the transcode. The C++
//! handed the bytes to `basisu_transcoder`, which is a C++-only library pulled
//! in at build time; `docs/architecture/01-crate-map.md` records `basis_universal`
//! as deliberately not ported. What is portable is the container: header, level
//! index, DFD presence, and the mip chain — everything a plain, uncompressed
//! (or RDO-compressed-then-inflated-by-the-author) KTX2 needs, and everything
//! needed to *refuse* a Basis payload with a reason instead of handing a
//! renderer bytes it will upload as noise.

use crate::error::{Error, Result};
use crate::image::{rgba8_size, MipLevel};

/// The KTX2 identifier, byte for byte as the specification defines it.
const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// Identifier, nine `u32` header fields and the two `u64` supercompression
/// offsets: 80 bytes, which is also where the level index begins.
const HEADER_SIZE: usize = 80;
const LEVEL_INDEX_STRIDE: usize = 24;

const SS_NONE: u32 = 0;
const SS_BASISLZ: u32 = 1;
const SS_ZSTANDARD: u32 = 2;
const SS_ZLIB: u32 = 3;

const VK_FORMAT_UNDEFINED: u32 = 0;
const VK_FORMAT_R8G8B8_UNORM: u32 = 23;
const VK_FORMAT_R8G8B8_SRGB: u32 = 29;
const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
const VK_FORMAT_R8G8B8A8_SRGB: u32 = 43;
const VK_FORMAT_B8G8R8A8_UNORM: u32 = 44;
const VK_FORMAT_B8G8R8A8_SRGB: u32 = 50;

/// A chain longer than this cannot belong to a `u32`-sized texture, and the
/// bound is what keeps a hostile level count out of the index arithmetic.
const MAX_LEVELS: u32 = 32;

/// A decoded KTX2 texture: level 0 plus every level the file carried.
///
/// The C++ `KTX2Image` kept the chain when the file shipped one, so a
/// file-authored chain uploads directly and the GPU generator stays idle —
/// recomputing it would both waste the time and discard the author's filter.
/// `rgba8` aliases level 0 for a caller that never looks further.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ktx2Image {
    pub width: u32,
    pub height: u32,
    pub srgb: bool,
    pub rgba8: Vec<u8>,
    pub mips: Vec<MipLevel>,
}

/// Whether these bytes start with the KTX2 identifier.
///
/// Checking before handing bytes to a parser turns "wrong file passed in" into a
/// clean `false` instead of an error from somewhere else about a truncated
/// container.
pub fn is_ktx2(data: &[u8]) -> bool {
    data.len() >= IDENTIFIER.len() && data[..IDENTIFIER.len()] == IDENTIFIER
}

/// Decodes the first face and layer of every level a KTX2 container carries.
///
/// Refuses everything this build cannot produce correct pixels for, each with
/// the reason named in the error: a Basis payload (there is no transcoder), a
/// supercompressed level (there is no inflater), a cubemap, a texture array, a
/// 3D texture, and any `vkFormat` that is not one of the six 8-bit RGB/RGBA
/// layouts. Parsed structures the engine cannot upload are rejected loudly here
/// rather than silently somewhere downstream.
pub fn load_ktx2(data: &[u8]) -> Result<Ktx2Image> {
    if !is_ktx2(data) {
        return Err(Error::Decode(
            "not a KTX2 container (identifier mismatch)".to_owned(),
        ));
    }

    let header = |offset: usize| {
        le_u32(data, offset)
            .ok_or_else(|| Error::Decode("KTX2 header is truncated".to_owned()))
    };

    let vk_format = header(12)?;
    let type_size = header(16)?;
    let width = header(20)?;
    let height = header(24)?;
    let depth = header(28)?;
    let layers = header(32)?;
    let faces = header(36)?;
    let level_count = header(40)?.max(1);
    let supercompression = header(44)?;

    match supercompression {
        SS_NONE => {}
        SS_BASISLZ => {
            return Err(Error::Unsupported(
                "KTX2 supercompression BasisLZ (ETC1S): needs the Basis transcoder, which is not \
                 ported (docs/architecture/01-crate-map.md, 'Not ported')"
                    .to_owned(),
            ))
        }
        SS_ZSTANDARD => {
            return Err(Error::Unsupported(
                "KTX2 supercompression Zstandard: UASTC/Basis levels ship this way, and this crate \
                 has no zstd inflater"
                    .to_owned(),
            ))
        }
        SS_ZLIB => {
            return Err(Error::Unsupported(
                "KTX2 supercompression ZLIB: this crate has no inflater".to_owned(),
            ))
        }
        other => {
            return Err(Error::Unsupported(format!(
                "KTX2 supercompression scheme {other}: unknown"
            )))
        }
    }

    if vk_format == VK_FORMAT_UNDEFINED {
        return Err(Error::Unsupported(
            "KTX2 with vkFormat VK_FORMAT_UNDEFINED: a Basis payload (UASTC or ETC1S), which needs \
             the Basis transcoder this port does not carry"
                .to_owned(),
        ));
    }
    if faces != 1 {
        return Err(Error::Unsupported(format!(
            "KTX2 with {faces} faces: cubemaps are parsed but not uploadable yet"
        )));
    }
    if layers > 1 {
        return Err(Error::Unsupported(format!(
            "KTX2 with {layers} layers: texture arrays and video are not supported"
        )));
    }
    if depth > 1 {
        return Err(Error::Unsupported(format!(
            "KTX2 with depth {depth}: 3D textures are not supported"
        )));
    }
    if width == 0 || height == 0 {
        return Err(Error::Decode(format!("KTX2 with zero extent ({width}x{height})")));
    }
    if level_count > MAX_LEVELS {
        return Err(Error::Decode(format!(
            "KTX2 declares {level_count} levels, more than a {width}x{height} chain can have"
        )));
    }

    let format = Format::from_vk_format(vk_format)?;
    if type_size != 1 && type_size != 0 {
        return Err(Error::Unsupported(format!(
            "KTX2 typeSize {type_size}: only 8-bit texels are read"
        )));
    }

    let mut mips = Vec::with_capacity(level_count as usize);
    for level in 0..level_count {
        let entry = HEADER_SIZE + level as usize * LEVEL_INDEX_STRIDE;
        let offset = le_u64(data, entry)
            .ok_or_else(|| Error::Decode("KTX2 level index is truncated".to_owned()))?;
        let length = le_u64(data, entry + 8)
            .ok_or_else(|| Error::Decode("KTX2 level index is truncated".to_owned()))?;

        let level_width = (width >> level).max(1);
        let level_height = (height >> level).max(1);
        let expected = format.level_size(level_width, level_height);

        let end_offset = offset
            .checked_add(length)
            .ok_or_else(|| Error::Decode(format!("KTX2 level {level} extent overflows")))?;
        let start = usize::try_from(offset)
            .map_err(|_| Error::Decode(format!("KTX2 level {level} offset is out of range")))?;
        let end = usize::try_from(end_offset)
            .map_err(|_| Error::Decode(format!("KTX2 level {level} extent is out of range")))?;
        let bytes = data.get(start..end).ok_or_else(|| {
            Error::Decode(format!(
                "KTX2 level {level} extends past the end of the file ({start}..{end})"
            ))
        })?;
        if bytes.len() != expected {
            return Err(Error::Decode(format!(
                "KTX2 level {level} is {} bytes, expected {expected} for {level_width}x{level_height}",
                bytes.len()
            )));
        }

        mips.push(MipLevel {
            width: level_width,
            height: level_height,
            rgba8: format.to_rgba8(bytes, level_width, level_height),
        });
    }

    let Some(level0) = mips.first() else {
        return Err(Error::Decode("KTX2 carries no levels".to_owned()));
    };
    let (width, height, rgba8) = (level0.width, level0.height, level0.rgba8.clone());

    Ok(Ktx2Image {
        width,
        height,
        srgb: format.srgb(),
        rgba8,
        mips,
    })
}

/// The 8-bit layouts this build can put in an RGBA8 buffer.
#[derive(Clone, Copy)]
enum Format {
    Rgba8 { srgb: bool },
    Bgra8 { srgb: bool },
    Rgb8 { srgb: bool },
}

impl Format {
    fn from_vk_format(vk_format: u32) -> Result<Self> {
        Ok(match vk_format {
            VK_FORMAT_R8G8B8A8_UNORM => Format::Rgba8 { srgb: false },
            VK_FORMAT_R8G8B8A8_SRGB => Format::Rgba8 { srgb: true },
            VK_FORMAT_B8G8R8A8_UNORM => Format::Bgra8 { srgb: false },
            VK_FORMAT_B8G8R8A8_SRGB => Format::Bgra8 { srgb: true },
            VK_FORMAT_R8G8B8_UNORM => Format::Rgb8 { srgb: false },
            VK_FORMAT_R8G8B8_SRGB => Format::Rgb8 { srgb: true },
            other => {
                return Err(Error::Unsupported(format!(
                    "KTX2 vkFormat {other}: only the 8-bit RGB/RGBA UNORM and SRGB layouts are read"
                )))
            }
        })
    }

    fn channels(self) -> usize {
        match self {
            Format::Rgba8 { .. } | Format::Bgra8 { .. } => 4,
            Format::Rgb8 { .. } => 3,
        }
    }

    fn srgb(self) -> bool {
        match self {
            Format::Rgba8 { srgb } | Format::Bgra8 { srgb } | Format::Rgb8 { srgb } => srgb,
        }
    }

    fn level_size(self, width: u32, height: u32) -> usize {
        width as usize * height as usize * self.channels()
    }

    /// Raster order RGBA8 with `alpha = 255` where the file has none, which is
    /// the layout every upload path in this engine already expects.
    fn to_rgba8(self, bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
        let texels = width as usize * height as usize;
        let mut out = Vec::with_capacity(rgba8_size(width, height));
        match self {
            Format::Rgba8 { .. } => out.extend_from_slice(&bytes[..texels * 4]),
            Format::Bgra8 { .. } => {
                for texel in bytes[..texels * 4].chunks_exact(4) {
                    out.extend_from_slice(&[texel[2], texel[1], texel[0], texel[3]]);
                }
            }
            Format::Rgb8 { .. } => {
                for texel in bytes[..texels * 3].chunks_exact(3) {
                    out.extend_from_slice(&[texel[0], texel[1], texel[2], 255]);
                }
            }
        }
        out
    }
}

fn le_u32(data: &[u8], offset: usize) -> Option<u32> {
    let slice = data.get(offset..offset.checked_add(4)?)?;
    let bytes: [u8; 4] = slice.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn le_u64(data: &[u8], offset: usize) -> Option<u64> {
    let slice = data.get(offset..offset.checked_add(8)?)?;
    let bytes: [u8; 8] = slice.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a one-level, uncompressed KTX2 around `texels`.
    fn container(vk_format: u32, supercompression: u32, width: u32, height: u32, texels: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&IDENTIFIER);
        for value in [
            vk_format,
            1,
            width,
            height,
            0,
            0,
            1,
            1,
            supercompression,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        // dfd/kvd/sgd offsets and lengths, all empty.
        bytes.extend_from_slice(&[0u8; 32]);
        assert_eq!(bytes.len(), HEADER_SIZE);
        // Level index, then the payload.
        bytes.extend_from_slice(&(104u64).to_le_bytes());
        bytes.extend_from_slice(&(texels.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(texels.len() as u64).to_le_bytes());
        bytes.extend_from_slice(texels);
        bytes
    }

    #[test]
    fn an_uncompressed_rgba8_container_decodes() {
        let texels = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let bytes = container(VK_FORMAT_R8G8B8A8_SRGB, SS_NONE, 2, 1, &texels);
        let image = load_ktx2(&bytes).expect("plain container decodes");
        assert_eq!((image.width, image.height), (2, 1));
        assert!(image.srgb);
        assert_eq!(image.rgba8, texels.to_vec());
        assert_eq!(image.mips.len(), 1);
    }

    #[test]
    fn bgra_is_swizzled_and_rgb_expands() {
        let bgra = container(VK_FORMAT_B8G8R8A8_UNORM, SS_NONE, 1, 1, &[1, 2, 3, 4]);
        assert_eq!(load_ktx2(&bgra).expect("bgra decodes").rgba8, vec![3, 2, 1, 4]);

        let rgb = container(VK_FORMAT_R8G8B8_UNORM, SS_NONE, 1, 1, &[9, 8, 7]);
        assert_eq!(load_ktx2(&rgb).expect("rgb decodes").rgba8, vec![9, 8, 7, 255]);
    }

    #[test]
    fn a_basis_payload_is_refused_and_says_why() {
        let basislz = container(VK_FORMAT_UNDEFINED, SS_BASISLZ, 4, 4, &[0, 0, 0, 0]);
        let error = load_ktx2(&basislz).expect_err("BasisLZ is refused");
        let message = error.to_string();
        assert!(message.contains("BasisLZ"), "{message}");
        assert!(message.contains("Basis"), "{message}");

        let uastc = container(VK_FORMAT_UNDEFINED, SS_ZSTANDARD, 4, 4, &[0, 0, 0, 0]);
        let message = load_ktx2(&uastc)
            .expect_err("UASTC over zstd is refused")
            .to_string();
        assert!(message.contains("Zstandard") && message.contains("UASTC"), "{message}");

        let undefined = container(VK_FORMAT_UNDEFINED, SS_NONE, 4, 4, &[0, 0, 0, 0]);
        let message = load_ktx2(&undefined)
            .expect_err("undefined vkFormat is refused")
            .to_string();
        assert!(message.contains("Basis"), "{message}");
    }

    #[test]
    fn cubemaps_arrays_and_3d_textures_are_refused() {
        let mut faces = container(VK_FORMAT_R8G8B8A8_UNORM, SS_NONE, 1, 1, &[0, 0, 0, 0]);
        faces[36..40].copy_from_slice(&6u32.to_le_bytes());
        assert!(load_ktx2(&faces).unwrap_err().to_string().contains("faces"));

        let mut layers = container(VK_FORMAT_R8G8B8A8_UNORM, SS_NONE, 1, 1, &[0, 0, 0, 0]);
        layers[32..36].copy_from_slice(&4u32.to_le_bytes());
        assert!(load_ktx2(&layers).unwrap_err().to_string().contains("layers"));

        let mut depth = container(VK_FORMAT_R8G8B8A8_UNORM, SS_NONE, 1, 1, &[0, 0, 0, 0]);
        depth[28..32].copy_from_slice(&8u32.to_le_bytes());
        assert!(load_ktx2(&depth).unwrap_err().to_string().contains("3D"));
    }

    #[test]
    fn a_truncated_level_is_refused_rather_than_read_past() {
        let mut bytes = container(VK_FORMAT_R8G8B8A8_UNORM, SS_NONE, 2, 2, &[0u8; 16]);
        bytes.truncate(bytes.len() - 8);
        assert!(load_ktx2(&bytes).is_err());
        assert!(!is_ktx2(&bytes[..8]));
    }
}
