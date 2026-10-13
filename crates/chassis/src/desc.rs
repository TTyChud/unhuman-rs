use crate::format::{BufferUsage, SampleCount, TextureFormat, TextureUsage};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Extent {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

impl Extent {
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            depth: 1,
        }
    }

    pub const fn new_3d(width: u32, height: u32, depth: u32) -> Self {
        Self {
            width,
            height,
            depth,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BufferDesc {
    pub size: u64,
    pub usage: BufferUsage,
}

impl BufferDesc {
    pub const fn new(size: u64, usage: BufferUsage) -> Self {
        Self { size, usage }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextureDesc {
    pub extent: Extent,
    pub format: TextureFormat,
    pub usage: TextureUsage,
    pub mip_levels: u32,
    pub samples: SampleCount,
    /// Initial contents. The backend is free to skip the copy when `None`.
    pub initial_data: Option<Vec<u8>>,
}

impl TextureDesc {
    pub fn new_2d(extent: Extent, format: TextureFormat, usage: TextureUsage) -> Self {
        Self {
            extent,
            format,
            usage,
            mip_levels: 1,
            samples: SampleCount::X1,
            initial_data: None,
        }
    }

    pub fn with_mips(mut self, levels: u32) -> Self {
        self.mip_levels = levels.max(1);
        self
    }

    pub fn with_samples(mut self, samples: SampleCount) -> Self {
        self.samples = samples;
        self
    }

    /// Bytes needed for mip level 0. Mips are derived, not stored.
    pub fn level0_size(&self) -> Option<u64> {
        let bytes = self.format.bytes_per_texel()? as u64;
        let texels =
            self.extent.width as u64 * self.extent.height as u64 * self.extent.depth.max(1) as u64;
        Some(texels * bytes)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IndexFormat {
    U16,
    U32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VertexAttribute {
    pub location: u32,
    pub format: TextureFormat,
    pub offset: u32,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct VertexLayout {
    pub stride: u32,
    pub attributes: Vec<VertexAttribute>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PrimitiveState {
    pub topology: u32,
    pub cull_mode: u32,
    pub front_face: u32,
}

impl Default for PrimitiveState {
    fn default() -> Self {
        Self {
            topology: 4,  // triangle list
            cull_mode: 1,  // back
            front_face: 0, // counter-clockwise
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DepthState {
    pub format: Option<TextureFormat>,
    pub write_enabled: bool,
    pub compare: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColorTargetState {
    pub format: TextureFormat,
    pub blend: bool,
    pub write_mask: u8,
}

impl Default for ColorTargetState {
    fn default() -> Self {
        Self {
            format: TextureFormat::Undefined,
            blend: false,
            write_mask: 0xF,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct BlendState {
    pub enabled: bool,
}

/// A complete graphics pipeline description.
///
/// One struct rather than a builder: a second backend either consumes this or
/// does not exist yet, and a builder here would be guessing at the shape of a
/// backend nobody has written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PipelineDesc {
    pub label: String,
    pub shader: crate::handle::Shader,
    pub vertex: VertexLayout,
    pub index_format: Option<IndexFormat>,
    pub primitive: PrimitiveState,
    pub depth: DepthState,
    pub blend: BlendState,
    pub color_targets: Vec<ColorTargetState>,
    pub samples: SampleCount,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ColorAttachmentDesc {
    pub format: TextureFormat,
    pub load: LoadOp,
    pub store: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadOp {
    Load,
    Clear,
    /// Matches the C++ habit of hardcoding `oldLayout = eUndefined` on entry
    /// (`VulkanCommandBuffer.cpp:104,153`). Now it is a choice the pass makes.
    DontCare,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DepthAttachmentDesc {
    pub format: TextureFormat,
    pub load: LoadOp,
    pub store: bool,
}

/// Attachments a pass writes during one `begin_rendering` scope.
///
/// This is the shape a frame graph pass sees. It carries no layouts: layout
/// transitions and barriers are the render graph's job, derived from the
/// declared read/write sets. That is the property that keeps
/// `VulkanRenderGraphExecutor.h:106-134` — passes get no barrier API — true on
/// the Rust side too.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RenderPassDesc {
    pub label: String,
    pub colors: Vec<ColorAttachmentDesc>,
    pub depth: Option<DepthAttachmentDesc>,
}

impl RenderPassDesc {
    pub fn color(format: TextureFormat) -> Self {
        Self {
            label: String::new(),
            colors: vec![ColorAttachmentDesc {
                format,
                load: LoadOp::Clear,
                store: true,
            }],
            depth: None,
        }
    }

    pub fn with_depth(mut self, format: TextureFormat) -> Self {
        self.depth = Some(DepthAttachmentDesc {
            format,
            load: LoadOp::Clear,
            store: true,
        });
        self
    }

    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}