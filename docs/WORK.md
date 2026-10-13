# Work list

The loop state. Each item is done when the **gate** passes and the work is
committed. Nothing is marked done because a document says so.

## The gate

Every item must leave this true, checked in this order:

```bash
cargo build --workspace
cargo test --workspace
UHE_VALIDATION=1 cargo test --workspace 2>&1 | grep -c "Validation Error"   # must be 0
cargo clippy --workspace --all-targets                                       # must be clean
git commit
```

`UHE_VALIDATION=1` is not optional. It found five real bugs in this backend that
`cargo test` alone could not see — including a command pool shared across worker
threads, a missing `RESET_COMMAND_BUFFER` flag, and a timeline value that did not
strictly increase. Without it, invalid Vulkan usage shows up as a segfault or
`ERROR_DEVICE_LOST` in an unrelated test rather than as the error it is.

Never push. Commit only; the owner pushes.

---

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
- [ ] **2. `VulkanRenderGraphExecutor`** — `CompiledGraph` in, one command list
      per pass out, recorded in parallel across `grind::Jobs`, submitted in
      compiled order. **The executor owns the `Recorder`**: the graph's compiled
      order is the submit order, and a second source of truth is what produced
      `VulkanDevice.cpp:630-637`.
- [ ] **3. `VulkanGpu`** — implements `chassis::Gpu` and `chassis::Resources`.
      Needs one `chassis` change first: `Gpu::swapchain()` must return
      `Option<&dyn Swapchain>` so headless is not a lie.
- [ ] **4. Real pipeline layouts** — `create_layout` currently returns a layout
      with zero descriptor sets. Needs `bifrost` to declare bind groups.
- [ ] **5. Executor test on hardware** — record a graph into a headless-surface
      swapchain image, read the pixels back, assert on them. Proves parallel
      recording end to end.

## Shaders

- [ ] **6. `forge`** — Slang. Chosen explicitly over alternatives. Separate
      repo producing a `slang-sys` crate (cmake + bindgen against the existing
      C++ Slang tree), because `Cargo.lock` pins crates and not C++ builds.
- [ ] **7. Until then**: generate SPIR-V for tests with `naga` as a
      **dev-dependency only**, so the executor is provable on hardware now.

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