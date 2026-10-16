# Work list

The loop state. Each item is done when the **gate** passes and the work is
committed. Nothing is marked done because a document says so.

## The gate

One script, so a green run locally means a green run in CI:

```bash
./scripts/gate.sh              # everything
./scripts/gate.sh --no-vulkan  # no driver present
./scripts/gate.sh --serial     # one test at a time
```

It runs, in order: build, `clippy -D warnings`, the full test suite under
validation, then a second pass that greps for `Validation Error` and fails if the
count is not zero.

Two rules it encodes, both of which have already cost time here.

`UHE_VALIDATION=1` enables validation only through `DeviceOptions::from_env`.
`DeviceOptions::new` defaults it **off**, so a suite calling `new` reports zero
validation errors while having checked nothing. That was true for this project's
entire life.

`--test-threads=1`. Several suites share one Vulkan device and one scratch
directory; in parallel they collide.

Plus `cargo fmt --check`, which found real drift the moment CI was written down.

## CI

`.github/workflows/ci.yml`, **Linux and Windows**, because the C++ engine
supported both and this one will.

The Linux jobs carry the weight -- lavapipe means the Vulkan backend is genuinely
exercised every commit. The Windows job proves something narrower and says so: a
GitHub Windows runner has **no Vulkan driver**, so it builds and runs the logic
suites and nothing touching the backend. That is still worth minutes per commit,
because platform breakage is invisible from Linux until it lands: a `#[cfg]` that
silently excludes a file, a path separator assumption, an `unsafe` block that only
misbehaves on one allocator. Compile-only is not "working", but it is not nothing.

`scripts/gate.ps1` mirrors `scripts/gate.sh`. Two copies rather than a wrapper,
because bash on a Windows runner is a Git Bash layer with its own path rules and a
gate that behaves differently on the platform it is meant to check is worse than
no gate.

Six jobs:

- **logic** — no GPU needed. fmt, clippy, and the suites that never touch a
  driver: `grind`, `chassis`, `forge`.
- **smoke** — build questions, not behaviour: `adam`'s `build_smoke` suite builds
  the workspace with all targets, in release and `--locked`, runs the doc tests,
  and checks the member list. A job of its own so a red run names the failure —
  a compile shape, not a behaviour.
- **parity** — the check behind this repository's claim that nothing was left
  behind in the rewrite: `scripts/parity.sh` against a fresh, shallow clone of
  the C++ tree (`unhuman-engine/UNHUMAN`). Every C++ file must have a Rust
  counterpart or a recorded reason it does not, so a file that quietly stops
  being ported fails the build. That is why `VulkanRenderPass` is no longer on a
  "dead code, skip it" list.
- **vulkan** — the backend against Mesa's **lavapipe** with the validation layer
  loaded. Same driver interface, same validation rules, on a CPU. Without this
  the backend suite would only ever run on one machine, which is how nine real
  bugs survived here for the length of the project.
- **shaders** — regenerates the checked-in SPIR-V and fails if it differs from
  the `.slang` source. A shader edit that forgets to regenerate leaves the suite
  testing the old bytes and still green. It needs the C++ tree's vendored
  `slangc`; where that compiler is not built, the step says so and skips rather
  than passing silently.
- **windows** — build, release, clippy, fmt and the driver-free suites, on
  Windows. It runs what the platform can and says what it cannot: a GitHub
  Windows runner has no Vulkan driver.

Never push. Commit only; the owner pushes.

## Layout

```
UHE/                        at the repository root, like the C++ tree
  Stormbreaker/             every GPU backend, behind chassis::Gpu
    vulkan/                 stormbreaker
    wgpu/                   stormglass
  platform/                 platform code only -- windowing, input, OS specifics
  src/
    core/                   adam: engine, app, the Plugin base
    rhi/                    chassis: the backend-neutral RHI, the seam
    jobs/                   grind: the job system
    shaders/                forge: Slang and WGSL in, SPIR-V out
    events/                 pings: the event bus, key and mouse codes
    platform/               eyes: window and input, winit re-exported
    scene/                  meats: ECS, components, hierarchy, .unhuman
    assets/                 vault: registry, VFS, loaders
    renderer/               bifrost: the renderer frontend, as a Plugin
    audio/                  wire: audio, as a Plugin
    physics/                femur: rigid bodies, as a Plugin
    editor/                 mirror: the editor model, as a Plugin
  bins/
    nocap/                  headless: gpu: None, fixed steps, exits
    sandbox/                windowed: wgpu, a real swapchain
    editor/                 gpui over mirror's model, gpui pinned by rev
  games/aimlab/             the UHEGAME demo layer, as a Plugin
  and chassis (UHE/src/rhi) now owns the *render graph*:
    chassis::graph          types, resources, builder, compiler, facade,
                           and `pass.rs` -- SlotTable, PassScope, PassRecorder,
                           the renderer-to-backend seam. A renderer implements
                           PassRecorder once and both backends call it.
                           -- moved out of the Vulkan backend, because a wgpu
                              backend that needed the graph would otherwise have
                              to depend on the Vulkan crate. Two backends
                              depending on each other is a fork waiting to happen.
```

Not under `crates/`. That was `crates/UHE/src`, which said `UHE` twice and
meant nothing -- the directory name and the crate names already carry it.

### Why the backends are in `Stormbreaker/` and not `platform/`

Vulkan and wgpu are graphics APIs, not platforms. The same Vulkan backend runs on
Linux and Windows, and wgpu runs on Vulkan, Metal and DX12 from one
implementation. Putting them under `platform/` would say "this is the Windows
graphics" and "this is the Linux graphics", which is precisely the false choice
the two-backend design exists to avoid.

`UHE/platform/` keeps one rule: **if moving to another operating system would
mean rewriting it, it is not platform code.** That leaves windowing, input and
anything naming `std::os::windows` -- which is `eyes`, and nothing yet.

The renderer is deliberately not in `Stormbreaker/` either. A backend that knows
how to draw is a backend that cannot be swapped for one that draws differently.
It belongs beside the engine as a plugin, which is where the C++ puts the
frontend too.

## Backend completion

- [x] **File parity with `Platform/Vulkan`** — 35 files, named after their C++
      counterparts. Remaining: the six render graph files.
- [x] **Draw path** — `VulkanCommandBuffer.rs` went 79 → 397 lines: dynamic
      rendering, bind pipeline/group, push constants, vertex/index buffers,
      draw, draw indexed, dispatch.
- [x] **1. Move the render graph into `stormbreaker`** as
      `VulkanRenderGraph.rs`, `VulkanRenderGraphTypes.rs`,
      `VulkanRenderGraphResources.rs`, `VulkanRenderGraphBuilder.rs`,
      `VulkanRenderGraphCompiler.rs`, `VulkanRenderGraphExecutor.rs`.
      Done: `stormgraph` is deleted, its 24 tests are
      `stormbreaker/tests/render_graph.rs`, and `stormbreaker` was already in
      `default-members`. Files landed:
      `VulkanRenderGraph{,Types,Resources,Builder,Compiler}.rs`.
      `barrier.rs` folded into `VulkanRenderGraphTypes.rs` — barriers are types,
      and a seventh file would have had no C++ twin. `VulkanRenderGraphExecutor.rs`
      is still to come.
      Note: the compiler must not import `ash`. That is what keeps it testable
      with no device.
- [x] **2. `VulkanRenderGraphExecutor`** — `CompiledGraph` in, one command list
      per pass out, recorded in parallel across `grind::Jobs`, submitted in
      compiled order. **The executor owns the `Recorder`**: the graph's compiled
      order is the submit order, and a second source of truth is what produced
      `VulkanDevice.cpp:630-637`.
- [ ] **3. `VulkanGpu`** — implements `chassis::Gpu` and `chassis::Resources`.
      Needs one `chassis` change first: `Gpu::swapchain()` must return
      `Option<&dyn Swapchain>` so headless is not a lie.
- [x] **4. Real pipeline layouts and multiple stages** — `create_layout` built a
      layout with zero descriptor sets, and the graphics path hardcoded
      `stages: [vertex]`, so no pipeline could ever shade.
      `chassis` gained `ShaderStage`, `StageMask`, `DescriptorKind`,
      `BindingDesc`, `DescriptorSetLayoutDesc`, and `PipelineDesc.shaders`
      replaced its single `shader` handle.
      Also collapsed **two** `DescriptorKind` enums into one: the RHI's and the
      backend's, with six overlapping variants and `InputAttachment` only in the
      backend — so any comparison between an RHI-described layout and a
      backend-built one was comparing distinct types, and the compiler allowed it.
      `DepthState` and `ColorTargetState` were also `pub` in `desc.rs` but never
      re-exported, so nothing outside `chassis` could name them.
- [x] **5. Executor test on hardware** — record a graph into a headless-surface
      swapchain image, read the pixels back, assert on them. Proves parallel
      recording end to end. The shader it draws with already exists:
      `assets/shaders/Triangle.vert.spv`. Everything else it needs is item 4,
      because a pipeline with no descriptor set layout cannot bind a shader that
      takes one — and this shader deliberately takes none.

## Shaders

- [x] **7. Test SPIR-V, from the vendored Slang.** `assets/shaders/Triangle.slang`
      compiles to a vertex/fragment pair with the `slangc` already built in the
      C++ tree (`UHE/vendor/bin/linux/slang/bin/slangc`, version 2026.11). The
      `.spv` files are checked in and `compile.sh` regenerates them, so `cargo
      test` is hermetic and a shader-compiler bug cannot surface as a backend
      bug.
- [x] **6. `forge`** — done, as a **driver** not an FFI. `Forge` wraps the
      vendored `slangc` behind a `ShaderCompiler` trait, with a content-addressed
      cache and parsed diagnostics. An in-process `libslang.so` binding can
      replace the driver without touching a caller.
      Why not FFI first: the Slang C API is C++ virtual interfaces
      (`slang::ISession`, `slang::IModule`), so binding it means binding a
      vtable layout across a crate boundary plus libclang at build time — real
      ABI risk for no benefit until compile time actually matters.
      The cache key includes the **compiler version**: Slang can legitimately
      emit different SPIR-V across releases, and a key without it serves stale
      shaders after a toolchain bump.

## The port: the game side

The 21,745-line C++ engine outside the backend now has a Rust counterpart, crate per
subsystem, written against `docs/architecture/01-crate-map.md` — that file is the
contract: the seams, the pinned shapes, the dependency direction, and everything
deliberately not ported with its reason.

| crate | C++ it replaces | what it is |
|---|---|---|
| `pings` | `Events/*`, `Core/keyCodes.h`, `mouseButtonCodes.h` | the event bus, the key table, one `Event` enum |
| `eyes` | `Platform/Windows/*`, `Core/Window.h` | winit 0.30 behind one `run_windowed(desc, frame)` |
| `meats` | `Scene/*` | hecs ECS, components, hierarchy, `.unhuman` YAML |
| `vault` | `AssestsManager/*`, `KTX2`, the CPU half of `LoadModel*` | registry with generational handles, VFS, loaders |
| `bifrost` | `Renderer3D/*`, `Renderer/*`, `MaterialGPU`, `LightSystem`, `SSAO` | passes, cameras, mesh and material caches, text and the 2D batch, animation, IK |
| `wire` | `Audio/*` | rodio behind `adam::AudioBackend`, voices, attenuation |
| `femur` | `Physics/PhysicsSystem3D` | rapier3d behind `adam::PhysicsBackend` |
| `mirror` | `UHE_EDITOR/*`, `ImGui/*` | the editor model: hierarchy, inspector, content browser, gizmos, play state, undo |

`adam::Engine` grew the fields those need (`events`, `input`, `scene`, `assets`,
`physics`, `audio`) and `App::frame` now owns the frame's shape: input is fed from the
event bus, fixed steps run `simulate_fixed_step` before the hooks, and plugins drop
before the engine so a renderer's GPU resources outlive nothing.

Status, stated the way this file states everything else: **the port has not been
compiled.** It was written on a machine with no compiler available, so every claim above
is a claim about files that exist and seams that are documented, not about a green run.
The next step is `cargo build --workspace`, then `./scripts/gate.sh`.

What *is* checked, mechanically, is coverage: `scripts/parity.sh` maps all 232 C++ files
under `UHE/src/UHE`, `UHE/src/Platform`, `UHE_EDITOR/src` and `UHEGAME/src` to either the
Rust file that holds the port or a one-line reason it was not ported — 193 moved, 39
skipped, 0 unaccounted. It fails on a C++ file with no entry and on a table entry whose
Rust target no longer exists, so a rename or a new C++ file breaks the check rather than
the table. The reasons are the ones in this file and in
`docs/architecture/01-crate-map.md`.

## Gaps the port records rather than hides

Each of these is a named refusal or a documented absence, not a silent no-op. They are
what a first green build will still not do.

| gap | what happens | what closes it |
|---|---|---|
| A Vulkan frame that draws the swapchain | `record_frame` refuses by name: a Vulkan swapchain image has no `chassis::Texture` handle, so the backbuffer slot cannot be bound | a non-owning `Textures` entry (`register_external`) plus binding it in `record_frame`, and an acquire in `VulkanGpu::frame_begin`, which today only resets the graph. `VulkanGpu::present` also transitions from `UNDEFINED`, which must stop once the graph's own epilogue leaves the image in `Present` |
| The editor viewport | shows the theme background and a label saying why: the RHI has no readback, so the drawn image cannot reach a gpui panel, and there is nothing to overlay a gizmo on or pick against | an `Image`/readback path in `chassis`, or gpui owning the surface the frame is presented to. Viewport picking itself is implemented (`mirror::picking`, CPU ray against mesh bounds) and needs only the camera ray a viewport would supply |
| `Gpu::graph()` / `RenderGraph::{add_pass, add_dependency}` | refused by name on both backends: a `RenderPassDesc` carries no resource states, and a pass declared from one would be ordered against states the caller never gave | deleting the dead trait surface, or growing `RenderPassDesc` to carry states. `bifrost` declares through `chassis::graph::RenderGraph`, which is the real path |
| Per-frame uniforms and the per-draw arena | buffers are re-created (with their bind groups) every frame, and Vulkan's `destroy_bind_group` is a no-op, so the groups are not reclaimed (`shortcut:` in `passes/uniforms.rs`). **Under load this is a live bug on Vulkan**: `VulkanDescriptorManager` rebuilds its pool (and resets its set registry) when the pool fills or a new layout appears, so an earlier `BindGroup` goes stale and the next bind panics on a stale handle — a frame with more than ~64 draws reaches that. wgpu is unaffected | a `Resources::upload_buffer` for the update-in-place path, plus `DescriptorPool::reset` once per frame and/or freeing sets in `destroy_bind_group` |
| Push constants | unusable: `PipelineDesc` has no push-constant range, `VulkanPipelineState` declares `&[]`, and wgpu's `CommandList::set_push_constants` is `unimplemented!()` — a panic, which nothing reaches because the renderer sends uniforms as buffers. The C++'s 192-byte block is a descriptor set here | a range in `PipelineDesc` and one lowering per backend |
| Two passes the graph cannot express | the transparent draws share the opaque pass (sorted back-to-front, per-draw pipeline) and the grid/gizmo pass has no depth attachment. A clear pass can also be culled, because `LoadOp::Load` is not a read. `PassBuilder::depth_load` and the `PASS_TRANSPARENT` recorder slot have landed; **nothing declares the pass yet**, so the shares-the-opaque-pass half is still true | the split in `bifrost`'s frame construction, now that the builder can express it, and a cull rule for a pass whose only job is a clear or a dependency |
| Mip chains, skinning, shadow-map sampling on wgpu | textures upload mip 0 (no GPU generate-mipmaps in either backend); nothing builds a bone palette from a loaded model into the vertex data yet; the depth descriptor kind that makes wgpu accept a depth view is `DescriptorKind::SampledDepth` (added, so both depth reads are describable) | backend mip generation, a `bifrost` skinning path, and nothing further for the depth kind |
| Demo assets | `assets/models/{Sphere.glb,Box.gltf,Box0.bin}` and `assets/audio/gunshot.wav` ship; the 12 MB animated pistol does not, because no code path names it | port the pistol's animation path, or leave it out |



Do not port. Each was measured, not assumed.

| C++ | why |
|---|---|
| `VulkanBinaryFence.h` | `VulkanFence.h:16` says in its own comment that it overlaps with `VulkanFence` and "the two should be consolidated". One sync type, not two. |
| `VulkanBinarySemaphore.h/.cpp` | the `.cpp` says "Intentionally empty: VulkanBinarySemaphore is an unimplemented placeholder". Nothing in the tree references it. |
| `VulkanShaderManager.h/.cpp` | **it does have a `.cpp`** — earlier note here claimed otherwise, and that was wrong. The correct reason to skip it: `Init()` and `ShutDown()` are both empty bodies. |
| `vma_usage.cpp` | 4 lines, allocates nothing |
| `UI/VulkanImGui*.h` | **not skipped — relocated.** Referenced by `UHE/src/UHE/ImGui/ImGuiLayer.cpp`. They belong with the UI layer, not the backend. |
| `UHE/Renderer2D/` | already `EXCLUDE`d from the C++ build (`UHE/CMakeLists.txt:12`) |
| `Platform/Linux/` | vestigial; `Window::Create` always returns `WindowsWindow` |
| `Platform/Windows/*` | winit replaces it, on both platforms, from one implementation |
| `ImGui/*`, `UHE_EDITOR/*` | replaced, not dropped: the model is `mirror`, drawn by gpui in `UHE/bins/editor` |
| `box2d` 2D physics | the 3D path is the product; 2D returns when a scene needs it, as a `femur` feature |
| `vendor/basis_universal` | UASTC transcode needs a C++ toolchain at build time; the KTX2 container is parsed by hand and a supercompressed payload is refused by name |
| msdf-atlas-gen **runtime** atlas generation | FreeType + msdfgen as a build-time C++ dependency. `vault`/`bifrost` rasterise a coverage atlas with `ab_glyph` and read a pre-baked MSDF atlas when one is present; the shader has both sampling modes |
| `AudioEngine::s_ActiveSounds`, `WindowInput`'s second copy of input state | ambient statics; `Engine` owns audio state and input state, and nothing else does |
| `UHE/Test/` | declarations with no bodies; `cargo test` replaces them |
| Tracy macros | replaced by `VulkanScopes.rs` debug-utils labels |
| global descriptor set | `VulkanCommandBuffer.cpp:370-375`, flagged at `docs/framegraph-and-rendergraph.md:57`; not carried over |

## Ash 0.38 builder trap

`default().some_builder().p_pointer(&slice)` sets the pointer and **leaves the
count at zero**. Count fields are only set by the struct literal.

Hit three times: `stage_count` (segfaulted RADV), `dynamic_state_count`,
`attachment_count`. Every `*_count` beside a pointer in this backend is now
written as a struct field.

## Backend selection

**wgpu is the default. Vulkan is opt-in.** Not a performance decision and not a
preference -- a *stability* one. When a frame goes wrong the first question is
always "the backend or the engine", and the default should be the one that
usually works. Vulkan is fully implemented and fully tested; it is the path to
bare metal and the only one that reaches a 1.0 device, and it is opt-*in* so that
finding out whether it is the unstable one does not need a rebuild.

Three ways to say it, first wins:

1. `--backend vulkan` (also `--backend=vulkan`)
2. `UHE_BACKEND=vulkan`
3. a `backend` key in the settings file

`chassis::backend::BackendSelection::resolve` is a pure function of
`(arguments, environment, settings)`, so every combination is tested -- pairwise,
not in a chain, because "flag beats environment" passing says nothing about "flag
beats settings", and the failing pair is the one nobody thinks to test. An
unrecognised value is an **error**, not a fallback: `--backend vlkan` that quietly
got wgpu would be reported as "the wrong backend", and nothing would point here.

`chassis::settings` is the file: `key = value`, no sections, no types, `#`
comments. It is deliberately that small, because the only key in it today is
`backend` and a settings system with nesting and inheritance would be a project
waiting for a second caller. A line that is not a `key = value` pair is *ignored*
rather than refused -- it is nearly always a comment style the parser does not
know, and a settings file that refuses to load because of a stray `;` is a file
people stop editing.

`BackendSource::Settings` existed from the first commit that introduced the enum
and **nothing could produce it** -- `resolve` took two arguments. The variant was
a claim about a feature that did not exist. It does now.

Reading the file is `engine`'s job, not `chassis`'s: a library that reaches for
the filesystem cannot be tested without one. A *missing* file is silent, because
every game ships with settings nobody wrote. A file that cannot be read is
skipped with a line on stderr saying so -- an engine that silently used wgpu
because the file was unreadable, having logged that the backend was vulkan, is
the exact failure `BackendSource` exists to prevent.

`engine::create_gpu` turns a `BackendSelection` into a `Box<dyn Gpu>`. Both
backends are implemented. The settings-file source is still a stub.

## Both backends, one trait

`chassis::Gpu` has two implementations: `WgpuGpu` and `VulkanGpu`. They are
shaped method for method -- same constructors, same accessors, same trait impls
-- because two backends that differ in their accessor names are not
interchangeable however well the trait fits. The genuine differences are in
return types: Vulkan has a queue *family*, wgpu has one queue; Vulkan can hold a
swapchain, wgpu headless cannot.

`UHE/src/engine` is the one crate that sees both. `adam` (core) must not depend
on a driver, and neither backend may depend on the other, so the factory has
nowhere else to live. Its real job is the parity test.

## The parity test

`UHE/src/engine/tests/backend_parity.rs`. One test body, run against both
backends, reached only as `&mut dyn Gpu`. "Both backends implement `Gpu`" is a
claim about types; this is a claim about behaviour. Nine cases, and every one has
to opt out of the run explicitly if a backend will not open -- a parity suite
that reports green having tested nothing is the failure the validation gate
already had once.

Three bugs it caught that per-backend tests could not:

- **`WgpuGpu` had two `WgpuResources`.** One for `gpu.resources()`, one inside
  the executor. A texture made through `dyn Resources` had a handle the executor
  could not resolve -- `InvalidHandle` at record time, far from the cause.
- **`VulkanGpu` declared `context` first, so it dropped the device first.**
  Drop order *is* declaration order; the recorder's 64 command buffers were all
  leaked at `vkDestroyDevice`. Invisible without validation, and under
  validation it blamed whichever test built a device first.
- **`SyncTier` existed twice**, in the RHI and in the backend, with the same
  name and the same variants -- so `GpuInfo::sync_tier` could not be assigned
  from `Capabilities::sync_tier`. Now one type, re-exported.

## The windowed path, which had never run

`VulkanGpu::with_swapchain` stored a swapchain and `VulkanGpu::swapchain()`
returned `None` regardless, so attaching one changed nothing observable. Fixing
that meant making the path work, and it had never worked.

**The instance enabled zero extensions.** Not "the wrong ones" -- `&[]`. So
`VK_KHR_surface` was absent, `ash::khr::surface::Instance::new` could not resolve
a single entry point, and the first call aborted the process. `VK_KHR_swapchain`
was likewise never requested on the device. Both are now enabled when offered.

**`ash` panics where Vulkan returns.** `headless_surface::Instance::new` resolves
entry points at construction and aborts if one is missing. It does not return an
error. So `create_headless_surface` checks the extension first and returns
`ExtensionUnavailable` -- a machine without it is ordinary, not a reason to kill
the process.

**`currentExtent == UINT32_MAX` means "you choose".** It is in the spec, it is
what every headless surface answers, and it is not a sentinel we invented. It was
passed straight to `vkCreateSwapchainKHR`, which asked for a 4294967295-wide
image. Now clamped into `minImageExtent..maxImageExtent` -- including a
*reported* extent, since a window larger than the display should letterbox rather
than fail. `choose_extent` is pure, so all four cases are tested with no device.

**`vkAcquireNextImageKHR` returns an index, not an image.** The handle was
fabricated as `vkImage::from_raw(index)`, which looks plausible and is not an
object the driver knows. The image is `get_swapchain_images()[index]`.

**Five more, all of them invisible until the path ran:**

- the acquire fence was submitted in a signalled state -- `present` waits it, so
  every acquire after the first was `VUID-vkAcquireNextImageKHR-fence-01287`
- the acquire semaphore was never consumed, so the second acquire was
  `VUID-vkAcquireNextImageKHR-semaphore-01286`; it is now `VK_NULL_HANDLE` and
  the fence alone carries the wait
- nothing transitioned the image to `PRESENT_SRC_KHR`, so present was
  `VUID-VkPresentInfoKHR-pImageIndices-01430`. The render graph cannot do this
  one: a swapchain image is handed out by the driver and no pass ever declares
  it, so `transition_to_present` is its own single-list submit
- that extra submit did not advance the timeline value, so it signalled the value
  the frame's last pass already signalled
- the image views were never destroyed, and the surface leaked whenever
  `Swapchain::new` failed

**Ownership was ambiguous and said so.** `Drop` destroyed the surface; the comment
said "owned by someone else". `new_owning_surface` takes it and frees it on every
error path; `new` is the borrow, for a window system that really does outlive the
swapchain.

Tests use `VK_EXT_headless_surface`, so the whole thing runs on a machine with no
display -- which is most CI machines, and therefore most of the tests that should
exist.

## Bind groups

`Resources::create_bind_group` used to take `layout: &[u32]`, and **that is why
neither backend implemented it**. A list of opaque `u32`s carries no binding
types, no counts and no stages, so there was nothing to build a descriptor set
layout from. Both backends returned an error saying "a layout is needed" and
both were right.

The signature now takes the same `DescriptorSetLayoutDesc` a pipeline declares for
its set, so a bind group and the pipeline that reads it are built from one
description and cannot disagree. Buffer bindings are explicit -- `(binding,
buffer, offset)` -- rather than positional, because a layout may declare bindings
0 and 7 and leave the gap for another resource kind.

Vulkan implements it. wgpu still refuses, honestly.

**Three bugs the first working version had**, all found by running it:

- **The descriptor pool was sized for one set and capped at 64.** `max_sets` is
  only a cap on set *count*; the descriptor counts are what the driver enforces,
  and they were the requirement of a single set. The first allocation succeeded
  and the second was `ERROR_OUT_OF_POOL_MEMORY`. `DescriptorPool::create` now
  multiplies by `max_sets` itself, so the two cannot be set inconsistently.
- **A zero-length descriptor range "succeeded".** `size.saturating_sub(offset)`
  turned an offset of 256 into a range of 0, which is
  `VUID-VkDescriptorBufferInfo-range-00341`. The call returned `Ok` and the
  descriptor was written pointing at nothing. An offset at or past the end is now
  refused, naming the offset and the buffer size.
- **Misaligned offsets were not checked at all.** A uniform buffer offset must be
  a multiple of `minUniformBufferOffsetAlignment`, and leaving that to validation
  produces an error naming a *limit* and a *descriptor* and neither the binding
  nor the offset. The limit is read into `Capabilities` at device creation and
  checked here.

**`CommandList::bind_group` was passing `VK_NULL_HANDLE` as the pipeline layout.**
That is a validation error the moment anything binds a group, and nothing bound
one, so it had never been caught. The layout is now remembered by
`bind_pipeline` -- it is the only correct answer, since a descriptor set is only
meaningful relative to the pipeline that reads it.

**Two tests were asserting the absence of the feature.** `bind_groups_refuse_
rather_than_silently_binding_nothing` asserted that *every* bind group failed. It
would have kept passing if `create_bind_group` had started returning a handle
that binds nothing -- the exact failure it exists to prevent, and the one it can
no longer see. Both tests now check the refusals that remain.

## The wgpu side of bind groups

Vulkan implements `create_bind_group`; wgpu now does too, and the parity test
runs one body against both through `Box<dyn Gpu>`.

Three wgpu-specific things, none of which were known before running them:

**`count: Some(1)` is rejected. `None` is accepted.** In wgpu 30, `count: Some(n)`
means a *binding array* and needs the matching `*_BINDING_ARRAY` feature --
including `Some(1)`. `Some(1)` is the natural translation of `count: 1` and it is
wrong; the error names a *feature*, not the count. Found by probing all three
spellings (`Some(1)`, `None`, wider visibility) on a real device, after reading
the source and the changelog had both said it should work.

**Layouts are compared by identity, not structurally.** Vulkan compares set
layouts structurally, so two equal-but-separately-created
`vk::DescriptorSetLayout`s are interchangeable. wgpu does not. So a shared cache
keyed on `DescriptorSetLayoutDesc::hash` -- which excludes the label, matching
`is_compatible` exactly -- is an optimisation on Vulkan and a correctness
requirement on wgpu. `WgpuPipeline::create` now takes its layouts from that cache
too; it used to build its own per pipeline, which would have made every bind
group unusable with the pipeline it was meant for.

**`create_bind_group` cannot fail.** It panics on a binding the layout does not
declare. The RHI returns `Result`, and Vulkan returns an error for the same
mistake, so a caller cannot defend itself -- the only way to give it an error is
to check first. Same asymmetry as shader modules, where wgpu panics on a corrupt
SPIR-V body. The rule: where a wgpu call is infallible, this layer checks what it
needs and turns the panic into an error.

`DescriptorSetLayoutDesc::hash` was added for the cache. It excludes the label for
the same reason `is_compatible` does, and the two *must* agree: if the hash
deduped and `is_compatible` did not, which one a backend believes would decide
whether a bind group and its pipeline end up with the same object.

## Passes record in parallel now

The executor takes a `grind::Jobs` and records every pass on a worker thread.
8 passes across 8 threads, measured -- see below for why that needed measuring.

**The blocker was `'static`, not `Send`.** `Jobs` spawns its workers when the pool
is built, so a job cannot borrow from the frame that queued it, and
`parallel_for` takes an `'static` closure. The registries are therefore
`Arc<RwLock<_>>`: `Arc` because a recording thread must *own* a handle,
`RwLock` because creation needs `&mut` and `Arc::make_mut` requires `Clone` on a
registry that owns driver objects. The lock is taken once per pass, not once per
handle lookup.

**One command pool per pass is what makes this sound.** The spec's
external-synchronisation object for recording is the **pool**, not the buffer. A
shared pool would be the same threading error as sharing a pool across threads,
which is the mistake this arena layout exists to avoid.

**Three things had to become owned** to satisfy `'static`: the `BarrierEncoder`
(now owns an `ash::Device` clone), the `CompiledPass` list (cloned per frame --
pure data, a few strings), and the `PassRecorder` (`Arc<dyn PassRecorder>`, which
`VulkanGpu` already held).

**Errors are recorded per pass slot, not first-come.** Two passes failing at once
must not make the reported error depend on which thread won the race; the lowest
failing index wins. Every recording is ended and recycled either way, because a
command buffer left in the recording state makes the pool reset illegal.

**Recordings come back through a channel, not shared storage.** `parallel_for`
takes an `Fn`, and `Fn` cannot mutate what it captured. They are also taken under
a lock rather than by unwrapping the `Arc` -- `JobCounter` settles *before* the
job closure is destroyed, so a worker can still hold a clone for a few
instructions after `wait` returns, and `try_unwrap` would fail with `Recording`
not being `Clone` and there being no fallback.

**The "is it actually parallel" test counted the wrong thing.** It collected
distinct thread ids into a set and compared the set's length against the pass
count. An eight-pass frame spread over two threads -- which is what happens the
moment the machine is busy -- reported "6 of 8 passes ran". No pass was lost;
eight had run on two threads. The fix counts invocations and threads separately,
and it took a deliberate load test to find, because on an idle machine the two
numbers coincide.

`parallel_for` runs inline when there is one worker, so the test states that as
its precondition instead of failing on a single-core box.

## The render graph, split like the C++ one

`VulkanRenderGraphExecutor.rs` was 757 lines holding the executor, the barrier
lowering, the render scope and per-pass recording -- none of which you could read
without reading the rest. C++ keeps these apart, and for the same reason. Now:

| file | what it owns |
|---|---|
| `VulkanRenderGraphResources.rs` | `RegisterTexture` / `RegisterBuffer` / `ClearRegistrations`, and `resolve_frame` |
| `VulkanRenderGraphBarriers.rs` | pre/post barrier recording, `open_render_scope` |
| `VulkanRenderGraphExecutor.rs` | resolve, record in parallel, submit, stats |

The bodies were **moved verbatim**. A file split that also rewrites the functions
is two changes wearing one name, and the rewrite is the one that breaks.

**The split found a missing stage.** C++ separates `Resolve` (producing an
`RGResolvedFrame` with real `vk::Image` / `vk::ImageView` / `vk::Buffer` handles)
from `Record`. We did not: resolution happened *inside* each recording thread, so
a slot pointing at nothing failed on whichever worker reached that pass, naming a
thread and not a handle.

`resolve_frame` now runs the whole frame first, before a single
`vkBeginCommandBuffer`. A bad slot is one error, on the submitting thread, naming
the handle.

**`ClearRegistrations` is not an optimisation.** A texture destroyed after
resolve but before submit leaves a dangling `vk::Image` in the table and tomorrow's
barrier names a freed image. Clearing every frame is what makes the table's
lifetime exactly one frame.

### The other four files, and why they did not move

C++ has `VulkanRenderGraphTypes`, `Resources`, `Builder`, `Compiler` and
`VulkanRenderGraph` alongside the executor. Ours are `chassis/graph/{types,
resources,builder,compiler,mod}.rs`, deliberately backend-neutral so wgpu can use
the graph without depending on ash. A `VulkanRenderGraphTypes.h` could not be used
by the second backend, and the second backend is the reason this exists.

So the parity mapping is:

| C++ | here |
|---|---|
| `VulkanRenderGraphTypes.{h,cpp}` | `chassis/graph/types.rs` |
| `VulkanRenderGraphResources.{h,cpp}` | `chassis/graph/resources.rs` (neutral) + `RenderGraph/VulkanRenderGraphResources.rs` (driver objects) |
| `VulkanRenderGraphBuilder.{h,cpp}` | `chassis/graph/builder.rs` |
| `VulkanRenderGraphCompiler.{h,cpp}` | `chassis/graph/compiler.rs` |
| `VulkanRenderGraph.{h,cpp}` | `chassis/graph/mod.rs` |
| `VulkanRenderGraphExecutor.{h,cpp}` | `RenderGraph/VulkanRenderGraphExecutor.rs` |

Every C++ file has a counterpart. Five of the six are in the RHI rather than the
backend, and that is the one place this port deliberately gave up 1:1 file
parity -- for a reason that a third backend would benefit from.

## Samplers, sampled images, and the one descriptor kind that does not port

`Resources::create_bind_group` now takes images and samplers alongside buffers,
and `create_sampler` exists on both backends. `chassis::Sampler` is its own
handle with its own tag: a sampler is not a view of anything, it outlives every
texture it samples, and folding it into `Texture` or `ImageView` would make
"which image, filtered how" unexpressible.

**Three bugs validation caught in the first working version**, all of them mine:

- **`imageLayout` was left `UNDEFINED`.** `vk::DescriptorImageInfo` zero-
  initialises, and writing a sampled image in `UNDEFINED` is
  `VUID-VkWriteDescriptorSet-descriptorType-04149`. It follows from the
  *declared* kind -- `SHADER_READ_ONLY_OPTIMAL` for a sample,
  `GENERAL` for a storage image -- and not from what is convenient here.
- **Two loops disagreed about which bindings were combined.** One decided the
  kind and skipped combined bindings; the other wrote them and indexed an empty
  vector. Fixed by sharing one `kinds` vector, so there is one decision read
  twice rather than two decisions.
- **Samplers leaked.** `VulkanResources` now has a `Drop` for them, with the
  argument written down: a sampler is small, is created per material rather than
  per resource, and nothing else in the frame owns it.

**`mipmap_mode` is not `vk::Filter`.** A mip mode is three-valued and a
mag/min filter is two, so mapping three onto two loses `LINEAR_MIPMAP_LINEAR` --
which is trilinear -- and quietly gives bilinear. That bug looks like "the
textures are blurry" and is in most engines.

**`maxLod` is not `f32::MAX` and not zero.** An infinite `maxLod` is a
validation error; zero silently drops every mip. The largest finite float is both.

### The one that does not port: `CombinedImageSampler`

Vulkan has `COMBINED_IMAGE_SAMPLER` and it is the convenient default. **wgpu
does not.** WebGPU has no combined descriptor, so `BindingResource` is Buffer,
Sampler, TextureView and their array forms -- there is no fifth. The RHI's most
convenient descriptor kind cannot be expressed on one of the two backends.

It is refused, and the refusal names the alternative. The parity test asserts the
two backends *disagree* here, which is the one place they must: pretending
otherwise would mean a renderer written against the common subset.

### The other asymmetry worth knowing

A view bound to a `SampledImage` descriptor must come from a `TEXTURE_BINDING`
texture on wgpu, and may come from anything on Vulkan -- which checks layouts at
barrier time instead. So "bind a render target as a texture by mistake" is a
validation error on one backend and invisible on the other.

### The third asymmetry: a sampled depth texture

A depth texture bound as a `SampledImage` is legal on Vulkan and refused on wgpu.
A depth view needs `TextureSampleType::Depth`, and wgpu checks the sample type of
the view a bind group carries against the layout's -- so a `Float` binding holding
the shadow map is a validation error naming neither the pass nor the shader.
Vulkan lowers both to one `SAMPLED_IMAGE` descriptor and checks the layout at
barrier time, which is why this was invisible there and why `Renderer3D`'s depth
reads only ever failed on the default backend.

So `DescriptorKind` has `SampledDepth`. This is the `CombinedImageSampler` case
with the sign flipped: there Vulkan had a kind wgpu has not got, here one
descriptor needs two declarations. Both of the renderer's depth reads name it --
the frame set's shadow map and SSAO's depth buffer.

## Texture upload: the hole the sampling shader found

`Textured.slang` exists and compiles. **There is no way to give a texture
contents.**

`Resources::create_buffer` takes initial data. `Resources::create_texture` does
not, and `CommandList` has `copy_texture_to_buffer` but not the other direction.
So a sampled texture is necessarily empty, which means there is nothing for a
sampler to sample.

This is why the sampling test is named `every_object_on_the_descriptor_path_
creates_and_resolves` and not something that claims pixels. Every way to fake
texels -- a debug-fill flag, a constant colour in the shader, skipping the upload
-- produces a green test for a renderer that cannot load an image. A test that
says it cannot prove the thing yet is worth more than one that pretends.

Landed since: `Resources::upload_texture` on both backends, and the Vulkan half
is `UploadRing::copy_to_image` -- the image sibling of the `copy_to_buffer` that
already existed. The material path follows: `TextureCache::upload` built its
desc with `TextureDesc::initial_data`, which `stormglass` never reads, so every
texture it made was empty on the default backend — it now uploads through
`upload_texture` like the rest. What remains is the *draw* half: a full-screen
quad over a 2x2 texture, read back, each quadrant compared exactly.

### Two barriers, and the second one is the point

The Vulkan upload transitions `UNDEFINED -> TRANSFER_DST_OPTIMAL` before the copy
and `TRANSFER_DST_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL` after. The second one is
the one that matters: a texture left in `TRANSFER_DST_OPTIMAL` is a texture every
later descriptor write refuses with
`VUID-VkWriteDescriptorSet-descriptorType-04149`.

Which is the shape of the bug this path exists to prevent: the upload works, the
texture is created, the bind group is created, and the draw is wrong because
nobody moved the image out of its transfer layout.

### The upload is queued, not finished

`upload_texture` returns when the copy is **submitted**. Destroying the texture
before the ring drains is `VUID-vkDestroyImage-image-01000`, which is what
validation reported against the test written to prove the feature worked.

The first draft of the trait doc said "synchronous". It was not, and it is the
worst kind of wrong -- reassuring, and it would have been followed. It now says
what is true and names `wait_uploads`.

### The one place the two backends differ in kind

**wgpu's upload is synchronous. Vulkan's is not.** `queue.write_texture` copies
before it returns; Vulkan queues a copy on a ring. A caller written for wgpu and
moved to Vulkan would destroy textures while they were in flight. Each test says
which side it is on.

### And one usage flag, spelled differently

Vulkan's `vkCmdCopyBufferToImage` needs `TRANSFER_DST`. wgpu's
`queue.write_texture` needs `COPY_DST`. The RHI's `TRANSFER_DST` maps to both,
so one description uploads on either -- but a texture created `SAMPLED` alone
fails on wgpu with "Usage flags TEXTURE_BINDING do not contain COPY_DST" and
works perfectly on Vulkan.

## Extension checking, which was in the wrong file

`VulkanExtensionCheck.rs` existed and held the sync-tier ladder and the
`Capabilities` struct. The *checking* was scattered across
`VulkanPhysicalDevice` (five call sites) and `VulkanLogicalDevice` (three), via
`has_device_extension`, which called `vkEnumerateDeviceExtensionProperties`
**every time**.

That is eight driver round-trips during one device creation where the C++ engine
does one. And `has_device_extension` answers "is this string present", which is
the wrong question for any extension promoted into core: a Vulkan 1.3 driver is
not required to advertise `VK_KHR_synchronization2`.

The sync path happened to be right -- it went through `select_sync_tier`, which
checks the API version. The other three went through the string check, so a device
that had a capability because of its version rather than its extension list was
reported as not having it.

`ExtensionCheck` now enumerates once, caches, and answers named queries that are
core-or-extension. `probe_capabilities` and logical-device creation take it
rather than re-querying. Five tests cover the promotion rule, including the two
halves: a promoted extension counts by core version alone, and a never-promoted
one (timeline semaphores) ignores the version entirely.

## The parallelism assertion was demanding the scheduler, not the executor

`seen.len() >= 2` failed under CPU contention with every pass recorded correctly
and on *one* thread. Not a defect: the waiting thread work-steals, so if every
worker is descheduled it runs the batches itself. Four spinners and twelve
workers makes that the correct outcome, and the test called it a failure.

The invariant is `ran == passes.len()` -- every pass exactly once, no drops and
no duplicates. The spread is measured and reported, and only demanded when
`pool_has_spare_worker` says the pool actually has an idle worker -- asked of the
pool, not guessed from the core count.

## The flake that was not a flake, and the lie in a commit message

`passes_record_on_more_than_one_thread` failed intermittently under CPU
contention, reporting 3, 4, 5, 6 and 7 of 8 passes -- which reads as a scheduler
dropping work under load.

It was not. Instrumenting the executor showed **all eight closures entered and all
eight recordings came back**, every time. The assertion was comparing the number
of *distinct threads* against the number of passes, and a scheduler under
contention spreads eight passes over two or three threads.

Three things made this expensive:

**The bug is invisible while it is wrong.** On an idle machine the two numbers
coincide, so the assertion passed for as long as it was nonsense.

**Commit `f5c30db` claimed the fix and did not contain it.** Its message says the
test "now counts invocations and threads separately". It did not -- a `runs`
counter was added and then never read, and the assert kept using the thread set.
The fix was lost in a backup-and-restore shuffle, and the message was written from
memory of the intent rather than from the diff.

**`Relaxed` was the wrong first theory.** The obvious explanation for a stale
count under contention is a `Relaxed` atomic, so it was changed to `SeqCst`. That
changed nothing, which is what proved the ordering was never the problem.

Reproduced on demand rather than re-run until green: four `nice`-d spinners in a
self-contained script that kills its own load, so the load cannot outlive the
measurement the way it did the first time -- when one survived a `kill` and
starved the shell for twenty minutes.

Three full gates green under load before committing.

## Roadmap

The full plan is in `session.md` (untracked — it is machine-local). Ten phases,
in dependency order:

0. Close the backend gap: window system, wgpu swapchain, descriptors, samplers,
   wgpu errors, wgpu parallel recording, compute on both.
1. `bifrost` — the render graph frontend over `chassis::graph`.
2. Resources — cache, deferred destruction by frame number, upload heap.
3. Geometry — meshes, vertex layouts, sub-allocation, LOD.
4. Materials and shaders — parameters as data, bind group caching, variants.
5. Camera, culling, draw submission.
6. Lighting — clustered, shadows, PBR, IBL, AO.
7. Post — HDR, bloom, tonemap, TAA, DoF, SSR.
8. Animation, particles, terrain.
9. `meats`, `vault`, `mirror`, `femur`, `wire`, `pings`, the binaries.
10. Shipping — one backend per build, GPU timestamps, frame capture, device loss.

Each phase ends with a test that would fail if it regressed, not with "it
compiles". The table of those tests is in `session.md`.

## Decisions made, do not relitigate without a reason

- Static plugin selection by Cargo feature, one crate per alternative.
- `Engine` holds data, plugins hold behavior; the registry sits beside `Engine`.
- No barrier API on `chassis::CommandList`. The graph is the only owner of sync.
- One `CommandPool` per command list. The pool, not the buffer, is the external
  synchronisation object during recording.
- Primary command buffers, not secondary: a secondary cannot be submitted to a
  queue, and routing through `vkCmdExecuteCommands` puts one thread back in the
  recording path.
- Pre-barriers only. Each pass owns the transitions for what it touches, so its
  command list is self-contained and lists can record in parallel.
- No compile cache for the graph. It is rebuilt and recompiled every frame; at
  realistic pass counts that is microseconds.
- `vma` 0.3.1 has `destroy_buffer`/`destroy_image` but **no `new_buffer`,
  `new_slice` or `map_slice`** — it cannot allocate. Manual
  `vk::DeviceMemory` in `VulkanAllocation.rs` until a working crate exists, behind
  a seam so VMA drops in later.