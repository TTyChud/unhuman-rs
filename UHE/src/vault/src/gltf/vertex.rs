//! The vertex format: one vertex, and the layout that uploads it.
//!
//! ## 88 bytes, and why not fewer
//!
//! | field | bytes | |
//! |---|---|---|
//! | `position` | 12 | `vec3` |
//! | `normal` | 12 | `vec3`, synthesized when the file omits it |
//! | `uv` | 8 | `vec2`, V flipped on load |
//! | `tangent` | 16 | `[f32; 4]` — xyz plus handedness in `w` |
//! | `joints` | 8 | 4 x `i16` |
//! | `weights` | 16 | 4 x `f32` |
//! | `colour` | 16 | `[f32; 4]` |
//! | **total** | **88** | |
//!
//! The alternatives are worse. Packing joints into `u8` would cap a skeleton at
//! 256 bones, which is real, and would not save anything once the weights are
//! 16 bytes anyway. Dropping colour for the 80% of models that have none would
//! mean two vertex layouts and two pipeline variants, and the branch on which
//! one a mesh uses is a bug waiting to happen.
//!
//! ## Why `[f32; 4]` and not `Vec4`
//!
//! Because `glam`'s `Vec4` is **16-byte aligned**. That is the right choice for
//! a standalone vector and the wrong one here: a `Vec4` after a `[f32; 4]` field
//! forces 8 bytes of padding, so the vertex becomes 96 bytes with a hole in the
//! middle — and vertex merging hashes every byte, so that hole is uninitialised
//! memory being compared between vertices. Two vertices that should merge can
//! differ in the padding, and the merge silently stops working.
//!
//! `[f32; 4]` is 4-byte aligned like everything else here, so the struct is 88
//! contiguous bytes. `Vec4::from_array` converts at the few call sites that want
//! vector maths. The padding test in `tests/gltf_optimize.rs` is what keeps this
//! true.

use glam::{Vec2, Vec3};

/// One vertex. `#[repr(C)]` because this is memcpy'd to the GPU.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
#[repr(C)]
pub struct Vertex {
    /// Object-space position. Required -- a primitive without one is skipped.
    pub position: Vec3,
    /// Object-space normal. Synthesized when the file omits it.
    pub normal: Vec3,
    /// Texture coordinate, V already flipped on load.
    pub uv: Vec2,
    /// xyz is the tangent, `w` is the bitangent sign. `[f32; 4]` rather than
    /// `Vec4` -- see the module docs on padding.
    pub tangent: [f32; 4],
    /// Four bone indices, or `[-1; 4]` for an unskinned mesh.
    pub joints: [i16; 4],
    /// Bone weights, summing to 1. All zero for an unskinned mesh.
    pub joint_weights: [f32; 4],
    /// Vertex colour. **Defaults to white, never black** -- black tints every
    /// untextured model black.
    pub colour: [f32; 4],
}

impl Vertex {
    /// The stride every vertex occupies.
    pub const STRIDE: u32 = std::mem::size_of::<Vertex>() as u32;

    /// Bytes for one attribute, by name, for building a pipeline's vertex layout.
    pub fn attribute_stride(name: &str) -> Option<u32> {
        Some(match name {
            "position" | "normal" => 12,
            "uv" => 8,
            "tangent" | "colour" => 16,
            "joints" => 8,
            "joint_weights" => 16,
            _ => return None,
        })
    }

    /// A vertex for a static mesh: no skinning, white, tangent along +X.
    ///
    /// The defaults matter. `colour` must be **white**, not black -- black
    /// tints every untextured model to black, which is the single most common
    /// "why is my model black" cause. `tangent.w` must be `1.0`, because a zero
    /// handedness makes every normal map mirror.
    pub fn unskinned(position: Vec3) -> Self {
        Self {
            position,
            normal: Vec3::Y,
            uv: Vec2::ZERO,
            tangent: [1.0, 0.0, 0.0, 1.0],
            joints: [-1; 4],
            joint_weights: [0.0; 4],
            colour: [1.0; 4],
        }
    }

    /// Whether this vertex is influenced by a skeleton.
    pub fn is_skinned(&self) -> bool {
        self.joint_weights.iter().any(|w| *w > 0.0)
    }

    /// The exact bytes, for hashing during vertex merging.
    ///
    /// Hashing the *bytes* rather than the fields is the point, and it is why the
    /// tangent is generated **before** merging: two vertices at the same
    /// position with different UVs must not merge, and any field-wise
    /// comparison that forgets UV will happily collapse a UV seam into a
    /// degenerate strip.
    pub fn identity_bytes(&self) -> [u8; std::mem::size_of::<Vertex>()] {
        // Sound because `Vertex` is `#[repr(C)]` with no padding, so every bit
        // pattern of this size is a valid `Vertex`. That is not assumed: the
        // `vertex_has_no_padding` test in `tests/gltf_optimize.rs` walks every
        // field offset and asserts they are contiguous.
        let mut bytes = [0u8; std::mem::size_of::<Vertex>()];
        // SAFETY: both sides are `size_of::<Vertex>()` bytes, `bytes` is a live
        // array of that length, and `Vertex` is POD with no padding.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (self as *const Vertex).cast::<u8>(),
                bytes.as_mut_ptr(),
                bytes.len(),
            );
        }
        bytes
    }
}

/// Byte offsets of each attribute within a [`Vertex`].
///
/// Derived from `offset_of!` so it cannot drift from the struct. Upstream C++
/// had a "vertex layout mismatch" bug that this makes impossible: the layout is
/// a property of the type, not a second list someone has to remember to update.
pub mod offsets {
    use std::mem::offset_of;

    use super::Vertex;

    /// Byte offset of `position`.
    pub const POSITION: u32 = offset_of!(Vertex, position) as u32;
    /// Byte offset of `normal`.
    pub const NORMAL: u32 = offset_of!(Vertex, normal) as u32;
    /// Byte offset of `uv`.
    pub const UV: u32 = offset_of!(Vertex, uv) as u32;
    /// Byte offset of `tangent`.
    pub const TANGENT: u32 = offset_of!(Vertex, tangent) as u32;
    /// Byte offset of `joints`.
    pub const JOINTS: u32 = offset_of!(Vertex, joints) as u32;
    /// Byte offset of `joint_weights`.
    pub const JOINT_WEIGHTS: u32 = offset_of!(Vertex, joint_weights) as u32;
    /// Byte offset of `colour`.
    pub const COLOUR: u32 = offset_of!(Vertex, colour) as u32;
}
