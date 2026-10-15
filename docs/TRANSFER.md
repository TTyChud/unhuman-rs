# Transfer: the whole C++ engine, and the answers to the architecture questions

Authoritative work list. Derived from an actual survey of
`../unhuman/UHE/src/` — `Platform/Vulkan` (60 files), `UHE/Renderer` (17),
`Renderer3D` (12), `RHI` (12), `Scene` (10), `Renderer2D` (8), `Core` (17), and
the rest. Phase ordering is in `session.md`; this file is *what exists* and
*what it maps to*.

---

## 1. Message system: events, or messages?

The C++ engine has an **event bus**: `EventType` enum, `EventCategory` bitflags,
`EVENT_CLASS_TYPE` / `EVENT_CLASS_CATEGORY` macros, and an `EventDispatcher`
(`UHE/src/UHE/Events/`). Window, input and app-tick events go through it.

**Keep that, and keep it small.** The honest answer is *both*, for two different
jobs, and conflating them is the mistake:

| | event | message |
|---|---|---|
| shape | immediate, broadcast, `dyn` | queued, typed, deferred |
| consumers | many, none known to each other | exactly one, named |
| ordering | does not matter | matters |
| latency | now | next drain point |
| example | window resized, key pressed | texture reloaded, scene loaded |

- **Events** for platform and input: *many* unrelated systems react and none of
  them knows the others exist. A window-resize must reach the swapchain, the
  renderer and the editor, and none of those may depend on each other.
- **Messages** for renderer-to-backend work: one owner, order matters, and
  deferring is the point. "This texture changed" is not broadcast.

**What not to do:** a single `mpsc` channel for everything. That is a queue that
pretends to be a bus, and it turns "who handled this?" into a runtime question.

`pings` therefore has **two types, not one**: `Event` (immediate bus) and
`Message` (deferred queue). Both typed. Neither used for data that a caller could
just have passed as an argument.

---

## 2. ECS: EnTT, or what?

The C++ engine **does use EnTT** — `Scene.cpp` is full of `entt::entity`,
`m_UUIDIndex[uuid] = (entt::entity)entity`.

**Do not bind EnTT.** It is header-only C++ and would drag a C++ toolchain into
a crate whose whole premise is "native Rust". The Rust-native options:

| crate | shape | notes |
|---|---|---|
| **`hecs`** | archetype storage, sparse sets | **the recommendation.** Same archetype idea as EnTT, cache-friendly, pure Rust, small, no build script |
| `flecs` | bindings to the flecs C library | fastest, but it *is* a C binding — exactly what we are avoiding |
| `bevy_ecs` | archetype + reflection + schedules | pure Rust and excellent, but it brings its own world/schedule model and would fight `meats` |
| write our own | — | a week, and we would reimplement hecs badly |

**Decision: `hecs` as storage, wrapped by `meats`.** `meats` owns the engine's
view — components mirroring `Scene/Components.h`, the UUID index, world-transform
cache, relationship/parent handling. Storage details stay behind `meats` so
swapping later is one file.

EnTT's actual feature list maps cleanly: `registry`, `view<T>`, `group`, the UUID
index, and `try_get` all exist in hecs or are 20 lines.

---

## 3. Library policy

**Rule: Rust-native first. A C++ binding library only when the Rust one does not
exist and the capability is required.** EnTT and Jolt both fail that test — they
have good Rust equivalents.

| C++ uses | Rust choice | binding? |
|---|---|---|
| `glm` math | **`glam`** | no — faster, drop-in, the de facto standard |
| `entt` ECS | **`hecs`** | no |
| `imgui` / `imGuizmo` | **`egui` + `egui-gizmos`** | no — genuinely better, and pure Rust |
| `spdlog` logging | **`tracing`** | no — spans, structured, the async story is better |
| `miniaudio` audio | **`kira`** | no — built on miniaudio, pure Rust API |
| `fastgltf` | **`gltf`** | no |
| `meshoptimizer` | **`meshopt`** | thin bindings, but it wraps C SIMD and nothing native is competitive |
| `msdf-atlas-gen` | **`msdfgen`** | crate exists |
| `stb_image` | **`image`** | no |
| `GLFW` window | **`winit`** | no |
| `volk` Vulkan loader | **`ash`** | no — generated, no separate loader lib needed |
| `VulkanMemoryAllocator` | **`gpu-allocator`** or ours | Rust-native; ours is Phase 10 |
| `tracy` profiler | **`tracing` spans**, `puffin` | no |
| `basis_universal` | `basis-universal-rs` | only if Basis is actually needed; prefer `.ktx2`/`transcoder` |
| `jolt` physics | **`rapier`** | no — Jolt's own successor, pure Rust |
| `box2d` 2D physics | **`parry2d`** | no — same authors as Rapier, pure Rust |
| `Glad` OpenGL loader | — | not applicable; there is no GL backend |

**Net: zero required C++ bindings.** The only borderline cases are `meshopt`
(SIMD we would not write) and possibly Basis transcoding.

---

## 4. The transfer list

Everything the C++ engine has, by directory. ✅ done · ⬜ not started.

### Platform / Vulkan — 60 files

| C++ | Rust | state |
|---|---|---|
| `VulkanInstance` | `VulkanInstance` | ✅ |
| `VulkanDevice`, `LogicalDevice`, `PhysicalDevice`, `Context` | same | ✅ |
| `VulkanExtensionCheck` | `VulkanExtensionCheck` | ✅ |
| `VulkanBuffer`, `Texture`, `Texture2D`, `ImageView`, `Allocation` | same | ✅ |
| `VulkanCommandBuffer`, `CommandPool`, `Framebuffer`, `RenderPass` | same | ✅ |
| `VulkanDescriptorManager`, `DescriptorPool`, `DescriptorSet` | same | ✅ |
| `VulkanPipelineCache`, `PipelineState`, `GraphicPipeline`, `ComputePipeline` | same | ⬜ compute |
| `VulkanShader`, `VulkanBarrierEncoder`, `VulkanScopes` | same | ✅ |
| `VulkanSwapChain`, `VulkanFence`, `VulkanSemaphore` | same | ✅ swapchain; ⬜ binary pair |
| `VulkanUpload`, `VulkanDeletionQueue`, `VulkanSubmitEncoder` | same | ⬜ deletion queue |
| `VulkanBinaryFence`, `VulkanBinarySemaphore` | — | ⬜ decide or delete |
| `VulkanTypes`, `VulkanError`, `VulkanCommandPool` | same | ✅ |
| `RenderGraph/{Types,Resources,Builder,Compiler,RenderGraph,Executor}` | `chassis/graph/*` + `RenderGraph/*` | ✅ |
| `ShaderSystem/` | `forge` | ✅ |
| `UI/` | `mirror` | ⬜ **no Rust counterpart at all** |

### Core — 17 files

| C++ | Rust crate | state |
|---|---|---|
| `Application`, `EntryPoint` | `engine` | ⬜ |
| `Layer`, `LayerStack` | `adam` | ⬜ |
| `Log` | `tracing` | ⬜ |
| `Timestep` | `grind` / `adam` | ⬜ |
| `input.h`, `keyCodes`, `mouseButtonCodes` | `eyes` | ⬜ |
| `Window.h` | `eyes` | ⬜ |
| `UIID` | `meats` (stable ids) | ⬜ |

### Scene / ECS — 10 files

`Entity`, `Scene`, `Components` (19 component types), `ScriptableEntity`,
`SceneCamera`, `SceneSerializer` → `meats`. ⬜ all of it.

Components to transfer: Transform, Camera, Tag, SpriteRenderer,
SpriteAnimation, NativeScript, RigidBody2D, BoxCollider, ID, Model3D, ModelNode,
Relationship, RigidBody3D, BoxCollider3D, SphereCollider3D, CapsuleCollider3D.

### Events — 4 files

`Event.h`, `ApplicationEvent`, `KeyEvent`, `MouseEvent` → `pings`. ⬜

### Renderer3D — 12 files

`Renderer3D`, `LightSystem`, `SSAO`, `Animator`, `LoadModel`, `Animation`,
`Shape` → `bifrost`. ⬜ all.

### Renderer — 17 files

`Renderer`, `Renderer2D`, `Camera`, `EditorCamera`, `Font`, `Framebuffer`,
`Shader`, `SlangCompiler`, `Texture` → `bifrost`. ⬜ all.

### Renderer2D — 8 files

`Renderer2D`, `OrthographicCamera`, `OrthographicCameraController`,
`SubTexture2D` → `bifrost`. ⬜ all.

### Physics / Animation / Audio / Assets / ImGui / Math / Utils / Debug

`PhysicsSystem3D` → `femur` on rapier · `AudioEngine` → `wire` on kira ·
`ShaderManager`, `VfsSystem` → `vault` · `ImGui` → `mirror` on egui ·
`Math` → `glam` · `Debug` → tracing + `mirror`. ⬜ all.

### Bins

`sandbox`, `editor`, `nocap`. ⬜

---

## 5. What is *not* a transfer

Things the C++ does that we should deliberately not copy:

- **OpenGL.** There is no GL loader in Rust and there should not be.
- **`glm`.** Replaced by `glam`, not wrapped.
- **The hand-written render graph barriers.** Ours derives them from declared
  reads and writes; the C++ mixes a graph-synthesised barrier with a hand-written
  `oldLayout = eUndefined` in `VulkanCommandBuffer.cpp`. That is the bug we
  designed out.
- **`VulkanFrame`, if it owns a command buffer per pass.** Ours gives each pass
  its own pool *and* buffer so they record in parallel.

If something in the C++ looks like it should be copied but the list above says
otherwise, the list's reason is the argument — read it before overriding.
