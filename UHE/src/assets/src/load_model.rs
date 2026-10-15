//! glTF/GLB model loading entry point and scene-graph walk.
//!
//! Port of `UHE/src/UHE/Renderer3D/LoadModel.cpp`. This file owns everything
//! that needs the whole asset — materials, the scene walk, resource lifetime —
//! while primitive extraction and animation parsing live in their own modules,
//! which is the same split the C++ used across `LoadModel.cpp`,
//! `LoadModelGeometry.cpp` and `LoadModelAnimation.cpp`.
//!
//! **Not ported from those files:** `EXT_meshopt_compression`. The C++ decoded
//! compressed buffer views through meshoptimizer (`MeshoptDecode.cpp`), a C
//! library. Reading those views without decoding yields garbage geometry, so a
//! file that declares the extension is refused with that reason instead. The
//! KHR extension blocks (`KHR_materials_clearcoat`, `…specular`, `…transmission`,
//! `KHR_texture_transform`, `KHR_lights_punctual`, `KHR_texture_basisu`) are not
//! read either: `gltf`'s accessors for them are feature-gated and the workspace
//! manifest enables only `utils`/`names`/`extensions`. They are reported by name
//! in [`ModelData::unsupported_extensions`], which is what the C++ did with an
//! extension it had not implemented.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use glam::Mat4;
use gltf::buffer::Data as BufferData;

use crate::error::{Error, Result};
use crate::image::{decode_image, Image, MipLevel};
use crate::ktx2::{is_ktx2, load_ktx2};
use crate::load_model_animation::{parse_animations, parse_skins};
use crate::load_model_geometry::extract_primitive;
use crate::mesh::{
    AlphaMode, Filter, MaterialData, MeshData, ModelData, ModelNode, SamplerData, TextureData, Wrap,
};

/// Extensions whose values reach this crate's output.
///
/// The C++ list (`IsExtensionSupported`, `LoadModel.cpp:32`) held an extension
/// only if its values were actually shaded with, because a parsed-but-ignored
/// extension renders subtly wrong with no error anywhere. This build reads no
/// extension at all — the `gltf` features that would parse them are off — so
/// the list is empty and every declared name is reported instead.
const SUPPORTED_EXTENSIONS: &[&str] = &[];

pub(crate) struct LoadedModel {
    pub meshes: Vec<MeshData>,
    pub model: ModelData,
}

/// The bytes of one buffer view, for the accessor readers.
///
/// The returned slice borrows from `buffers`, not from the `Buffer` handle, so
/// the two lifetimes are independent: a reader needs the bytes to outlive its
/// own borrow of the document, and tying them together would refuse that.
pub(crate) fn view_bytes<'a, 'b>(
    buffer: gltf::Buffer<'a>,
    buffers: &'b [BufferData],
) -> Option<&'b [u8]> {
    buffers.get(buffer.index()).map(|data| data.0.as_slice())
}

/// The local transform of one node, `T * R * S` or the file's own matrix.
///
/// Port of `Model::NodeLocalTransform` (`LoadModel.cpp:112`).
pub(crate) fn node_transform(node: &gltf::Node<'_>) -> Mat4 {
    let matrix = node.transform().matrix();
    Mat4::from_cols_array_2d(&matrix)
}

/// Loads a glTF or GLB file.
///
/// `path` is the physical file the bytes came from: a `.gltf` may reference
/// buffers and images relative to itself, and that directory is the only thing
/// that makes those references resolvable.
pub(crate) fn load(path: &Path, bytes: &[u8]) -> Result<LoadedModel> {
    let parsed = match gltf::Gltf::from_slice(bytes) {
        Ok(parsed) => parsed,
        // The C++ parsed with `DontRequireValidAssetMember`; a file that fails
        // the crate's validation is retried unvalidated rather than refused.
        Err(gltf::Error::Validation(_)) => gltf::Gltf::from_slice_without_validation(bytes)
            .map_err(|error| Error::Decode(format!("glTF: {error}")))?,
        Err(error) => return Err(Error::Decode(format!("glTF: {error}"))),
    };

    let gltf::Gltf { document, blob } = parsed;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let buffers = gltf::import_buffers(&document, Some(base), blob)
        .map_err(|error| Error::Decode(format!("glTF buffers: {error}")))?;

    let mut unsupported_extensions: Vec<String> = Vec::new();
    for name in document
        .extensions_used()
        .chain(document.extensions_required())
    {
        if name == "EXT_meshopt_compression" {
            return Err(Error::Unsupported(
                "EXT_meshopt_compression: the file's buffer views are meshopt-compressed and the \
                 meshoptimizer decoder is a C library this crate does not link, so the bytes would \
                 decode as garbage geometry"
                    .to_owned(),
            ));
        }
        if !SUPPORTED_EXTENSIONS.contains(&name)
            && !unsupported_extensions
                .iter()
                .any(|seen| seen.as_str() == name)
        {
            unsupported_extensions.push(name.to_owned());
        }
    }
    for name in &unsupported_extensions {
        log::warn!("glTF declares unimplemented extension '{name}' - shading may be wrong");
    }

    let materials = load_materials(&document, base, &buffers);
    let (skins, skeleton) = parse_skins(&document, &buffers);
    let animations = parse_animations(&document, &buffers);

    let nodes: Vec<gltf::Node> = document.nodes().collect();
    let mut walk = Walk {
        nodes,
        buffers: buffers.as_slice(),
        materials: materials.as_slice(),
        geometry: HashMap::new(),
        mesh_data: Vec::new(),
        model_nodes: Vec::new(),
    };

    let mut roots = Vec::new();
    // The C++ fell back to scene 0 when the file named no default scene.
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
    if let Some(scene) = scene {
        let root_indices: Vec<usize> = scene.nodes().map(|node| node.index()).collect();
        for index in root_indices {
            let slot = walk.walk(index, Mat4::IDENTITY);
            if slot != u32::MAX {
                roots.push(slot);
            }
        }
    }

    // A model's clips and skeleton belong to the model, but the contract puts
    // them on `MeshData`; the first mesh carries them and the rest stay empty.
    let mut meshes = walk.mesh_data;
    if let Some(first) = meshes.first_mut() {
        first.animations = animations;
        if !skins.is_empty() {
            first.skeleton = Some(skeleton);
        }
    }

    let model = ModelData {
        nodes: walk.model_nodes,
        roots,
        skins,
        unsupported_extensions,
    };
    Ok(LoadedModel { meshes, model })
}

/// The glTF scene walk: node records in depth-first order, geometry extracted
/// once per `glTF` mesh however many nodes reference it.
struct Walk<'a, 'b> {
    nodes: Vec<gltf::Node<'a>>,
    buffers: &'b [BufferData],
    materials: &'b [MaterialData],
    /// `glTF` mesh index -> indices into `mesh_data`.
    geometry: HashMap<usize, Vec<u32>>,
    mesh_data: Vec<MeshData>,
    model_nodes: Vec<ModelNode>,
}

impl<'a, 'b> Walk<'a, 'b> {
    /// Appends one node and its subtree, returning its index in
    /// `model_nodes`, or `u32::MAX` for a node index that does not exist.
    ///
    /// Port of `Model::ProcessNode` (`LoadModel.cpp:756`). The C++ set the
    /// parent and child links *after* recursing because a reference into
    /// `m_Nodes` would dangle the moment a child was appended; the same trap
    /// exists here, and the answer is the same: keep the index, not the
    /// reference.
    fn walk(&mut self, node_index: usize, parent_transform: Mat4) -> u32 {
        let Some(node) = self.nodes.get(node_index).cloned() else {
            return u32::MAX;
        };

        let (translation, rotation, scale) = node.transform().decomposed();
        let world_transform = parent_transform * node_transform(&node);
        let name = node
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Node_{node_index}"));

        let (mesh, primitives) = match node.mesh() {
            Some(mesh) => (mesh.index() as i32, self.geometry_for(&mesh)),
            None => (-1, Vec::new()),
        };

        let slot = self.model_nodes.len() as u32;
        self.model_nodes.push(ModelNode {
            name,
            parent: -1,
            children: Vec::new(),
            mesh,
            primitives,
            translation,
            rotation,
            scale,
            world_transform: world_transform.to_cols_array_2d(),
        });

        let children: Vec<usize> = node.children().map(|child| child.index()).collect();
        for child_index in children {
            let child_slot = self.walk(child_index, world_transform);
            if child_slot == u32::MAX {
                continue;
            }
            if let Some(child) = self.model_nodes.get_mut(child_slot as usize) {
                child.parent = slot as i32;
            }
            if let Some(parent) = self.model_nodes.get_mut(slot as usize) {
                parent.children.push(child_slot);
            }
        }

        slot
    }

    /// Extracts a `glTF` mesh at most once, however many nodes use it.
    ///
    /// This is what makes per-node transforms affordable: repeated props and
    /// foliage reference one set of vertex arrays instead of one each.
    fn geometry_for(&mut self, mesh: &gltf::Mesh<'a>) -> Vec<u32> {
        if let Some(cached) = self.geometry.get(&mesh.index()) {
            return cached.clone();
        }

        let name = mesh
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| "Unnamed_Mesh".to_owned());
        let mut indices = Vec::new();
        for primitive in mesh.primitives() {
            if let Some(extracted) =
                extract_primitive(&primitive, self.buffers, self.materials, &name)
            {
                indices.push(self.mesh_data.len() as u32);
                self.mesh_data.push(extracted);
            }
        }

        self.geometry.insert(mesh.index(), indices.clone());
        indices
    }
}

/// Decoded material textures, keyed by (image index, colour space, sampler).
type TextureCache = HashMap<(usize, bool, SamplerData), Option<Arc<TextureData>>>;

/// Every material in the file, with its textures decoded.
///
/// Port of `Model::LoadMaterials` (`LoadModel.cpp:330`). Colour space is a
/// property of the *slot*: `baseColor` and `emissive` are sRGB, `metallicRoughness`,
/// `normal` and `occlusion` are linear data, and using one format for all of
/// them gamma-decodes the linear maps on every sample.
fn load_materials(
    document: &gltf::Document,
    base: &Path,
    buffers: &[BufferData],
) -> Vec<MaterialData> {
    // One glTF image is routinely bound by several materials and slots; without
    // this the identical pixels are decoded and resident once per reference
    // (`LoadModel.cpp:363`).
    let mut cache = TextureCache::new();
    let mut materials = Vec::new();

    for material in document.materials() {
        let mut out = MaterialData {
            name: material.name().map(str::to_owned).unwrap_or_default(),
            ..Default::default()
        };

        let pbr = material.pbr_metallic_roughness();
        out.base_color = pbr.base_color_factor();
        out.metallic = pbr.metallic_factor();
        out.roughness = pbr.roughness_factor();
        out.emissive = material.emissive_factor();
        out.alpha = match material.alpha_mode() {
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
        };
        out.alpha_cutoff = material.alpha_cutoff().unwrap_or(0.5);
        out.double_sided = material.double_sided();

        out.albedo = pbr.base_color_texture().and_then(|info| {
            load_texture(info.texture(), &mut cache, base, buffers, true)
        });
        out.metallic_roughness = pbr.metallic_roughness_texture().and_then(|info| {
            // Metallic (B) and roughness (G) are scalars, not colour: gamma
            // decoding them shifts both.
            load_texture(info.texture(), &mut cache, base, buffers, false)
        });

        if let Some(normal) = material.normal_texture() {
            out.normal = load_texture(normal.texture(), &mut cache, base, buffers, false);
            out.normal_scale = normal.scale();
        }
        if let Some(occlusion) = material.occlusion_texture() {
            out.occlusion = load_texture(occlusion.texture(), &mut cache, base, buffers, false);
            out.occlusion_strength = occlusion.strength();
        }
        if let Some(info) = material.emissive_texture() {
            out.emissive_texture = load_texture(info.texture(), &mut cache, base, buffers, true);
        }

        materials.push(out);
    }

    materials
}

fn load_texture(
    texture: gltf::Texture<'_>,
    cache: &mut TextureCache,
    base: &Path,
    buffers: &[BufferData],
    srgb: bool,
) -> Option<Arc<TextureData>> {
    let sampler = sampler_data(texture.sampler());
    let key = (texture.index(), srgb, sampler);
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }

    let image = image_bytes(texture.source().source(), base, buffers)
        .and_then(|bytes| decode_texture(&bytes, srgb))
        .map(|(image, mips)| {
            Arc::new(TextureData {
                image: Arc::new(image),
                mips: mips.into(),
                sampler,
            })
        });

    cache.insert(key, image.clone());
    image
}

/// Decodes an image payload, KTX2 or otherwise, keeping any authored mip chain.
fn decode_texture(bytes: &[u8], srgb: bool) -> Option<(Image, Vec<MipLevel>)> {
    if is_ktx2(bytes) {
        match load_ktx2(bytes) {
            Ok(ktx) => {
                let image = Image {
                    width: ktx.width,
                    height: ktx.height,
                    rgba8: ktx.rgba8,
                    srgb: ktx.srgb,
                };
                Some((image, ktx.mips))
            }
            Err(error) => {
                log::warn!("model texture: {error}");
                None
            }
        }
    } else {
        match decode_image(bytes, srgb) {
            Ok(image) => Some((image, Vec::new())),
            Err(error) => {
                log::warn!("model texture: {error}");
                None
            }
        }
    }
}

/// The encoded bytes of one `glTF` image, however the file stores it.
///
/// Port of the `std::visit` in `LoadModel.cpp:399`: an external URI, a
/// `data:` URI, or a slice of a buffer view (which in a GLB is the common case
/// for embedded textures).
fn image_bytes(
    source: gltf::image::Source<'_>,
    base: &Path,
    buffers: &[BufferData],
) -> Option<Vec<u8>> {
    match source {
        gltf::image::Source::Uri { uri, .. } => {
            if let Some(rest) = uri.strip_prefix("data:") {
                let (_, payload) = rest.split_once(";base64,")?;
                base64_decode(payload)
            } else {
                std::fs::read(base.join(uri)).ok()
            }
        }
        gltf::image::Source::View { view, .. } => {
            let buffer = buffers.get(view.buffer().index())?;
            let end = view.offset().checked_add(view.length())?;
            buffer.0.get(view.offset()..end).map(|bytes| bytes.to_vec())
        }
    }
}

/// The sampler a texture declares, with the spec defaults for what it omits.
///
/// Port of `SamplerDescForTexture` (`LoadModel.cpp:213`). Every mip-map variant
/// of a filter folds into the filter itself, because mip selection is the
/// sampler's other axis; wrap S and wrap T stay independent, since applying one
/// mode to both axes is wrong for any atlas that clamps U and repeats V.
fn sampler_data(sampler: gltf::texture::Sampler<'_>) -> SamplerData {
    let default = SamplerData::default();
    SamplerData {
        mag_filter: sampler.mag_filter().map_or(default.mag_filter, filter),
        min_filter: sampler.min_filter().map_or(default.min_filter, min_filter),
        wrap_s: wrap(sampler.wrap_s()),
        wrap_t: wrap(sampler.wrap_t()),
    }
}

fn filter(mag: gltf::texture::MagFilter) -> Filter {
    match mag {
        gltf::texture::MagFilter::Nearest => Filter::Nearest,
        gltf::texture::MagFilter::Linear => Filter::Linear,
    }
}

fn min_filter(min: gltf::texture::MinFilter) -> Filter {
    match min {
        gltf::texture::MinFilter::Nearest
        | gltf::texture::MinFilter::NearestMipmapNearest
        | gltf::texture::MinFilter::NearestMipmapLinear => Filter::Nearest,
        gltf::texture::MinFilter::Linear
        | gltf::texture::MinFilter::LinearMipmapNearest
        | gltf::texture::MinFilter::LinearMipmapLinear => Filter::Linear,
    }
}

fn wrap(mode: gltf::texture::WrappingMode) -> Wrap {
    match mode {
        gltf::texture::WrappingMode::ClampToEdge => Wrap::ClampToEdge,
        gltf::texture::WrappingMode::MirroredRepeat => Wrap::MirroredRepeat,
        gltf::texture::WrappingMode::Repeat => Wrap::Repeat,
    }
}

/// Standard base64, tolerating whitespace and stopping at the padding.
///
/// The glTF crate decodes `data:` URIs for buffers itself, but an *image*
/// embedded that way is handed to the caller as a URI, so one of the two has to
/// own the decoder. Kept local and small rather than adding a crate for it.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        } as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_padding_and_ignores_whitespace() {
        assert_eq!(base64_decode("AQIDBA=="), Some(vec![1, 2, 3, 4]));
        assert_eq!(base64_decode("AQID\nBA=="), Some(vec![1, 2, 3, 4]));
        assert_eq!(base64_decode(""), Some(Vec::new()));
        assert_eq!(base64_decode("AQID*"), None);
    }

    #[test]
    fn min_filter_folds_the_mip_variants() {
        assert_eq!(
            min_filter(gltf::texture::MinFilter::LinearMipmapLinear),
            Filter::Linear
        );
        assert_eq!(
            min_filter(gltf::texture::MinFilter::NearestMipmapNearest),
            Filter::Nearest
        );
    }
}
