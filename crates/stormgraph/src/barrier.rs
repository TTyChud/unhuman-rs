use crate::types::{Access, BufferId, ImageState, Stage, TextureId};

/// A texture state transition, in backend-neutral terms.
///
/// The C++ compiler writes `VkImageMemoryBarrier` here
/// (`VulkanRenderGraphCompiler.cpp:406-476`). Emitting this instead keeps the
/// compiler testable without a device, and leaves exactly one place in
/// stormbreaker that turns a transition into `vkCmdPipelineBarrier2` or
/// `vkCmdPipelineBarrier` depending on `SyncTier`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageBarrier {
    pub texture: TextureId,
    pub src_state: ImageState,
    pub dst_state: ImageState,
    pub src_access: Access,
    pub dst_access: Access,
    pub src_stage: Stage,
    pub dst_stage: Stage,
}

impl ImageBarrier {
    /// True when the transition is a no-op, so the backend can drop it.
    pub fn is_empty(&self) -> bool {
        self.src_state == self.dst_state && self.src_access == self.dst_access
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BufferBarrier {
    pub buffer: BufferId,
    pub src_access: Access,
    pub dst_access: Access,
    pub src_stage: Stage,
    pub dst_stage: Stage,
    pub offset: u64,
    pub size: u64,
}

/// Access scope implied by a state, for the *destination* side of a barrier.
pub fn access_for_state(state: ImageState) -> Access {
    match state {
        ImageState::Undefined => Access::None,
        ImageState::ColorAttachment => Access::ColorWrite,
        ImageState::DepthAttachment => Access::DepthWrite,
        ImageState::ShaderRead => Access::ShaderRead,
        ImageState::Storage => Access::ShaderWrite,
        ImageState::TransferSrc => Access::TransferRead,
        ImageState::TransferDst => Access::TransferWrite,
        ImageState::Present => Access::MemoryRead,
    }
}

/// Pipeline stage implied by a state, for the destination side.
pub fn stage_for_state(state: ImageState) -> Stage {
    match state {
        ImageState::Undefined => Stage::TopOfPipe,
        ImageState::ColorAttachment => Stage::ColorAttachmentOutput,
        ImageState::DepthAttachment => Stage::EarlyFragmentTests,
        ImageState::ShaderRead => Stage::FragmentShader,
        ImageState::Storage => Stage::ComputeShader,
        ImageState::TransferSrc | ImageState::TransferDst => Stage::Transfer,
        ImageState::Present => Stage::BottomOfPipe,
    }
}

/// Stages a pass must wait on for everything it reads.
///
/// Unlike the destination side, this is a *union* over the pass's reads: a pass
/// cannot start drawing until every texture it samples is available. Getting
/// this wrong is the classic missing-barrier bug, so it is computed in one
/// place rather than left to each pass author.
pub fn src_stage_for_state(state: ImageState) -> Stage {
    stage_for_state(state)
}

/// A resource's transition between two states, or `None` when nothing is needed.
pub fn transition(
    texture: TextureId,
    src: ImageState,
    dst: ImageState,
) -> Option<ImageBarrier> {
    // Leaving Undefined never needs a real source scope: there is nothing to
    // wait for, and content is undefined by definition.
    let (src_access, src_stage) = if src.is_defined() {
        (access_for_state(src), src_stage_for_state(src))
    } else {
        (Access::None, Stage::TopOfPipe)
    };

    let barrier = ImageBarrier {
        texture,
        src_state: src,
        dst_state: dst,
        src_access,
        dst_access: access_for_state(dst),
        src_stage,
        dst_stage: stage_for_state(dst),
    };

    (!barrier.is_empty()).then_some(barrier)
}