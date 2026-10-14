//! The RHI interface. Vulkan's problem is not that it was abstracted, it is
//! that the abstraction was hollow: one implementation, `void*` handles
//! (`UHE/src/UHE/RHI/RHITypes.h:37-41`), and the "backend-agnostic" layer
//! `dynamic_cast` straight through it (`UHE/src/UHE/ImGui/ImGuiLayer.cpp:11`).
//!
//! The test that keeps this crate honest: **no Vulkan type appears in any
//! signature outside the backend.** That is enforced here by the dependency
//! list being empty, not by discipline.

/// The render graph: declaration, versioning, compilation.
///
/// Backend-neutral by construction -- nothing here imports `ash` or any driver,
/// which is what lets both `UHE/Stormbreaker/*` backends use the same one.
pub mod graph;

pub mod backend;
mod desc;
mod format;
mod handle;
mod rhi;

pub use backend::{
    flag_value, BackendChoice, BackendSelection, BackendSource, UnknownBackend,
    ENVIRONMENT_VARIABLE as BACKEND_ENVIRONMENT_VARIABLE, FLAG as BACKEND_FLAG,
};
pub use desc::{
    BindingDesc, BlendState, BufferDesc, ColorAttachmentDesc, ColorTargetState,
    DepthAttachmentDesc, DepthState, DescriptorKind, DescriptorSetLayoutDesc, Extent, IndexFormat,
    LoadOp, PipelineDesc, PrimitiveState, RenderPassDesc, ShaderStage, ShaderStageDesc, StageMask,
    TextureDesc, VertexAttribute, VertexLayout,
};
pub use format::{BufferUsage, SampleCount, TextureFormat, TextureUsage};
pub use handle::{
    BindGroup, BindGroupTag, Buffer, BufferTag, Handle, ImageView, ImageViewTag, Pipeline,
    PipelineTag, Registry, Shader, ShaderTag, Texture, TextureTag,
};
pub use rhi::{
    CommandList, Error, Extent2d, Gpu, GpuInfo, PassId, RenderGraph, Resources, Result, Swapchain,
    SyncTier,
};
