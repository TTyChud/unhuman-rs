//! The render graph: declaration, versioning and compilation.
//!
//! Ported from `UHE/src/Platform/Vulkan/RenderGraph/`, with one structural
//! change: **this crate emits backend-neutral barriers.** The C++ compiler
//! writes `VkImageMemoryBarrier` directly
//! (`VulkanRenderGraphCompiler.cpp:406-476`), which is why `tests/rendergraph`
//! has to compile 11 graph sources plus a `VmaDispatchStub` just to test the
//! topological sort. Here the compiler emits [`barrier::ImageBarrier`], and
//! stormbreaker lowers it — which is what makes the `SyncTier` design in the
//! C++ `docs/vulkan-sync-and-rendergraph.md` a single decision point instead of a
//! property of every barrier the graph happens to emit.
//!
//! Consequence: the whole compiler is testable with no device, no driver and no
//! Vulkan loader.

mod barrier;
mod builder;
mod compiler;
mod resources;
mod types;

pub use barrier::{access_for_state, stage_for_state, transition, BufferBarrier, ImageBarrier};
pub use builder::{PassBuilder, RenderGraph};
pub use compiler::{compile, validate, CompiledGraph, CompiledPass, ResourceLifetime};
pub use resources::{BufferSlot, Resources, TextureSlot, VersionEntry};
pub use types::{
    Access, BufferId, ColorAttachment, DepthAttachment, ErrorKind, ImageState, LoadOp, PassDesc,
    PassType, Queue, ResourceAccess, Stage, TextureId, ValidationError, NO_RESOURCE,
};