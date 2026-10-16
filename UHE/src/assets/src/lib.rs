//! Asset registry, virtual file system, loaders.
//!
//! Port of `UHE/src/UHE/AssestsManager/` (`AssetManager`, `VfsSystem`), the
//! CPU half of `UHE/src/UHE/Renderer/{Texture,KTX2,Mipmap}` and the
//! geometry/skin/animation reading of `UHE/src/UHE/Renderer3D/LoadModel*.cpp`.
//!
//! The C++ registry was path-based and keyed by payload type
//! (`Load<TextAsset>("a.txt")`), with an `AssetHandle` that held a
//! `shared_ptr<AssetRecord>` and compared a generation counter on every
//! dereference. `IDEA.md:61` calls that debt. Here the registry is the only way
//! to reach a payload: every asset is an [`AssetId`] (slot index + generation),
//! a caller asks for the id once with [`Assets::load`], and a stale id is
//! refused rather than silently aliasing whatever took the slot next.
//!
//! Nothing here names a GPU type. `chassis` is not a dependency of this crate,
//! so the headless binary links the whole loader set and the editor can decode
//! a texture without a device.

mod anim;
mod assets;
mod error;
mod id;
mod image;
mod ktx2;
mod load_model;
mod load_model_animation;
mod load_model_geometry;
mod mesh;
mod vfs;

pub use anim::{compress, measure, CompressSettings, CompressionStats};
pub use assets::{Assets, LoadResult};
pub use error::{Error, Result};
pub use id::{AssetData, AssetId, AssetKind, Handle};
pub use image::{
    decode_image, downsample_rgba8, generate_mipmap_chain_rgba8, rgba8_size, Image, MipLevel,
};
pub use ktx2::{is_ktx2, load_ktx2, Ktx2Image};
pub use mesh::{
    AlphaMode, AnimationData, BoneData, Filter, Interpolation, MaterialData, MeshData, ModelData,
    ModelNode, QuaternionTrack, SamplerData, SkeletonData, SkinData, TextureData, VectorTrack,
    Wrap,
};
pub use vfs::Vfs;
