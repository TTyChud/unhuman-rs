# Work list

The loop state. Each item is done when the **gate** passes and the work is
committed. Nothing is marked done because a document says so.

## The gate

Every item must leave this true, checked in this order:

```bash
cargo build --workspace
UHE_VALIDATION=1 cargo test --workspace -- --test-threads=1
UHE_VALIDATION=1 cargo test --workspace -- --test-threads=1 2>&1 | grep -c "Validation Error"  # must be 0
cargo clippy --workspace --all-targets                                            # must be clean
git commit
```

**Two traps in that gate, both of which have already cost time here.**

`DeviceOptions::from_env` reads `UHE_VALIDATION`. `DeviceOptions::new` does not, and
defaults validation to *off*. A test suite that calls `new` reports zero validation
errors while the layer never loaded — which is exactly what happened here for most
of the project's life, and for `forge`'s own end-to-end tests.

`--test-threads=1`. Several tests share one Vulkan device and the scratch
directory. In parallel they collide; serially they do not.

`UHE_VALIDATION=1` is not optional. It found five real bugs in this backend that
`cargo test` alone could not see — including a command pool shared across worker
threads, a missing `RESET_COMMAND_BUFFER` flag, and a timeline value that did not
strictly increase. Without it, invalid Vulkan usage shows up as a segfault or
`ERROR_DEVICE_LOST` in an unrelated test rather than as the error it is.

Never push. Commit only; the owner pushes.

---

## Layout

```
crates/UHE/
  platform/vulkan/     the Vulkan backend. No renderer, no render graph.
  src/
    core/              adam: engine, app, the Plugin base
    rhi/               chassis: the backend-neutral RHI interface
    jobs/              grind: the job system
    shaders/           forge: Slang in, SPIR-V out
    renderer/          the render graph and the renderer -- as a Plugin. Coming.
```

`platform/vulkan` is `UHE/src/Platform/Vulkan/` and `src/core` is `UHE/src/UHE/`.
The renderer is deliberately **not** in `platform`: a backend that knows how to
draw is a backend that cannot be swapped for one that knows how to draw
differently. It belongs beside the engine as a plugin, which is where the C++
puts the frontend too.

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

## Game side — none of this exists yet

0% of the 21,745-line C++ engine outside the backend.

- [ ] **8. `eyes`** — window + input. Needs `ash-window` + `winit`. Unblocks a
      real window and therefore a real swapchain.
- [ ] **9. `meats`** — ECS on hecs, hierarchy, scene serialisation (2,943 C++
      lines).
- [ ] **10. `bifrost`** — renderer frontend: passes, camera, material, mesh.
- [ ] **11. First triangle on screen.** Requires 8, 9, 10 and a shader.
- [ ] **12. `vault`** — asset registry with generational handles. The C++ is
      path-based with no registry (`IDEA.md:61`).
- [ ] **13. `femur`, `wire`, `mirror`, `pings`** — physics, audio, UI, events.

## Dead or skipped, deliberately

Do not port. Each was measured, not assumed.

| C++ | why |
|---|---|
| `VulkanRenderPass.cpp` | 39 lines, obsolete under dynamic rendering |
| `VulkanShaderManager.h` | has no `.cpp` at all |
| `vma_usage.cpp` | 4 lines, allocates nothing |
| `VulkanBinaryFence.h` / `VulkanBinarySemaphore.h` | the same type defined twice, an ODR violation flagged in the engine's own `docs/vulkan-sync-and-rendergraph.md:87-90` |
| `UHE/Renderer2D/` | already `EXCLUDE`d from the C++ build (`UHE/CMakeLists.txt:12`) |
| `Platform/Linux/` | vestigial; `Window::Create` always returns `WindowsWindow` |
| `UHE/Test/` | declarations with no bodies; `cargo test` replaces them |
| Tracy macros | replaced by `VulkanScopes.rs` debug-utils labels |
| global descriptor set | `VulkanCommandBuffer.cpp:370-375`, flagged at `docs/framegraph-and-rendergraph.md:57`; not carried over |

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