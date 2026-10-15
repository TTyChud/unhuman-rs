#!/usr/bin/env bash
# Coverage: every C++ file has a Rust counterpart, or a recorded reason it does not.
#
# The port's whole claim is "nothing is left behind", and a claim like that is
# worth exactly as much as the check behind it. This is that check: it walks the
# C++ tree and asks the table below where each file went. A file the table does
# not name fails; so does a table entry whose Rust counterpart is gone.
#
# The table is per-file and explicit on purpose. A counterpart is not derivable
# from a filename here: the port merged modules as it went, so `SceneCamera.h` is
# `scene/src/camera.rs`, `SceneSerializer.*` is `serialize.rs`, `AssetManager.*`
# is `assets.rs` and `AudioEngine.*` is `audio/src/engine.rs`. The table is also
# the port's own record of what it did, which is why it lives here rather than in
# a heuristic that guesses.
#
# The reasons are the ones written down in `docs/architecture/01-crate-map.md`
# ("Not ported") and `docs/WORK.md` ("Dead or skipped, deliberately") — a file
# does not get to disappear just because nobody wanted to port it.
#
# Table format, one line per C++ file, ` + ` separating merged destinations:
#   <cpp path>|moved|<rust path>[ + <rust path>][|<caveat>]
#   <cpp path>|skip|<reason>
#
# Usage:
#   scripts/parity.sh [path-to-cpp-tree]      # default: ../UNHUMAN
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

CPP_ROOT="${1:-../UNHUMAN}"
if [[ ! -d "$CPP_ROOT" ]]; then
  echo "parity: no C++ tree at $CPP_ROOT (nothing to check against)" >&2
  exit 0
fi

# The C++ files this check covers: the engine, the platform layer, the editor and
# the game, plus the translation units at the top of `UHE/src` (`uhepch.{cpp,h}`,
# `UHE.h`), which belong to no directory and were the walk's blind spot.
cpp_files() {
  find "$CPP_ROOT/UHE/src/UHE" "$CPP_ROOT/UHE/src/Platform" \
    "$CPP_ROOT/UHE_EDITOR/src" "$CPP_ROOT/UHEGAME/src" \
    -type f \( -name '*.cpp' -o -name '*.h' -o -name '*.hpp' \)
  find "$CPP_ROOT/UHE/src" -maxdepth 1 -type f \( -name '*.cpp' -o -name '*.h' -o -name '*.hpp' \)
}

parity_table() {
  cat <<'MAP'
UHE/src/Platform/Linux/Linux.cpp|skip|Platform/Linux: winit replaces both platform layers; the vestigial Linux layer never ran (`docs/architecture/01-crate-map.md`)
UHE/src/Platform/Linux/Linux.h|skip|Platform/Linux: winit replaces both platform layers; the vestigial Linux layer never ran (`docs/architecture/01-crate-map.md`)
UHE/src/Platform/Linux/LinuxPlatformUtils.cpp|skip|Platform/Linux: winit replaces both platform layers; the vestigial Linux layer never ran (`docs/architecture/01-crate-map.md`)
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraph.cpp|moved|UHE/src/rhi/src/graph/mod.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraph.h|moved|UHE/src/rhi/src/graph/mod.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphBuilder.cpp|moved|UHE/src/rhi/src/graph/builder.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphBuilder.h|moved|UHE/src/rhi/src/graph/builder.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphCompiler.cpp|moved|UHE/src/rhi/src/graph/compiler.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphCompiler.h|moved|UHE/src/rhi/src/graph/compiler.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphExecutor.cpp|moved|UHE/Stormbreaker/vulkan/src/RenderGraph/VulkanRenderGraphExecutor.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphExecutor.h|moved|UHE/Stormbreaker/vulkan/src/RenderGraph/VulkanRenderGraphExecutor.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphResources.cpp|moved|UHE/Stormbreaker/vulkan/src/RenderGraph/VulkanRenderGraphResources.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphResources.h|moved|UHE/Stormbreaker/vulkan/src/RenderGraph/VulkanRenderGraphResources.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphTypes.cpp|moved|UHE/src/rhi/src/graph/types.rs
UHE/src/Platform/Vulkan/RenderGraph/VulkanRenderGraphTypes.h|moved|UHE/src/rhi/src/graph/types.rs
UHE/src/Platform/Vulkan/ShaderSystem/VulkanShaderManager.cpp|skip|`Init()` and `ShutDown()` are both empty bodies (`docs/WORK.md`)
UHE/src/Platform/Vulkan/ShaderSystem/VulkanShaderManager.h|skip|`Init()` and `ShutDown()` are both empty bodies (`docs/WORK.md`)
UHE/src/Platform/Vulkan/UI/VulkanImGuiBackend.cpp|moved|UHE/bins/editor/src/main.rs + UHE/src/editor/src/lib.rs
UHE/src/Platform/Vulkan/UI/VulkanImGuiBackend.h|moved|UHE/bins/editor/src/main.rs + UHE/src/editor/src/lib.rs
UHE/src/Platform/Vulkan/UI/VulkanImGuiPass.cpp|moved|UHE/bins/editor/src/main.rs + UHE/src/editor/src/lib.rs
UHE/src/Platform/Vulkan/UI/VulkanImGuiPass.h|moved|UHE/bins/editor/src/main.rs + UHE/src/editor/src/lib.rs
UHE/src/Platform/Vulkan/VulkanBarrierEncoder.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanBarrierEncoder.rs
UHE/src/Platform/Vulkan/VulkanBarrierEncoder.h|moved|UHE/Stormbreaker/vulkan/src/VulkanBarrierEncoder.rs
UHE/src/Platform/Vulkan/VulkanBinaryFence.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanFence.rs
UHE/src/Platform/Vulkan/VulkanBinaryFence.h|moved|UHE/Stormbreaker/vulkan/src/VulkanFence.rs
UHE/src/Platform/Vulkan/VulkanBinarySemaphore.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanSemaphore.rs
UHE/src/Platform/Vulkan/VulkanBinarySemaphore.h|moved|UHE/Stormbreaker/vulkan/src/VulkanSemaphore.rs
UHE/src/Platform/Vulkan/VulkanBuffer.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanBuffer.rs
UHE/src/Platform/Vulkan/VulkanBuffer.h|moved|UHE/Stormbreaker/vulkan/src/VulkanBuffer.rs
UHE/src/Platform/Vulkan/VulkanCommandBuffer.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanCommandBuffer.rs
UHE/src/Platform/Vulkan/VulkanCommandBuffer.h|moved|UHE/Stormbreaker/vulkan/src/VulkanCommandBuffer.rs
UHE/src/Platform/Vulkan/VulkanCommandPool.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanCommandPool.rs
UHE/src/Platform/Vulkan/VulkanCommandPool.h|moved|UHE/Stormbreaker/vulkan/src/VulkanCommandPool.rs
UHE/src/Platform/Vulkan/VulkanComputePipeline.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanComputePipeline.rs
UHE/src/Platform/Vulkan/VulkanComputePipeline.h|moved|UHE/Stormbreaker/vulkan/src/VulkanComputePipeline.rs
UHE/src/Platform/Vulkan/VulkanContext.h|moved|UHE/Stormbreaker/vulkan/src/VulkanContext.rs
UHE/src/Platform/Vulkan/VulkanDescriptorManager.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanDescriptorManager.rs
UHE/src/Platform/Vulkan/VulkanDescriptorManager.h|moved|UHE/Stormbreaker/vulkan/src/VulkanDescriptorManager.rs
UHE/src/Platform/Vulkan/VulkanDescriptorPool.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanDescriptorPool.rs
UHE/src/Platform/Vulkan/VulkanDescriptorPool.h|moved|UHE/Stormbreaker/vulkan/src/VulkanDescriptorPool.rs
UHE/src/Platform/Vulkan/VulkanDescriptorSet.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanDescriptorSet.rs
UHE/src/Platform/Vulkan/VulkanDescriptorSet.h|moved|UHE/Stormbreaker/vulkan/src/VulkanDescriptorSet.rs
UHE/src/Platform/Vulkan/VulkanDevice.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanDevice.rs
UHE/src/Platform/Vulkan/VulkanDevice.h|moved|UHE/Stormbreaker/vulkan/src/VulkanDevice.rs
UHE/src/Platform/Vulkan/VulkanExtensionCheck.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanExtensionCheck.rs
UHE/src/Platform/Vulkan/VulkanExtensionCheck.h|moved|UHE/Stormbreaker/vulkan/src/VulkanExtensionCheck.rs
UHE/src/Platform/Vulkan/VulkanFence.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanFence.rs
UHE/src/Platform/Vulkan/VulkanFence.h|moved|UHE/Stormbreaker/vulkan/src/VulkanFence.rs
UHE/src/Platform/Vulkan/VulkanFrameContext.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanFrameContext.rs
UHE/src/Platform/Vulkan/VulkanFrameContext.h|moved|UHE/Stormbreaker/vulkan/src/VulkanFrameContext.rs
UHE/src/Platform/Vulkan/VulkanFramebuffer.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanFramebuffer.rs
UHE/src/Platform/Vulkan/VulkanFramebuffer.h|moved|UHE/Stormbreaker/vulkan/src/VulkanFramebuffer.rs
UHE/src/Platform/Vulkan/VulkanGraphicPipeline.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanGraphicPipeline.rs
UHE/src/Platform/Vulkan/VulkanGraphicPipeline.h|moved|UHE/Stormbreaker/vulkan/src/VulkanGraphicPipeline.rs
UHE/src/Platform/Vulkan/VulkanImageView.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanImageView.rs
UHE/src/Platform/Vulkan/VulkanImageView.h|moved|UHE/Stormbreaker/vulkan/src/VulkanImageView.rs
UHE/src/Platform/Vulkan/VulkanInstance.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanInstance.rs
UHE/src/Platform/Vulkan/VulkanInstance.h|moved|UHE/Stormbreaker/vulkan/src/VulkanInstance.rs
UHE/src/Platform/Vulkan/VulkanLogicalDevice.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanLogicalDevice.rs
UHE/src/Platform/Vulkan/VulkanLogicalDevice.h|moved|UHE/Stormbreaker/vulkan/src/VulkanLogicalDevice.rs
UHE/src/Platform/Vulkan/VulkanPhysicalDevice.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanPhysicalDevice.rs
UHE/src/Platform/Vulkan/VulkanPhysicalDevice.h|moved|UHE/Stormbreaker/vulkan/src/VulkanPhysicalDevice.rs
UHE/src/Platform/Vulkan/VulkanPipelineState.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanPipelineState.rs
UHE/src/Platform/Vulkan/VulkanPipelineState.h|moved|UHE/Stormbreaker/vulkan/src/VulkanPipelineState.rs
UHE/src/Platform/Vulkan/VulkanRenderPass.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanRenderPass.rs
UHE/src/Platform/Vulkan/VulkanRenderPass.h|moved|UHE/Stormbreaker/vulkan/src/VulkanRenderPass.rs
UHE/src/Platform/Vulkan/VulkanSemaphore.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanSemaphore.rs
UHE/src/Platform/Vulkan/VulkanSemaphore.h|moved|UHE/Stormbreaker/vulkan/src/VulkanSemaphore.rs
UHE/src/Platform/Vulkan/VulkanShader.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanShader.rs
UHE/src/Platform/Vulkan/VulkanShader.h|moved|UHE/Stormbreaker/vulkan/src/VulkanShader.rs
UHE/src/Platform/Vulkan/VulkanSwapChain.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanSwapChain.rs
UHE/src/Platform/Vulkan/VulkanSwapChain.h|moved|UHE/Stormbreaker/vulkan/src/VulkanSwapChain.rs
UHE/src/Platform/Vulkan/VulkanTexture.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanTexture.rs
UHE/src/Platform/Vulkan/VulkanTexture.h|moved|UHE/Stormbreaker/vulkan/src/VulkanTexture.rs
UHE/src/Platform/Vulkan/VulkanTexture2D.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanTexture2D.rs
UHE/src/Platform/Vulkan/VulkanTexture2D.h|moved|UHE/Stormbreaker/vulkan/src/VulkanTexture2D.rs
UHE/src/Platform/Vulkan/VulkanTypes.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanTypes.rs
UHE/src/Platform/Vulkan/VulkanTypes.h|moved|UHE/Stormbreaker/vulkan/src/VulkanTypes.rs
UHE/src/Platform/Vulkan/VulkanUtils.cpp|moved|UHE/Stormbreaker/vulkan/src/VulkanUtils.rs
UHE/src/Platform/Vulkan/VulkanUtils.h|moved|UHE/Stormbreaker/vulkan/src/VulkanUtils.rs
UHE/src/Platform/Vulkan/vma_usage.cpp|skip|4 lines, allocates nothing (`docs/WORK.md`)
UHE/src/Platform/Windows/WindowInput.cpp|moved|UHE/src/platform/src/input.rs
UHE/src/Platform/Windows/WindowPlatformUtils.cpp|skip|Win32 file dialog: no caller needs one — winit covers the window and its input (`UHE/src/platform/src/lib.rs`)
UHE/src/Platform/Windows/WindowsWindow.cpp|skip|winit replaces it, on both platforms, from one implementation (`docs/WORK.md`)
UHE/src/Platform/Windows/WindowsWindow.h|skip|winit replaces it, on both platforms, from one implementation (`docs/WORK.md`)
UHE/src/UHE.h|skip|umbrella include: it exists to pull the public headers into one translation unit; a Rust crate names what it uses
UHE/src/UHE/Animation/Animation2D/SpriteAnimation.cpp|moved|UHE/src/scene/src/components.rs
UHE/src/UHE/Animation/Animation2D/SpriteAnimation.h|moved|UHE/src/scene/src/components.rs
UHE/src/UHE/AssestsManager/AssetID.h|moved|UHE/src/assets/src/id.rs
UHE/src/UHE/AssestsManager/AssetManager.cpp|moved|UHE/src/assets/src/assets.rs
UHE/src/UHE/AssestsManager/AssetManager.h|moved|UHE/src/assets/src/assets.rs
UHE/src/UHE/AssestsManager/ShaderManager.cpp|skip|`Init()` and `ShutDown()` are both empty bodies; `forge` is the shader path (`docs/WORK.md`)
UHE/src/UHE/AssestsManager/ShaderManager.h|skip|`Init()` and `ShutDown()` are both empty bodies; `forge` is the shader path (`docs/WORK.md`)
UHE/src/UHE/AssestsManager/VfsSystem.cpp|moved|UHE/src/assets/src/vfs.rs
UHE/src/UHE/AssestsManager/VfsSystem.h|moved|UHE/src/assets/src/vfs.rs
UHE/src/UHE/Audio/AudioEngine.cpp|moved|UHE/src/audio/src/engine.rs
UHE/src/UHE/Audio/AudioEngine.h|moved|UHE/src/audio/src/engine.rs
UHE/src/UHE/Audio/MiniAudioBackend.cpp|moved|UHE/src/audio/src/backend.rs
UHE/src/UHE/Audio/MiniAudioBackend.h|moved|UHE/src/audio/src/backend.rs
UHE/src/UHE/Core/Application.cpp|moved|UHE/src/core/src/app.rs
UHE/src/UHE/Core/Application.h|moved|UHE/src/core/src/app.rs
UHE/src/UHE/Core/Core.h|skip|`UHE_API`/assert/`Ref` macros and the `u32`/`f32` aliases: export and ownership scaffolding with no Rust equivalent
UHE/src/UHE/Core/EntryPoint.h|moved|UHE/bins/sandbox/src/main.rs + UHE/bins/nocap/src/main.rs
UHE/src/UHE/Core/Layer.cpp|moved|UHE/src/core/src/plugin.rs
UHE/src/UHE/Core/Layer.h|moved|UHE/src/core/src/plugin.rs
UHE/src/UHE/Core/LayerStack.cpp|moved|UHE/src/core/src/plugin.rs
UHE/src/UHE/Core/LayerStack.h|moved|UHE/src/core/src/plugin.rs
UHE/src/UHE/Core/Log.cpp|skip|spdlog: `log`/`env_logger` for logging (`docs/architecture/01-crate-map.md`)
UHE/src/UHE/Core/Log.h|skip|spdlog: `log`/`env_logger` for logging (`docs/architecture/01-crate-map.md`)
UHE/src/UHE/Core/Timestep.h|moved|UHE/src/core/src/time.rs
UHE/src/UHE/Core/UUID.cpp|moved|UHE/src/scene/src/components.rs
UHE/src/UHE/Core/UUID.h|moved|UHE/src/scene/src/components.rs
UHE/src/UHE/Core/Window.h|moved|UHE/src/platform/src/window.rs
UHE/src/UHE/Core/input.h|moved|UHE/src/platform/src/input.rs
UHE/src/UHE/Core/keyCodes.h|moved|UHE/src/events/src/key.rs
UHE/src/UHE/Core/mouseButtonCodes.h|moved|UHE/src/events/src/key.rs
UHE/src/UHE/Debug/TracySetup.cpp|skip|Tracy macros: replaced by `VulkanScopes.rs` debug-utils labels (`docs/WORK.md`)
UHE/src/UHE/Debug/TracySetup.h|skip|Tracy macros: replaced by `VulkanScopes.rs` debug-utils labels (`docs/WORK.md`)
UHE/src/UHE/Events/ApplicationEvent.h|moved|UHE/src/events/src/event.rs
UHE/src/UHE/Events/Event.h|moved|UHE/src/events/src/event.rs
UHE/src/UHE/Events/KeyEvent.h|moved|UHE/src/events/src/event.rs
UHE/src/UHE/Events/MouseEvent.h|moved|UHE/src/events/src/event.rs
UHE/src/UHE/ImGui/ImGuiBuild.cpp|skip|replaced, not dropped: the model is `mirror`, drawn by gpui in `UHE/bins/editor` (`docs/WORK.md`)
UHE/src/UHE/ImGui/ImGuiLayer.cpp|moved|UHE/bins/editor/src/main.rs
UHE/src/UHE/ImGui/ImGuiLayer.h|moved|UHE/bins/editor/src/main.rs
UHE/src/UHE/ImGui/ImGuiLogsink.h|skip|replaced, not dropped: the model is `mirror`, drawn by gpui in `UHE/bins/editor` (`docs/WORK.md`)
UHE/src/UHE/Jobsystem/Jobsystem.cpp|moved|UHE/src/jobs/src/jobs.rs
UHE/src/UHE/Jobsystem/Jobsystem.h|moved|UHE/src/jobs/src/jobs.rs
UHE/src/UHE/Jobsystem/Taskgraph.cpp|moved|UHE/src/jobs/src/taskgraph.rs
UHE/src/UHE/Jobsystem/Taskgraph.h|moved|UHE/src/jobs/src/taskgraph.rs
UHE/src/UHE/Math/Math.cpp|skip|`glam` is the workspace's math crate; `DecomposeTransform` is `Mat4::to_scale_rotation_translation`
UHE/src/UHE/Math/Math.h|skip|`glam` is the workspace's math crate; `DecomposeTransform` is `Mat4::to_scale_rotation_translation`
UHE/src/UHE/Physics/PhysicsSystem3D.cpp|moved|UHE/src/physics/src/physics_system3d.rs
UHE/src/UHE/Physics/PhysicsSystem3D.h|moved|UHE/src/physics/src/physics_system3d.rs
UHE/src/UHE/RHI/DeletionQueue.h|moved|UHE/Stormbreaker/vulkan/src/VulkanDeletionQueue.rs
UHE/src/UHE/RHI/RHICommandBuffer.cpp|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHICommandBuffer.h|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHIDevice.cpp|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHIDevice.h|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHIShader.cpp|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHIShader.h|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHISwapChain.cpp|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHISwapChain.h|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHITexture.cpp|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHITexture.h|moved|UHE/src/rhi/src/rhi.rs
UHE/src/UHE/RHI/RHITypes.h|moved|UHE/src/rhi/src/desc.rs + UHE/src/rhi/src/handle.rs + UHE/src/rhi/src/format.rs
UHE/src/UHE/Renderer/Camera.h|moved|UHE/src/renderer/src/camera.rs + UHE/src/scene/src/camera.rs
UHE/src/UHE/Renderer/EditorCamera.cpp|moved|UHE/src/renderer/src/camera.rs
UHE/src/UHE/Renderer/EditorCamera.h|moved|UHE/src/renderer/src/camera.rs
UHE/src/UHE/Renderer/Font.cpp|moved|UHE/src/renderer/src/font/generate.rs
UHE/src/UHE/Renderer/Font.h|moved|UHE/src/renderer/src/font/atlas.rs
UHE/src/UHE/Renderer/Framebuffer.cpp|moved|UHE/src/renderer/src/renderer.rs
UHE/src/UHE/Renderer/Framebuffer.h|moved|UHE/src/renderer/src/renderer.rs
UHE/src/UHE/Renderer/KTX2.cpp|moved|UHE/src/assets/src/ktx2.rs
UHE/src/UHE/Renderer/KTX2.h|moved|UHE/src/assets/src/ktx2.rs
UHE/src/UHE/Renderer/Mipmap.cpp|moved|UHE/src/assets/src/image.rs
UHE/src/UHE/Renderer/Mipmap.h|moved|UHE/src/assets/src/image.rs
UHE/src/UHE/Renderer/Renderer.cpp|moved|UHE/src/core/src/engine.rs
UHE/src/UHE/Renderer/Renderer.h|moved|UHE/src/core/src/engine.rs
UHE/src/UHE/Renderer/Renderer2D.cpp|moved|UHE/src/renderer/src/batch2d.rs + UHE/src/renderer/src/text.rs
UHE/src/UHE/Renderer/Renderer2D.h|moved|UHE/src/renderer/src/batch2d.rs
UHE/src/UHE/Renderer/Shader.cpp|skip|excluded from the C++ build (`UHE/CMakeLists.txt:13`)
UHE/src/UHE/Renderer/Shader.h|moved|UHE/src/shaders/src/request.rs
UHE/src/UHE/Renderer/SlangCompiler.cpp|moved|UHE/src/shaders/src/slangc.rs
UHE/src/UHE/Renderer/SlangCompiler.h|moved|UHE/src/shaders/src/slangc.rs
UHE/src/UHE/Renderer/Texture.cpp|moved|UHE/src/assets/src/image.rs
UHE/src/UHE/Renderer/Texture.h|moved|UHE/src/assets/src/image.rs
UHE/src/UHE/Renderer2D/OrthographicCamera.cpp|skip|2D path: excluded from the C++ build (`UHE/CMakeLists.txt:12`, `docs/WORK.md`)
UHE/src/UHE/Renderer2D/OrthographicCamera.h|skip|2D path: excluded from the C++ build (`UHE/CMakeLists.txt:12`, `docs/WORK.md`)
UHE/src/UHE/Renderer2D/OrthograpicCameraContoroller.cpp|skip|the C++ editor referenced it (`UHE_EDITOR/src/Editor.h`); the port has no 2D camera controller — a 2D view would be `meats::SceneCamera::new_orthographic` plus an input path the editor does not have yet (`mirror` and `UHE/bins/editor` read no input)
UHE/src/UHE/Renderer2D/OrthograpicCameraContoroller.h|skip|the C++ editor referenced it (`UHE_EDITOR/src/Editor.h`); the port has no 2D camera controller — a 2D view would be `meats::SceneCamera::new_orthographic` plus an input path the editor does not have yet (`mirror` and `UHE/bins/editor` read no input)
UHE/src/UHE/Renderer2D/Renderer2D.cpp|skip|2D path: excluded from the C++ build (`UHE/CMakeLists.txt:12`, `docs/WORK.md`)
UHE/src/UHE/Renderer2D/Renderer2D.h|skip|2D path: excluded from the C++ build (`UHE/CMakeLists.txt:12`, `docs/WORK.md`)
UHE/src/UHE/Renderer2D/SubTexture2D.cpp|moved|UHE/src/renderer/src/batch2d.rs|the type itself is not carried: what arrives is the four atlas corners its draw used, as `push_sprite_uv`
UHE/src/UHE/Renderer2D/SubTexture2D.h|moved|UHE/src/renderer/src/batch2d.rs|the type itself is not carried: what arrives is the four atlas corners its draw used, as `push_sprite_uv`
UHE/src/UHE/Renderer3D/Animation.h|moved|UHE/src/renderer/src/animation/clip.rs
UHE/src/UHE/Renderer3D/AnimationCompression.cpp|moved|UHE/src/renderer/src/animation/compression.rs
UHE/src/UHE/Renderer3D/AnimationCompression.h|moved|UHE/src/renderer/src/animation/compression.rs
UHE/src/UHE/Renderer3D/AnimationStateMachine.cpp|moved|UHE/src/renderer/src/animation/state_machine.rs
UHE/src/UHE/Renderer3D/AnimationStateMachine.h|moved|UHE/src/renderer/src/animation/state_machine.rs
UHE/src/UHE/Renderer3D/Animator.cpp|moved|UHE/src/renderer/src/animation/mod.rs
UHE/src/UHE/Renderer3D/Animator.h|moved|UHE/src/renderer/src/animation/mod.rs
UHE/src/UHE/Renderer3D/IKSolver.cpp|moved|UHE/src/renderer/src/ik.rs
UHE/src/UHE/Renderer3D/IKSolver.h|moved|UHE/src/renderer/src/ik.rs
UHE/src/UHE/Renderer3D/LightSystem.cpp|moved|UHE/src/renderer/src/light.rs
UHE/src/UHE/Renderer3D/LightSystem.h|moved|UHE/src/renderer/src/light.rs
UHE/src/UHE/Renderer3D/LoadModel.cpp|moved|UHE/src/assets/src/load_model.rs
UHE/src/UHE/Renderer3D/LoadModel.h|moved|UHE/src/assets/src/load_model.rs
UHE/src/UHE/Renderer3D/LoadModelAnimation.cpp|moved|UHE/src/assets/src/load_model_animation.rs
UHE/src/UHE/Renderer3D/LoadModelGeometry.cpp|moved|UHE/src/assets/src/load_model_geometry.rs
UHE/src/UHE/Renderer3D/LoadModelUpload.cpp|moved|UHE/src/renderer/src/mesh.rs
UHE/src/UHE/Renderer3D/MaterialExtensions.h|moved|UHE/src/renderer/src/material.rs
UHE/src/UHE/Renderer3D/MaterialGPU.cpp|moved|UHE/src/renderer/src/material.rs
UHE/src/UHE/Renderer3D/MaterialGPU.h|moved|UHE/src/renderer/src/material.rs
UHE/src/UHE/Renderer3D/MeshoptDecode.cpp|skip|meshoptimizer is a C library the crates do not link; a file declaring `EXT_meshopt_compression` is refused by name (`UHE/src/assets/src/load_model.rs`)
UHE/src/UHE/Renderer3D/MeshoptDecode.h|skip|meshoptimizer is a C library the crates do not link; a file declaring `EXT_meshopt_compression` is refused by name (`UHE/src/assets/src/load_model.rs`)
UHE/src/UHE/Renderer3D/Renderer3D.cpp|moved|UHE/src/renderer/src/renderer.rs
UHE/src/UHE/Renderer3D/Renderer3D.h|moved|UHE/src/renderer/src/renderer.rs
UHE/src/UHE/Renderer3D/SSAO.cpp|moved|UHE/src/renderer/src/passes/ssao.rs
UHE/src/UHE/Renderer3D/SSAO.h|moved|UHE/src/renderer/src/passes/ssao.rs
UHE/src/UHE/Renderer3D/Shape.h|skip|a declaration with no body and no caller; the collider shapes are `femur`'s (`UHE/src/physics/src/shape.rs`)
UHE/src/UHE/Scene/Components.h|moved|UHE/src/scene/src/components.rs
UHE/src/UHE/Scene/Entity.cpp|moved|UHE/src/scene/src/entity.rs
UHE/src/UHE/Scene/Entity.h|moved|UHE/src/scene/src/entity.rs
UHE/src/UHE/Scene/Scene.cpp|moved|UHE/src/scene/src/scene.rs
UHE/src/UHE/Scene/Scene.h|moved|UHE/src/scene/src/scene.rs
UHE/src/UHE/Scene/SceneCamera.cpp|moved|UHE/src/scene/src/camera.rs
UHE/src/UHE/Scene/SceneCamera.h|moved|UHE/src/scene/src/camera.rs
UHE/src/UHE/Scene/SceneSerializer.cpp|moved|UHE/src/scene/src/serialize.rs
UHE/src/UHE/Scene/SceneSerializer.h|moved|UHE/src/scene/src/serialize.rs
UHE/src/UHE/Scene/ScriptableEntity.h|moved|UHE/src/scene/src/components.rs
UHE/src/UHE/Test/TestRenderer.cpp|skip|declarations with no bodies; `cargo test` replaces them (`docs/WORK.md`)
UHE/src/UHE/Test/TestRenderer.h|skip|declarations with no bodies; `cargo test` replaces them (`docs/WORK.md`)
UHE/src/UHE/Test/TestVulkanBackend.cpp|skip|declarations with no bodies; `cargo test` replaces them (`docs/WORK.md`)
UHE/src/UHE/Test/TestVulkanBackend.h|skip|declarations with no bodies; `cargo test` replaces them (`docs/WORK.md`)
UHE/src/UHE/Utils/ObjectPool.h|skip|an empty file (0 bytes): the generational handle table it was reserved for is `vault`'s (`UHE/src/assets/src/id.rs`)
UHE/src/UHE/Utils/PlatfromUtils.h|skip|Win32 `FileDialogs`: no caller needs one, and nothing in a window is platform code once `winit` exists (`UHE/src/platform/src/lib.rs`)
UHE/src/UHE/Utils/YamlUtils.h|moved|UHE/src/scene/src/serialize.rs
UHE/src/uhepch.cpp|skip|precompiled header: Rust has no precompiled-header translation unit, and the crate boundaries are the include graph
UHE/src/uhepch.h|skip|precompiled header: Rust has no precompiled-header translation unit, and the crate boundaries are the include graph
UHEGAME/src/AimLabLayer.cpp|moved|UHE/games/aimlab/src/lib.rs + UHE/games/aimlab/src/rules.rs
UHEGAME/src/AimLabLayer.h|moved|UHE/games/aimlab/src/lib.rs
UHEGAME/src/GameApp.cpp|moved|UHE/games/aimlab/src/lib.rs
UHE_EDITOR/src/Editor.cpp|moved|UHE/src/editor/src/lib.rs
UHE_EDITOR/src/Editor.h|moved|UHE/src/editor/src/lib.rs
UHE_EDITOR/src/EditorTheme.cpp|moved|UHE/src/editor/src/theme.rs
UHE_EDITOR/src/EditorTheme.h|moved|UHE/src/editor/src/theme.rs
UHE_EDITOR/src/Panel/ContentBrowserPanel.cpp|moved|UHE/src/editor/src/content_browser.rs + UHE/bins/editor/src/panels/content_browser.rs
UHE_EDITOR/src/Panel/ContentBrowserPanel.h|moved|UHE/src/editor/src/content_browser.rs + UHE/bins/editor/src/panels/content_browser.rs
UHE_EDITOR/src/Panel/SceneHierachyPanel.cpp|moved|UHE/src/editor/src/hierarchy.rs + UHE/bins/editor/src/panels/hierarchy.rs
UHE_EDITOR/src/Panel/SceneHierachyPanel.h|moved|UHE/src/editor/src/hierarchy.rs + UHE/bins/editor/src/panels/hierarchy.rs
UHE_EDITOR/src/app.cpp|moved|UHE/bins/editor/src/main.rs
MAP
}

declare -A table_kind table_target table_note
while IFS='|' read -r cpp kind target note; do
  [[ -z "$cpp" ]] && continue
  if [[ -n "${table_kind[$cpp]:-}" ]]; then
    echo "parity: duplicate table entry for $cpp" >&2
    exit 2
  fi
  table_kind["$cpp"]="$kind"
  table_target["$cpp"]="$target"
  table_note["$cpp"]="$note"
done < <(parity_table)

declare -A seen
moved=0
skipped=0
missing=0
unaccounted=()

while IFS= read -r path; do
  rel="${path#"$CPP_ROOT"/}"
  seen["$rel"]=1

  case "${table_kind[$rel]:-}" in
    moved)
      target="${table_target[$rel]}"
      note="${table_note[$rel]}"
      for rust in ${target// + / }; do
        if [[ ! -e "$rust" ]]; then
          echo "parity: $rel says it went to $rust, which does not exist" >&2
          exit 2
        fi
      done
      moved=$((moved + 1))
      printf '  moved  %-56s -> %s%s\n' "$rel" "$target" "${note:+ ($note)}"
      ;;
    skip)
      skipped=$((skipped + 1))
      printf '  skip   %-56s %s\n' "$rel" "${table_target[$rel]}"
      ;;
    *)
      missing=$((missing + 1))
      unaccounted+=("$rel")
      ;;
  esac
done < <(cpp_files | LC_ALL=C sort -u)

# An entry whose C++ file is gone is dead weight, not a guess: it is reported
# rather than counted, because a tree that lost a file is a different tree.
absent=()
for cpp in "${!table_kind[@]}"; do
  [[ -n "${seen[$cpp]:-}" ]] || absent+=("$cpp")
done

printf '\nported or moved: %d\nskipped on purpose: %d\nunaccounted for: %d\n' \
  "$moved" "$skipped" "$missing"

if ((${#absent[@]} > 0)); then
  printf '\ntable entries with no C++ file (stale):\n'
  printf '  %s\n' "$(printf '%s\n' "${absent[@]}" | sort)"
fi

if ((missing > 0)); then
  printf '\nNo counterpart and no reason:\n' >&2
  printf '  %s\n' "${unaccounted[@]}" >&2
  exit 1
fi

echo "every C++ file is ported or has a recorded reason"
