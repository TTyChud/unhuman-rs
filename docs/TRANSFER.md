# Transfer: the whole C++ engine

Authoritative work list, derived from actual surveys of
`../unhuman/UHE/src/` and the `UHE_EDITOR/` tree. **Phase order is in
`session.md`. This file is the inventory and the decisions.**

Every finding below came from reading the C++, not from remembering it. Several
things were found that were expected to work and do not.

---

## 0. Read this first — four things that change the plan

### The animation branch

`upstream/improve-animation-and-compression` @ `9177d5c` is **not an ancestor of
HEAD**. It is a finished, tested superset: +2,136 lines adding
interpolation modes, loop modes, cross-fade, events, root motion, multi-skin,
an animation state machine, a compressor, and an IK solver.

**Decided: port `9177d5c`, not HEAD `871d3f0`.** Porting HEAD would port code
that branch already obsoletes. See `session.md` Phase 8 for what that changes.

### `Renderer2D/Renderer2D.cpp` does not compile

`UHE/CMakeLists.txt:12` excludes it. It is also internally broken —
`Renderer2DData::VertexArray`/`VertexBuffer` are commented out at `:35-36` but
assigned at `:67`/`:73`.

**The live 2D renderer is `Renderer/Renderer2D.{h,cpp}`** (56 + 699 lines), which
`Scene.cpp:9` actually includes. The dead directory contributes the *batching
design* (20k-quad CPU arena, 32 texture slots, per-vertex `TexIndex`) plus the
camera and `SubTexture2D` code, which do compile.

Two classes named `UHE::Renderer2D` exist, one per header, and `UHE.h:32`
includes the dead one — an ODR violation.

### Editor gaps, ranked

- **Undo/redo is entirely absent.** Every `InputText`, `DragFloat`, gizmo drag
  and component add is un-undoable. Largest single gap.
- **A synchronous `ReadPixel` every frame** for hover-picking (`Editor.cpp:533`).
  A full GPU→CPU stall to hover an entity.
- **`Q` sets `m_GizmoType = 1`** — a literal colliding with
  `ImGuizmo::TRANSLATE`. Menus and shortcuts hardcode items in two places and
  have drifted.
- **Unbalanced ImGui stacks.** A stray `PopItemWidth()` with no matching push
  (`SceneHierachyPanel.cpp:829`) corrupts widths for the rest of the window.
- **`typeid(T).hash_code()` as a tree-node ID** (`:406`) — not stable across
  builds. Reorder the component list and collapsed sections lose state.
- **Q/W/E/R fire while typing** — `WantCaptureKeyboard` is never checked.
- `imgui.ini` is **0 bytes**; no default layout, no persistence path.

### Things that are dead code

- **Audio has zero call sites.** `PlaySound2D`, `PlaySound3D`,
  `SetListenerPosition` are never called outside `Audio/`. `Update()` reaps a
  permanently empty vector. Only `Init`/`Shutdown` are wired.
- **`ShaderManager` and `VulkanShaderManager` are empty stubs.** There is no
  shader cache; the real path is `SlangCompiler::CompileToSPIRV` at renderer
  init only.
- **`RigidBody3DComponent::AllowedDOFs`** is serialized, deserialized and copied —
  and never applied to Jolt.
- **`BoxColliderComponent::RuntimeShape`** is declared and never written.
- **`Utils/ObjectPool.h` is a 0-byte file.** `Platform/Linux/Linux.cpp` is 6
  empty lines.

---

## 1. Library choices — corrected

An earlier version of this file was written from memory. The surveyed versions
correct five entries. **Versions verified against crates.io, C dependencies
verified against actual `build.rs` files.**

| Need | Crate | Version | Native? |
|---|---|---|---|
| ECS | **hecs** | 0.11.2 | pure |
| 3D physics | **rapier3d** | 0.36.1 | pure |
| 2D physics | **parry2d** | 0.31.1 | pure |
| Math | **glam** | 0.34.1 | pure |
| Window | **winit** (+ `raw-window-handle` 0.6) | 0.30.13 | pure |
| UI | **egui + egui-wgpu** | 0.37.0 | pure |
| Audio | **rodio** | 0.23.0 | **ALSA C on Linux** |
| glTF | **gltf** | 1.4.1 | pure |
| Logging | **tracing** | 0.1.44 | pure |
| Image | **image** | 0.25.10 | pure |
| Profiling | **profiling** facade + puffin | 1.0 / 0.20.0 | pure |
| YAML | **yaml_serde** | 0.10.7 | pure |

### Corrections to the earlier draft

- **`fastgltf` is not a Rust crate at all.** It is a C++20 header-only library.
  Absent from crates.io; `cargo add fastgltf` fails. The crate is `gltf`.
- **`serde_yaml` is deprecated** (archived 2024-03) *and* is the only YAML option
  carrying a C dependency (`unsafe-libyaml`). `serde_yml` is also deprecated.
  `yaml_serde` is the org-endorsed continuation and is pure Rust.
- **`egui_gizmo` is stale.** It depends on egui 0.26 and glam 0.25 against
  current 0.37 / 0.34 — adding it gives two glam versions and an egui eleven
  releases behind. **Draw gizmos ourselves**; we need a line pipeline anyway.
- **Neither `kira` nor `rodio` uses miniaudio.** Both go through `cpal`, which on
  Linux resolves to `alsa-sys` — a C binding. On Windows and macOS both are
  pure. **There is no pure-Rust Linux audio output path**; ALSA is a C system
  library and this is the one documented exception.
- **`jolt-physics` is unusable here.** 0.1.x, all earlier versions yanked, and
  `jolt-sys` needs `cmake` + `bindgen` + a C++17 compiler with no prebuilt
  escape. It would break the "cargo test works with no cmake" invariant.
  `rapier3d` is the answer.
- **`flecs` is abandoned** — last published 2023-10, and it vendors 52k lines of
  C.

### Two real impedance mismatches

**rapier defaults to `nalgebra`, not `glam`.** An opt-in `glamx` feature exists
but resolves glam 0.33, not 0.34 — so enabling it puts a second, older glam in
the graph. With defaults there is a conversion at every physics boundary. Keep
glam canonical; convert in `femur` only.

**`gltf` does no optimisation.** It parses. `meshoptimizer` is a separate step and
the `meshopt` crate compiles 16 `.cpp` files with `cc`. Start by hand-rolling
`optimize_vertex_cache` (tipsify), `optimize_vertex_fetch` (one linear pass) and
`optimize_overdraw` (depth prepass) — a few hundred lines of pure Rust covers
most of the win. Revisit `meshopt` only if profiling says 30%+ is being left.

### The window serves both backends

winit 0.30 implements `HasWindowHandle`, so **one window gives both** a raw
`RawWindowHandle` for ash *and* a `wgpu::SurfaceTarget`. Pin
`raw-window-handle = "0.6"` to match winit's `rwh_06`; a mismatch is the
classic failure here.

`eyes` therefore depends on `raw-window-handle` but **not** on `ash` or `wgpu`:
it hands over a handle and each backend decides what it is. Putting a
`wgpu::Surface` in `eyes` would be the same mistake as naming `vk::` in
`bifrost`, one layer earlier.

**Windows caveat:** winit 0.30 takes `any_thread` at *event-loop construction*,
not per call, so it has to be set in `eyes::winit_config`. Without it winit
refuses a raw handle to any thread but the creating one — and both backends
build their surface from the frame callback, which is not that thread. The
failure is a panic inside `create_surface` naming neither winit's rule nor the
cause, and **only on Windows**, so a Linux test run passes every time.

**Threading, which changed the design.** `EventLoop` is not `Send` and on macOS
must own the main thread, so it cannot be moved to a render thread. The window
therefore *owns the frame loop*: `Window::run` hands control to winit and invokes
the frame callback from inside it. That is what makes the C++'s one-frame-late
input structurally impossible rather than a convention — and it is why `eyes`
has a `HeadlessWindow`, so the loop can be tested at all.

### egui runs on our own render target

`egui-wgpu` takes `&mut RenderPass<'static>` and its docs say *"executes onto an
existing wgpu renderpass"* — which is exactly our
`RenderPass::forget_lifetime()` pattern. `register_native_texture` lets our
renderer feed its own textures in.

But `egui-wgpu` is welded to **wgpu ^30**, so there is no equivalent for the ash
backend.

**Decided: both, wgpu first.** Ship the editor on wgpu to get it working, then
add egui tessellation through `bifrost` so the Vulkan backend can host UI too.
The tessellation path is the same work either way; doing it second means the
editor exists before it needs it.

---

## 2. Events or messages

The C++ has an event bus (`EventType` enum, category bitflags, macros,
`EventDispatcher`). Keep it, and keep it small.

| | event | message |
|---|---|---|
| shape | immediate, broadcast, `dyn` | queued, typed, deferred |
| consumers | many, none know each other | exactly one, named |
| ordering | does not matter | matters |
| example | window resized, key pressed | texture reloaded, scene loaded |

`pings` has **two types, not one**. The mistake is a single `mpsc` for both — a
queue pretending to be a bus, which makes "who handled this?" a runtime question.

---

## 3. File size and layout

**Decision, per instruction: one concern per file, ~300 lines maximum, divided
by responsibility rather than mirroring C++ translation units.** The C++ 1:1
rule applies to the Vulkan backend, which already follows it.

Shape every new subsystem the same way:

- **data** — plain structs, no I/O, fully unit-testable
- **policy** — pure functions over that data
- **io / GPU boundary** — the only files touching a driver
- **façade** — owns the rest; the only file allowed to hold several nouns

Verified examples from the surveys: animation splits into `bone`, `skeleton`,
`clip`, `track`, `sample`, `pose`, `playback`, `blend`, `animator`, `skinning`,
`ik`, `state_machine`, `compress`, plus an `import/` subtree. `meats` splits into
`hierarchy`, `hierarchy_ops`, `world_transform`, `scene`, `script` and a
`components/` directory of one file per component. `mirror` splits into
`theme/`, `layout/`, `input/`, `widgets/`, `log/`, `files/`.

---

## 4. Subsystem notes

### Animation

C++: ~1,720 lines, but only ~240 are the engine — the rest is glTF ingest and
ECS plumbing. The `Animator` is O(N²): it finds children by scanning every bone
per bone, and looks up tracks by linear scan per bone per channel. Rust: build
`children` once at load, use a track index, walk iteratively rather than
recursively.

Also: `fmod(t, Duration)` with no zero guard (a static clip produces NaN), and
an empty track underflows `size() - 1`.

### Scene / ECS

20 components, not 19. EnTT 3.16 vendored, views only, never groups. Hierarchy
is **UUID-keyed, not entity-keyed**, deliberately — so it survives serialization.
Keep that.

Three things to drop: the `storage::<T>()` priming loop (a DLL workaround with
no Rust analogue), the 21 `OnComponentAdded` specializations (20 are empty), and
the 220-line hand-written `Scene::Copy` (derive `Clone`, keep three lines).

Keep verbatim: the deferred-mutation queue. hecs swaps storage on `insert` just
like EnTT on `emplace`, so the invalidation hazard is identical.

### Core / windowing

`Application` is a singleton with `Input` reaching through it to a raw pointer —
untestable as written. Rust: inject `&mut dyn Window`, `&mut dyn Clock`,
`&mut dyn RenderScope`; delete the singleton; `eyes` owns an `InputState` updated
*by* the event pump so polling reads memory, not the OS.

Add `HeadlessWindow` — C++ has no equivalent, and without it `eyes` cannot be
CI-tested at all.

Two bugs worth carrying the lesson from: `SetVSync` sets a bool and never calls
`glfwSwapInterval` (so VSync is a no-op), and input events are polled *after*
rendering so they land a frame late.

### Editor / UI

Two competing theme systems and the engine's hardcoded one wins. The editor's
`EditorTheme` (12 themes, semantic tokens, OKLCH-ish derivation, Rec.709
luminance check for accent foreground) is genuinely good — it is just an
editor-only override of an engine palette, so a non-editor build silently gets
the other one.

Rust `mirror` fixes that: semantic colour tokens in the library, one derivation
path for light and dark, named font roles instead of `Fonts[1]` positional
indexing. Plus the three things that make it better: **undo/redo as a first-class
ECS mutation command**, **one command registry** so menu/shortcut/palette cannot
drift (kills the `Q` bug permanently), and **async picking** instead of
`ReadPixel` per frame.

### Physics

C++ shows 270 lines but ~250 of the real logic lives inside `Scene.cpp`. The
port's main job is *extracting* it.

Variable timestep with no accumulator and `collision_steps` hardcoded to 1.
Sync-back skips `!IsActive(bodyID)` — **sleeping bodies never refresh**, so a
body that falls asleep visually freezes. Decide whether to preserve that.

2D: Box2D v3 is fully wired, **box-only** (no shape priority chain unlike 3D),
4 substeps, and `Rotation` documented as degrees but fed to `b2MakeRot` as
radians — only correct at z = 0.

### Assets

No shader cache (both ShaderManagers are stubs). No VFS — `FileSystem` is a
singleton with one `m_RootPath` and `Resolve` is `root/assets/v`, a hardcoded
segment, which most call sites bypass anyway.

**Loading is 100% synchronous on the calling thread.** `Jobsystem` exists and no
loader uses it. The one real cache in the whole codebase is `Font2D::s_Cache` —
that is the pattern to extract into `vault`.

### Debug / Utils

Both effectively empty. `TracySetup.h` defines profiling macros as no-ops
unless `UHE_PROFILE`, and nothing ever defines it.

---

## 5. What is not a transfer

- **OpenGL.** No GL loader in Rust, and none wanted.
- **`glm`.** Replaced by `glam`, not wrapped.
- **The hand-written render graph barriers.** Ours derives them from declared
  reads and writes; C++ mixes a graph-synthesised barrier with a hand-written
  `oldLayout = eUndefined` in `VulkanCommandBuffer.cpp`. Designed out.
- **The static `Renderer2DData` singleton.** Untestable and non-reentrant.
- **`Framebuffer`.** Dynamic rendering / implicit attachments on both backends.
- **`Utils/ObjectPool.h`.** It is a 0-byte file.
