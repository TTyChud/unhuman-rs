# unhuman-rs — Architecture Spine

Status: proposal for review. Renderer shape (forward vs deferred) is **deliberately
undecided** — §9 requires the graph to support both without changing.

---

## 0. What we are designing out of C++ UHE

Five concrete failure modes in the C++ tree, each with its cost:

| Failure | Where | Cost |
|---|---|---|
| Ambient process globals: `g_VulkanContext`, `Application::s_instance`, `Renderer::s_Device`, `AudioEngine::s_ActiveSounds` | `Platform/Vulkan/VulkanContext.h:56`, `UHE/Core/Application.h:50`, `UHE/Renderer/Renderer.h:16` | Nothing is constructible in isolation, so `tests/scene` **recompiles engine `.cpp` files and stubs the renderer** (`tests/scene/CMakeLists.txt:10-27`) instead of linking |
| Backend reaching up into the engine layer to recover its own device | `VulkanCommandBuffer.cpp:79-85` — `static_cast` of `Renderer::GetDevice()` while already holding a `VulkanContext*` containing the device | Layer inversion; every global is a second path to the same object |
| "Backend-agnostic" layer hard-includes the only backend | `ImGuiLayer.cpp:11` — `dynamic_cast<RHI::VULKAN::VulkanDevice*>` | The abstraction was never real: one backend plus ceremony |
| Two owners of synchronization | legacy `BeginRenderPass` hand-writes barriers (`VulkanCommandBuffer.cpp:104,153`) *and* the graph synthesizes them; reconciled by `MarkSwapchainRenderedByLegacy` (`VulkanDevice.h:82`) | Correctness of the graph is unenforceable |
| One command buffer for all passes ⇒ forced linearization | `VulkanDevice.cpp:630-637` chains every pass to its predecessor | **The DAG is dead.** The graph compiles a real graph, then discards the parallelism |

The last row matters most: the graph was built correctly and neutralised by the
recording model underneath it.

---

## 1. Six rules

1. **No ambient globals.** Every subsystem is reachable only through `&mut Engine`
   or `&mut App`. No `extern`, no function-local statics, no service locator.
2. **No Vulkan type crosses a crate boundary.** Enforced by dependency direction:
   the interface crate does not depend on `ash`. Not by discipline — by the
   compiler.
3. **Exactly one owner of synchronization: the render graph.** No
   `BeginRenderPass` that hand-writes barriers. Passes declare reads/writes, the
   graph emits barriers. Non-negotiable; rule 2's parity depends on it.
4. **Plugins hold behavior, `Engine` holds data.** A plugin owns its own private
   state and receives `&mut Engine` per hook. `adam` therefore never depends on
   any plugin crate.
5. **Recording is per-thread from day one.** Submit stays single-threaded
   (Vulkan requires it); recording does not.
6. **Headless is the same binary with `gpu: None`.** Not a separate target, not a
   retrofit.

---

## 2. Naming

Crate names are code names. **Inside a crate, names stay functional**
(`stormbreaker::rendergraph::compiler`), so grep still works and error messages
stay legible.

| crate | role |
|---|---|
| `adam` | base engine: `App`, main loop, core types, `UUID`, time |
| `chassis` | RHI interface — the standard every backend implements |
| `stormbreaker` | Vulkan backend (device, swapchain, resources) + render graph |
| `bifrost` | renderer frontend: passes, cameras, materials, mesh, 2D batch |
| `meats` | ECS, components, hierarchy, serialization |
| `grind` | job system + task graph (ported 1:1 from C++) |
| `eyes` | window + input |
| `vault` | asset registry, VFS, streaming |
| `forge` | Slang FFI (bindgen) + pipeline cache + shader registry |
| `femur` | physics |
| `wire` | audio |
| `mirror` | UI/editor overlay |
| `pings` | event bus |

Bins: `sandbox` (windowed) · `editor` (windowed + `mirror`) ·
`nocap` (headless / robotics).

---

## 3. Workspace

```
adam/            App, Engine, loop, core types, time, UUID          [always]
chassis/           RHI traits + descriptors + generational handles    [feature: gpu]
stormbreaker/    Vulkan backend                                     [feature: gpu]
  rendergraph/   graph crate — workspace-internal, see §8
bifrost/         renderer frontend passes                           [feature: render]
grind/           job system + task graph                            [always]
meats/           ECS + hierarchy                                    [feature: scene]
eyes/            window + input                                     [feature: window]
vault/           assets + VFS                                       [always]
forge/           Slang FFI + pipeline cache                         [feature: shaders]
femur/           jolt | rapier                                      [alt]
wire/            rodio | miniaudio                                  [alt]
mirror/          egui | iced | imgui                                [alt]
plugins are selected by feature; alternatives are sibling crates,
never a runtime switch inside one crate.
```

Direction of dependency is strictly downward:

```
   mirror  wire  femur  bifrost  stormbreaker  grind  vault  eyes  meats
                                 \          |        /
                                  \         |       /
                                    ───► chassis ◄───
                                         │
                                       adam
```

Nothing below ever names anything above it. `bifrost` depends on `chassis` and
never on `stormbreaker`.

---

## 4. `chassis` — the RHI interface

The C++ RHI failed for three reasons, all fixable without abandoning it: `void*`
handles (`RHITypes.h:37-41`), one impl, and the engine layer casting straight
through it. Keep the layer; make it real.

**The test: no Vulkan type appears in any signature outside `stormbreaker`.**
`chassis` does not depend on `ash`, `volk`, or VMA — that is the enforcement.

```rust
// Handles: generational, replacing the C++ void* encoding.
pub struct Handle<T> { index: u32, generation: u32, _tag: PhantomData<T> }
// Reuses the RGHandle<T>{index, generation} already correct at
// VulkanRenderGraphTypes.h:40-51, promoted from graph-internal to the whole RHI.
pub struct Buffer(pub Handle<Buffer>);
pub struct Texture(pub Handle<Texture>);
pub struct Pipeline(pub Handle<Pipeline>);
```

More surface than the C++ RHI, split so it never becomes a god trait:

| trait | scope |
|---|---|
| `Gpu` | device lifecycle, acquire, submit, present, `take_command_lists(n)` |
| `Resources` | create/destroy buffers, textures, views, pipelines, descriptors |
| `CommandList` | record only — **no barrier API**; the graph owns synchronization |
| `RenderGraph` | pass declaration, `validate`, `compile`, `execute` |
| `Swapchain` | extent, format, recreate, acquire |

`chassis` also owns the **descriptors** — `TextureDesc`, `BufferDesc`,
`PipelineDesc`, `RenderPassDesc`, format enums. That is where extensibility
actually lives: a second backend is a new crate implementing these, not new
`impl` blocks bolted onto a one-impl trait.

---

## 5. `Engine` — data only

```rust
pub struct Engine {
    pub jobs:    grind::Jobs,
    pub time:    Time,                     // accumulator, fixed_dt, scale
    pub scene:   meats::Scene,
    pub input:   eyes::Input,

    // --- substitutable: exactly one impl compiled in, chosen by feature ----
    pub gpu:     Option<Box<dyn Gpu>>,     // chassis::Gpu, impl in stormbreaker
    pub assets:  Box<dyn AssetSource>,
    pub models:  Box<dyn ModelLoader>,
    pub physics: Option<Box<dyn PhysicsBackend>>,
    pub ui:      Option<Box<dyn UiBackend>>,
}
```

Every field is either an `adam`/`chassis` interface type or a plain struct.
`adam` has no dependency on any plugin crate (rule 4). The renderer is **not**
here — it is private state inside `bifrost`'s plugin, which is why it can be a
concrete struct with no `dyn` in the frame's hot path.

Ownership: `Engine` owns everything. Subsystems take `&mut` borrows, never
`Arc<Self>` back to the Engine. `StormbreakerGpu` owns the `VkDevice`, VMA
allocator, swapchain and frame contexts; the C++ `g_VulkanContext` global bag
becomes a plain field inside it.

---

## 6. Plugin contract — a real trait and a registry

```rust
pub struct App {
    engine:  Engine,                 // data
    plugins: Vec<Box<dyn Plugin>>,   // behavior + ordering
    running: bool,
}

pub trait Plugin: Send + 'static {
    const NAME: &'static str;

    fn build(&mut self, eng: &mut Engine) -> Result<()>;

    fn on_start(&mut self, eng: &mut Engine) -> Result<()> { Ok(()) }
    fn on_update(&mut self, eng: &mut Engine, ctx: &mut FrameCtx) {}
    fn on_render(&mut self, eng: &mut Engine, ctx: &mut FrameCtx) {}
    fn on_stop(&mut self, eng: &mut Engine) {}
}
```

**The registry lives beside `Engine`, not inside it.** The previous Rust attempt
stored plugins in a field of `App` that was itself borrowed by the hooks
(`crates/unhuman/src/app/app.rs:29`), so `for p in &mut self.plugins { p.on_update(self) }`
aliased `&mut Engine` with `&mut Vec<Box<dyn Plugin>>` and needed
`std::mem::take` at `app.rs:101`. Here `&mut self.engine` and `&mut self.plugins`
are disjoint fields. No aliasing, no take, no `RefCell`.

A plugin owns its own private state:

```rust
pub struct BifrostPlugin { renderer: Bifrost }

impl Plugin for BifrostPlugin {
    const NAME: &'static str = "bifrost";

    fn build(&mut self, eng: &mut Engine) -> Result<()> {
        self.renderer = Bifrost::new(eng.gpu.as_ref()?)?;
        Ok(())
    }

    fn on_render(&mut self, eng: &mut Engine, ctx: &mut FrameCtx) {
        // borrows disjoint fields of Engine; own state is already private
        let Some(gpu) = &mut eng.gpu else { return };
        self.renderer.record(gpu.as_ref(), &mut eng.jobs, &eng.scene);
    }
}
```

Ordering is registration order, visible in the bin:

```rust
App::new()
    .add(Box::new(StormbreakerPlugin))     // installs gpu
    .add(Box::new(GrindPlugin))
    .add(Box::new(MeatsPlugin))
    .add(Box::new(EyesPlugin))
    .add(Box::new(VaultPlugin))
    .add(Box::new(FastgltfPlugin))
    .add(Box::new(JoltPlugin))
    .add(Box::new(ForgePlugin))
    .add(Box::new(BifrostPlugin))          // needs gpu from StormbreakerPlugin
    .add(Box::new(MirrorPlugin))
    .run()
```

Cost: one vtable call per plugin per frame. At ~10 plugins that is ~25 ns.
Irrelevant. What it buys: `Plugin::NAME` gives `--list-plugins` and a boot log
for free, and adding a plugin is one line.

---

## 7. Frame loop

```rust
fn frame(app: &mut App) {
    app.engine.input.begin_frame();

    // --- fixed-step simulation, N substeps, jobs inside ------------------
    while app.engine.time.consume_fixed_step() {
        app.engine.jobs.scope(|jobs| {
            app.engine.scene.update(jobs, dt);
            if let Some(p) = &mut app.engine.physics { p.step(dt); }
            app.engine.assets.tick(jobs);
        });
    }

    for p in &mut app.plugins { p.on_update(&mut app.engine, ctx); }

    // --- frame begin -------------------------------------------------------
    if let Some(gpu) = &mut app.engine.gpu { gpu.frame_begin(); }

    // --- parallel pass recording -------------------------------------------
    for p in &mut app.plugins { p.on_render(&mut app.engine, ctx); }

    // --- submit + present ---------------------------------------------------
    if let Some(gpu) = &mut app.engine.gpu { gpu.submit(); }

    app.engine.input.end_frame();
}
```

### Why recording parallelises (rule 5)

`Bifrost` owns its passes **by value**:

```rust
pub struct Bifrost {
    passes: Vec<Box<dyn Pass>>,   // each owns its pipelines, descriptors, layouts
}
```

`record` does:

1. `let lists = gpu.take_command_lists(jobs.worker_count() + 1);` — allocates N
   secondary command lists from the frame arena, **moving them out of the gpu**
   so the closures own them. No `Arc<Mutex>`.
2. Compiles the graph once, this frame, **with no caching** (§10).
3. For each pass node, pushes a job closure that **owns** `Box<dyn Pass>`, one
   command list, and `&Gpu`. The graph supplies pre-recorded barriers in the list;
   the closure calls `pass.draw(&list, ctx)`.
4. `gpu.submit()` walks the *compiled* order and submits the secondary buffers.

No locks, no `RefCell`, no `static_cast`. Each job owns exactly what it touches,
which is why this works in safe Rust.

### Headless

`nocap --steps 10000` runs the identical `frame()`, with `gpu == None` and
no `StormbreakerPlugin` / `BifrostPlugin` registered. No window, no vsync, no
present. Deterministic fixed steps, exits after N. Same code path — that is the
whole point of rule 6.

---

## 8. `stormbreaker` file map

Mirrors `Platform/Vulkan/` decomposition one-to-one, Rust naming, ~25 files.
The render graph is a workspace-internal crate *within the stormbreaker group* —
same plugin boundary, but the compiler stays unit-testable with no GPU and no
VMA, which is exactly what your `tests/rendergraph` proved was necessary.

| file | mirrors C++ | responsibility |
|---|---|---|
| `device.rs` | `VulkanDevice.{h,cpp}` | device, swapchain, frame begin/end, deletion flush |
| `context.rs` | `VulkanContext.h` | plain struct field — **not** `extern` |
| `extension_check.rs` | `VulkanExtensionCheck.h` | feature bits + `SyncTier` gating |
| `swapchain.rs` | `VulkanSwapchain` | surface, present modes, recreate |
| `command_list.rs` | `VulkanCommandBuffer` | recording, **no barrier API** |
| `barrier_encoder.rs` | `VulkanBarrierEncoder` | `vkCmdPipelineBarrier2` / legacy lowering |
| `submit_encoder.rs` | `VulkanSemaphore::Submit` | `vkQueueSubmit2` / legacy lowering |
| `semaphore.rs` | `VulkanSemaphore` | binary + timeline |
| `binary_fence.rs` | `VulkanBinaryFence` | (C++ has an ODR duplicate — see §10) |
| `frame_context.rs` | `VulkanFrameContext` | per-frame arena, frames in flight |
| `buffer.rs` | `VulkanBuffer` | VMA alloc, upload, barriers |
| `texture.rs` | `VulkanTexture` | mips, layouts, views |
| `image_view.rs` | `VulkanImageView` | |
| `pipeline.rs` | `VulkanPipelineState` | graphics/compute creation |
| `pipeline_cache.rs` | `VulkanPipelineCache` | per-device cache, PSO dedup |
| `shader.rs` | `VulkanShader` | `VkShaderModule` from SPIR-V |
| `descriptor.rs` | `VulkanDescriptorManager` | pool/set alloc, per-pass sets |
| `framebuffer.rs` | `VulkanFramebuffer` | |
| `deletion_queue.rs` | `RHI/DeletionQueue.h` | deferred destroy, N-frame latency |
| `upload.rs` | `VulkanDevice::ImmediateSubmit` | **rewritten**: per-thread staging, no pool `reset()` |
| `utils.rs` | `VulkanUtils.cpp` | conversions |
| `scopes.rs` | (new) | RAII debug names, profiling zones |
| `plugin.rs` | — | `impl Plugin for StormbreakerPlugin`, installs `eng.gpu` |
| `rendergraph/types.rs` | `VulkanRenderGraphTypes.h` | `Handle<T>`, pass/resource descs |
| `rendergraph/resources.rs` | `VulkanRenderGraphResources.cpp` | version log, slot registry |
| `rendergraph/builder.rs` | `VulkanRenderGraphBuilder.h` | fluent `color/depth/read/write/queue` |
| `rendergraph/compiler.rs` | `VulkanRenderGraphCompiler.cpp` | Kahn topo, cull, lifetimes, barriers |
| `rendergraph/executor.rs` | `VulkanRenderGraphExecutor.{h,cpp}` | resolve, record, submit |
| `rendergraph/facade.rs` | `VulkanRenderGraph.h` | validate / compile / dump / reset |

---

## 9. Renderer-shape neutrality

`bifrost` declares passes as data: `PassDesc { reads, writes, queue, kind }`. The
graph compiles from that. Forward+ and deferred differ only in which passes the
crate instantiates and how many attachments a `PassDesc` carries. No change to
`chassis` or `stormbreaker` either way. The decision stays open until the second
pass exists.

---

## 10. Deliberately not ported

- **Legacy `BeginRenderPass`** — the single biggest win. It is the reason C++ has
  two sync owners.
- **Cached graph compilation.** C++ recompiles every frame
  (`VulkanDevice::EndFrameGraph:595`); at realistic pass counts that is
  microseconds. A structural-hash cache is complexity with no measurable payoff.
- **Dead code**: `UHE/Renderer2D/Renderer2D.cpp` (867 L, already `EXCLUDE`d at
  `UHE/CMakeLists.txt:12`), the vestigial `Platform/Linux/` layer, both stub
  `ShaderManager`s, the Tracy dummy-work loop at `Application.cpp:150-158`.
- **`VulkanBinaryFence.h`** redefines `VulkanBinarySemaphore` (ODR violation,
  flagged in your own `docs/vulkan-sync-and-rendergraph.md:87-90`). One type.
- **`ImmediateSubmit` as written** — resets a shared command pool on every call
  and blocks on a fence (`VulkanDevice.cpp:670-696`). Replaced by per-thread
  staging plus the graph's copy pass.

## 11. Ported as-is, deliberately

- `SyncTier` + core-promotion-aware `Supports()` (`VulkanExtensionCheck.h:12-17,128`).
- The graph refusing to expose a barrier API to passes
  (`VulkanRenderGraphExecutor.h:106-134`) — this is what makes rule 3 hold.
- `UheJobsystem` + `TaskGraph`, including last-arrival dispatch and the documented
  memory-ordering argument (`docs/architecture/jobsystem.md:196-221`).

---

## 12. Open items

1. Graph **cull** root policy: imported roots only, or do persistent (non-transient)
   textures pin their producers?
2. Physics fixed-step rate — 120 Hz for robotics stability, or 60 Hz with per-backend
   substepping?
3. Port `SceneSerializer` (YAML) at all, or design the robotics scene format once?
4. Does `bifrost` own the mesh/GPU-resource cache, or does `vault` own GPU buffers and
   hand them out? Currently unstated and it decides the `AssetSource` trait shape.