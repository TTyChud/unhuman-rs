//! The uniform blocks the shaders read, and the buffers they live in.
//!
//! The C++ carried all of this in a 192-byte push-constant block
//! (`Renderer3D.cpp:352-378`) plus a per-frame material region. **Push constants
//! are not usable through this RHI**: `PipelineDesc` has no push-constant range,
//! `VulkanPipelineState.rs:222` declares `push_constant_ranges(&[])` so any
//! `vkCmdPushConstants` is a validation error, and wgpu's
//! `CommandList::set_push_constants` is `unimplemented!()` -- it panics rather
//! than pushing anything. The renderer never calls it. So the same values
//! travel as uniform buffers, and a draw's block is bound through a descriptor
//! set rather than pushed.
//!
//! `shortcut: each frame's uniform buffers and their bind groups are re-created`
//! `because Resources has no update-in-place call (create_buffer takes initial`
//! `contents and that is all). One upload_buffer, or a push-constant range in`
//! `PipelineDesc, removes this; until then a frame costs one small allocation`
//! `per draw and the bind groups are handed back with destroy_bind_group, which`
//! `the Vulkan backend currently ignores.`

use bytemuck::{Pod, Zeroable};
use chassis::{
    BindGroup, Buffer, BufferDesc, BufferUsage, DescriptorSetLayoutDesc, ImageView, Resources,
    Sampler,
};

/// Byte alignment between blocks in one buffer.
///
/// `VkPhysicalDeviceLimits::minUniformBufferOffsetAlignment` is 256 on every
/// desktop driver and 64 on some mobile ones; the RHI does not expose it, so the
/// larger of the two is used rather than a value that would be rejected where
/// the limit is 256.
pub const UNIFORM_ALIGNMENT: u64 = 256;

/// Set 0's uniform: everything that is the same for every draw in the frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FrameUniforms {
    pub view_projection: [[f32; 4]; 4],
    pub shadow_view_projection: [[f32; 4]; 4],
    pub camera_position: [f32; 4],
    /// `x` = light count, `y` = lighting enabled (1/0), `z`/`w` unused.
    pub params: [f32; 4],
}

impl FrameUniforms {
    pub fn new(
        view_projection: [[f32; 4]; 4],
        shadow_view_projection: [[f32; 4]; 4],
        camera_position: [f32; 3],
        light_count: usize,
        lighting: bool,
    ) -> Self {
        Self {
            view_projection,
            shadow_view_projection,
            camera_position: [
                camera_position[0],
                camera_position[1],
                camera_position[2],
                1.0,
            ],
            params: [
                light_count as f32,
                if lighting { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ],
        }
    }
}

/// Set 0 of the prepass: the light-space matrix the depth is measured in.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ShadowUniforms {
    pub light_view_projection: [[f32; 4]; 4],
}

impl ShadowUniforms {
    pub fn new(light_view_projection: [[f32; 4]; 4]) -> Self {
        Self {
            light_view_projection,
        }
    }
}

/// Set 2's uniform: everything that changes per draw.
///
/// The C++'s push constants also carried the buffer *indices* of the material,
/// light and bone arrays, because it addressed them bindlessly. There is nothing
/// to index here — the bind groups are the addressing — so what remains is the
/// model matrix, the entity id and the tint.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct DrawUniforms {
    pub model: [[f32; 4]; 4],
    /// The entity's tint, which `MeshSample` carries and the C++ did not have.
    pub tint: [f32; 4],
    /// `x` = entity id, `y` = use vertex colour, `z`/`w` unused.
    pub params: [f32; 4],
}

impl DrawUniforms {
    pub fn new(model: [[f32; 4]; 4], tint: [f32; 4], entity_id: i32) -> Self {
        Self {
            model,
            tint,
            params: [entity_id as f32, 0.0, 0.0, 0.0],
        }
    }
}

/// One descriptor set whose contents are uploaded as a unit: a uniform block,
/// plus whatever else its layout binds.
///
/// Rebuilt rather than written into, for the reason in this module's header.
/// The previous buffer and group are released with the destroy calls before the
/// new ones are made, so a frame holds one of each per set rather than one per
/// frame for the process's life.
pub struct BindSet {
    buffer: Option<Buffer>,
    group: Option<BindGroup>,
}

impl BindSet {
    pub fn new() -> Self {
        Self {
            buffer: None,
            group: None,
        }
    }

    /// Uploads the block and binds it together with `buffers`, `images` and
    /// `samplers`, addressed by binding number.
    pub fn rebuild<T: Pod>(
        &mut self,
        resources: &mut dyn Resources,
        layout: &DescriptorSetLayoutDesc,
        value: &T,
        buffers: &[(u32, Buffer, u64)],
        images: &[(u32, ImageView)],
        samplers: &[(u32, Sampler)],
    ) -> Result<BindGroup, chassis::Error> {
        self.destroy(resources);

        let bytes = bytemuck::bytes_of(value);
        let buffer = resources.create_buffer(
            BufferDesc::new(bytes.len() as u64, BufferUsage::UNIFORM),
            Some(bytes),
        )?;

        // The set's own uniform is binding 0, which is what every layout here
        // declares: the C++'s push-constant block, in the one place a set can
        // put it.
        let mut entries = vec![(0u32, buffer, 0u64)];
        entries.extend_from_slice(buffers);

        let group = resources.create_bind_group(layout, &entries, images, samplers)?;
        self.buffer = Some(buffer);
        self.group = Some(group);
        Ok(group)
    }

    pub fn group(&self) -> Option<BindGroup> {
        self.group
    }

    pub fn destroy(&mut self, resources: &mut dyn Resources) {
        if let Some(group) = self.group.take() {
            resources.destroy_bind_group(group);
        }
        if let Some(buffer) = self.buffer.take() {
            resources.destroy_buffer(buffer);
        }
    }
}

impl Default for BindSet {
    fn default() -> Self {
        Self::new()
    }
}

/// A descriptor set with no uniform block at all: only images and samplers.
///
/// The composite pass is the one that needs this — it reads two textures and
/// uploads nothing — and it is separate from [`BindSet`] because a `BindSet`
/// always allocates a uniform buffer for binding 0, which a layout without that
/// binding cannot accept.
pub struct ImageSet {
    group: Option<BindGroup>,
}

impl ImageSet {
    pub fn new() -> Self {
        Self { group: None }
    }

    pub fn rebuild(
        &mut self,
        resources: &mut dyn Resources,
        layout: &DescriptorSetLayoutDesc,
        images: &[(u32, ImageView)],
        samplers: &[(u32, Sampler)],
    ) -> Result<BindGroup, chassis::Error> {
        self.destroy(resources);
        let group = resources.create_bind_group(layout, &[], images, samplers)?;
        self.group = Some(group);
        Ok(group)
    }

    pub fn group(&self) -> Option<BindGroup> {
        self.group
    }

    pub fn destroy(&mut self, resources: &mut dyn Resources) {
        if let Some(group) = self.group.take() {
            resources.destroy_bind_group(group);
        }
    }
}

impl Default for ImageSet {
    fn default() -> Self {
        Self::new()
    }
}

/// One buffer holding a block per draw, and the group that binds each block.
///
/// The C++'s per-draw data was a push-constant block, which needs no buffer at
/// all. This is the closest the RHI allows: one buffer created once per frame
/// with every draw's block in it, and one group per block, each with its byte
/// offset baked in — there are no dynamic descriptor offsets either.
pub struct DrawArena {
    buffer: Option<Buffer>,
    groups: Vec<BindGroup>,
    stride: u64,
}

impl DrawArena {
    pub fn new() -> Self {
        Self {
            buffer: None,
            groups: Vec::new(),
            stride: 0,
        }
    }

    /// Uploads one block per draw, in order. `groups()` then matches `blocks`.
    pub fn begin(
        &mut self,
        resources: &mut dyn Resources,
        layout: &DescriptorSetLayoutDesc,
        blocks: &[Vec<u8>],
    ) -> Result<usize, chassis::Error> {
        self.destroy(resources);

        let stride = align_up(
            blocks
                .iter()
                .map(|block| block.len() as u64)
                .max()
                .unwrap_or(0),
            UNIFORM_ALIGNMENT,
        );
        self.stride = stride;

        let mut bytes = vec![0u8; stride as usize * blocks.len()];
        for (index, block) in blocks.iter().enumerate() {
            let start = index * stride as usize;
            bytes[start..start + block.len()].copy_from_slice(block);
        }

        let buffer = resources.create_buffer(
            BufferDesc::new(bytes.len() as u64, BufferUsage::UNIFORM),
            Some(&bytes),
        )?;

        for index in 0..blocks.len() {
            let offset = index as u64 * stride;
            let group = resources.create_bind_group(layout, &[(0, buffer, offset)], &[], &[])?;
            self.groups.push(group);
        }

        self.buffer = Some(buffer);
        Ok(self.groups.len())
    }

    pub fn group(&self, index: usize) -> Option<BindGroup> {
        self.groups.get(index).copied()
    }

    pub fn len(&self) -> usize {
        self.groups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn stride(&self) -> u64 {
        self.stride
    }

    pub fn destroy(&mut self, resources: &mut dyn Resources) {
        for group in self.groups.drain(..) {
            resources.destroy_bind_group(group);
        }
        if let Some(buffer) = self.buffer.take() {
            resources.destroy_buffer(buffer);
        }
    }
}

impl Default for DrawArena {
    fn default() -> Self {
        Self::new()
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}
