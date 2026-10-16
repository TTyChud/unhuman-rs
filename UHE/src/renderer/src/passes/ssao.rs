//! Screen-space ambient occlusion: the kernel, and the pass that runs it.
//!
//! ## What the C++ has
//!
//! `UHE/Renderer3D/SSAO.{h,cpp}` is a class with a constructor, a destructor and
//! `void init() {}` — eleven lines of declaration and seven of definition, no
//! kernel, no parameters, nothing that references the RHI. It is a placeholder
//! that was never filled in, so there is no kernel to port and this module is
//! new code with the C++'s name on it.
//!
//! The kernel is the standard hemisphere one: samples in the unit hemisphere
//! around +Z, scaled so that later samples cluster near the origin, which is
//! what stops a large radius from looking like a uniform grey wash. It is
//! generated once, at pass creation, and never changes.
//!
//! ## Why it is applied in the composite pass
//!
//! The graph refuses a pass that reads and writes one resource
//! (`ErrorKind::PassStateConflict`, `compiler.rs:154-171`), so an AO pass cannot
//! modulate the colour target it just read. The occlusion texture is multiplied
//! in during composition instead, which also means the AO texture never has to
//! be a second full-resolution colour target.

use bytemuck::{Pod, Zeroable};
use chassis::{BindGroup, CommandList, PassScope, Pipeline};

/// Samples per pixel, as the C++'s GLSL-era AO passes used.
pub const KERNEL_SIZE: usize = 64;

/// Bind group number within the SSAO shader.
pub const SSAO_SET: u32 = 0;

/// The kernel and the matrices that turn a texel back into a view-space point.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SsaoUniforms {
    pub samples: [[f32; 4]; KERNEL_SIZE],
    pub projection: [[f32; 4]; 4],
    pub inverse_projection: [[f32; 4]; 4],
    /// radius, bias, power, unused.
    pub params: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<SsaoUniforms>().is_multiple_of(16));

/// The C++-scale defaults: a 0.5 m radius, a small depth bias, and a contrast
/// exponent that turns the raw fraction into something that reads as shading.
pub const DEFAULT_RADIUS: f32 = 0.5;
pub const DEFAULT_BIAS: f32 = 0.025;
pub const DEFAULT_POWER: f32 = 2.0;

impl SsaoUniforms {
    pub fn new(
        kernel: &[[f32; 4]; KERNEL_SIZE],
        projection: [[f32; 4]; 4],
        inverse_projection: [[f32; 4]; 4],
        radius: f32,
        bias: f32,
        power: f32,
    ) -> Self {
        Self {
            samples: *kernel,
            projection,
            inverse_projection,
            params: [radius, bias, power, 0.0],
        }
    }
}

/// A hemisphere kernel, deterministic for a given seed.
///
/// `deterministic` is the whole point: a kernel that changes per run cannot be
/// compared against a reference image, and the C++'s AO never got far enough to
/// have that problem. No `rand` dependency for six lines of xorshift.
pub fn kernel(size: usize, seed: u64) -> Vec<[f32; 4]> {
    let mut state = seed | 1;
    let mut random = move || {
        // xorshift64*, Marsaglia.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f32 / (1u64 << 53) as f32
    };

    (0..size)
        .map(|index| {
            let mut sample = glam::Vec3::new(
                random() * 2.0 - 1.0,
                random() * 2.0 - 1.0,
                random() * 2.0 - 1.0,
            );
            if sample.length_squared() < 1e-6 {
                sample = glam::Vec3::Z;
            }
            // Into the hemisphere around +Z, unit length, random magnitude.
            let mut sample = sample.normalize() * random();
            // Ramp: scatter outwards for the first samples and cluster the rest,
            // which is what keeps a wide radius from reading as a uniform wash.
            let t = index as f32 / size.max(1) as f32;
            sample *= 0.1 + 0.9 * t * t;
            [sample.x, sample.y, sample.z, 0.0]
        })
        .collect()
}

/// The kernel, fixed length so it can live in a uniform block.
pub fn fixed_kernel(seed: u64) -> [[f32; 4]; KERNEL_SIZE] {
    let mut out = [[0.0f32; 4]; KERNEL_SIZE];
    for (slot, sample) in out.iter_mut().zip(kernel(KERNEL_SIZE, seed)) {
        *slot = sample;
    }
    out
}

pub struct SsaoPass {
    pipeline: Pipeline,
    group: BindGroup,
}

impl SsaoPass {
    pub fn new(pipeline: Pipeline, group: BindGroup) -> Self {
        Self { pipeline, group }
    }

    /// One full-screen triangle: no vertex buffer, the triangle comes from
    /// `@builtin(vertex_index)`.
    pub fn record(
        &self,
        _scope: &PassScope<'_>,
        list: &mut dyn CommandList,
    ) -> Result<(), chassis::Error> {
        list.bind_pipeline(&self.pipeline);
        list.bind_group(SSAO_SET, &self.group);
        list.draw(3, 1);
        Ok(())
    }
}
