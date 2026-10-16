# Parity: what "the same level" means, item by item

Target: `unhuman-engine/UNHUMAN` @ `upstream/master` = **780d010** (2026-10-12).

**Our local C++ checkout is 42 commits behind master** (it sits at `871d3f0`),
so everything below was verified against a fresh `git archive` snapshot, not the
local tree. Every plan written before this date was built against code the
project had already replaced.

---

## 0. Read this before trusting the C++'s own docs

The upstream `IDEA.md` and `docs/ROADMAP.md` are **stale in three places**, and
trusting them would mean porting less than the engine actually does:

| Upstream claim | Reality, verified in code |
|---|---|
| `KHR_texture_transform` still open | **Implemented and tested.** `LoadModel.cpp:312-324, :461-475` → `MaterialGPU.cpp:92-94` → shader |
| sampler wrap/filter on load still open | **Implemented and tested.** `LoadModel.cpp:223-293` |
| mipmaps on the model path still open | **Implemented.** GPU blit `VulkanTexture.cpp:194,:348-396`; authored KTX2 chain `:228-286` |
| `node.transform` ignored, multi-node models collapse (M3 step 1) | **Implemented.** `LoadModel.cpp:805`; asserted `tests/gltf/main.cpp:1223-1307` |
| `COLOR_0` still open | Half-true: parsed and tested, but the flag is **dropped before draw** — see §4 |
| "no in-tree unit tests" | **False.** 7 CTest harnesses registered |

Only `alphaMode: BLEND` pass ordering and Draco are genuinely open upstream.

---

## 1. glTF: the parity list

### 1.1 Extensions — 15, all reaching the renderer

Core: metallic-roughness, node hierarchy, shared-mesh instancing, non-indexed
geometry, scene selection.

| Extension | Read at |
|---|---|
| `KHR_texture_transform` | `LoadModel.cpp:312-324` |
| `KHR_materials_unlit` | `:580` |
| `KHR_materials_emissive_strength` | `:573` |
| `KHR_materials_ior` | `:576` |
| `KHR_materials_clearcoat` | `:594-611` |
| `KHR_materials_specular` | `:613-624` |
| `KHR_materials_sheen` | `:626-636` |
| `KHR_materials_transmission` | `:641-650` |
| `KHR_materials_volume` | `:651-660` |
| `KHR_materials_iridescence` | `:663-675` |
| `KHR_materials_anisotropy` | `:677-685` |
| `KHR_materials_diffuse_transmission` | `:691-704` |
| `KHR_lights_punctual` | `:730-764` |
| `EXT_meshopt_compression` | `MeshoptDecode.cpp:27-176` |
| `KHR_texture_basisu` | `LoadModel.cpp:371-372` |

Deliberately unsupported, and must warn by name: `KHR_draco_mesh_compression`.
Anything else declared is **reported by name**, never silently dropped — that
is the single behaviour worth copying first.

### 1.2 Vertex format

96-byte stride: `position vec3` · `normal vec3` · `uv vec2` (V flipped) ·
`tangent vec4` (xyz + handedness) · `jointIndices ivec4` · `jointWeights vec4` ·
`color vec4`.

Synthesized when absent, and this matters:
- **NORMAL** — area-weighted, degenerate → `(0,1,0)` (`LoadModelGeometry.cpp:269-293`)
- **TANGENT** — generated, must be **unit length and perpendicular to the normal** (asserted upstream)
- **COLOR_0** — VEC3 gets alpha 1; default is opaque white, never black

### 1.3 The meshopt pipeline, and its exact order

The loader calls four passes, **in this order**, and the order is load-bearing:

1. `generateTangents` — **first**, because the remap hashes every byte and
   would collapse UV seams
2. `optimizeVertexCache` — cache-friendly index order
3. `generateVertexRemap` + `remapIndexBuffer` — merge duplicates,
   **skipped entirely for skinned primitives**
4. `optimizeVertexFetchRemap` + `remapIndexBuffer` — **after** the reorder

Upstream's own test harness learned three things the hard way, and they are
worth porting as test design:
- a UV sphere is **not** a valid merge test — seam vertices share position and
  normal but differ in UV, so "no reduction" is the correct answer
- reversing triangle order is **not** a scramble for a grid mesh; consecutive
  triangles stay adjacent, so any ACMR assertion on it is vacuous
- `simplify` treats `target_index_count` as a budget to **meet**, not a floor;
  with a generous error bound it may legitimately undershoot

**Not load-time in upstream, so not parity-required:** `optimizeOverdraw`,
`spatialSortRemap`, `simplify` (LOD), `analyzeVertexCache`, `analyzeVertexFetch`,
`computePositionExponent`. Those exist only in `tests/meshopt`. **There is no
LOD generation anywhere in the engine.**

### 1.4 Textures

- KTX2/Basis → RGBA8, **every level of the authored chain preserved**
- sRGB **per texture slot**: albedo/emissive/specular-colour/sheen-colour are
  sRGB; metallic-roughness/normal/occlusion/every Tier-2 slot is linear
- Sampler: `magFilter`/`minFilter` → Nearest/Linear, `wrapS`/`wrapT`
  **independently**; defaults Linear / LinearMipmapLinear / Repeat / Repeat
- Dedup keyed on `(image, colourSpace, samplerIndex)` — colour space is in the
  key because one image legitimately serves both
- Five image sources handled, including `BufferView` (so a meshopt-compressed
  atlas page decodes)

---

## 2. Assets: the parity list, and what upstream actually wired

Upstream's `AssetManager` is **complete, correct, tested — and called by
nothing.** Zero call sites from engine, editor or game. `Update()` is never
pumped. There is no file watcher. Only `TextLoader` and `BinaryLoader` exist,
so hot reload can only ever fire for file types the engine does not use.

Parity means the **working** version, not the coded one:

- [ ] Content-addressed identity. Upstream's `AssetID` is a fresh random UUID
      per run and its header claims it survives renames; **it does not**, and
      there is no manifest. Use a content hash.
- [ ] Real loaders for texture, model, audio, shader, scene — the five
      `AssetType`s upstream declares and implements none of.
- [ ] Async on `grind`, with `Finalize` doing GPU work on the main thread
- [ ] Hot reload that is **actually reachable** from a frame loop, and a real
      watcher rather than a `stat` poll under the registry mutex
- [ ] Dependency edges between assets (upstream has the field, never writes it)
- [ ] VFS with more than one mount. Upstream has exactly one (`game:`), no
      `engine:`, and "layering" is a comment — `Mount` replaces rather than
      stacks.

---

## 3. Animation: the parity list

Decided: port `9177d5c` (already merged into master).

**Data model**: `Bone` · `Skeleton` · `Skin` · `Keyframe<T>` · `VectorTrack` ·
`QuaternionTrack` · `AnimationEvent` · `AnimationClip` · `BonePose` ·
`AnimationPose`. glTF's `(node, path)` channel is flattened at load into three
track vectors; duplicate channels merge.

**`Animator` — production-shaped, genuinely used.** O(N + T log K): child lists
built in one pass and cached on size change; track lookup is a binary search per
track. Cross-fade, root motion, events, three loop modes (None/Loop/PingPong),
three interpolations (Step/Linear/CubicSpline), skin switching.

**`AnimationStateMachine` — real, but single-layer and test-only.** bool/i32/f32
parameters plus a separate trigger table; AND-combined conditions; first-match
wins in declaration order (no priority field); exactly one transition per update.
No hierarchical states, no blend trees, no per-state layers.

**`AnimationCompressor` — not ACL, and not a codec.** A greedy chordal-error
keyframe reducer over in-memory `f32` keys; closest to a single-pass
Douglas-Peucker. **No quantization at all** — 16 B/20 B keys stay float. Errors:
`2*acos(|dot|)` for rotations, Euclidean for vectors. Tolerances: 1e-3 position,
1e-4 scale, 0.02 rad rotation. Step tracks are never reduced. Parallel per track.

**`IKSolver` — two-bone analytic, not FABRIK.** Law of cosines + pole vector,
reach clamped into the annulus `|l1−l2| < d < l1+l2` so out-of-reach targets
straighten rather than flip. No chain length other than 2, no joint limits.

**All three are called by nothing but their test.** Keyframe reduction is never
applied at load, so every model ships full-rate.

---

## 4. Bugs to fix rather than copy

Every one of these is live in upstream master. A faithful port would inherit
them, and a faithful *test suite* would pass while they stayed.

### glTF

| Bug | Location | Effect |
|---|---|---|
| `hasVertexColor` never copied from `Geometry` to `Mesh` | `LoadModel.cpp:872-883` vs `Renderer3D.cpp:415` | `COLOR_0` is parsed, uploaded, and then discarded at draw. Upstream's test only checks it on `GetGeometry()`, so it cannot see this |
| `primitive.mode` is never read | no field on `Primitive` | POINTS/LINES/TRIANGLE_STRIP/FAN all load and draw as TRIANGLE_LIST, silently |
| `JOINTS_1`/`WEIGHTS_1` dropped, and `IsSkinned` does not detect them | `LoadModelGeometry.cpp:41-45` | a JOINTS_1-only primitive is treated as non-skinned and destructively merged |
| `KHR_materials_pbrSpecularGlossiness` listed as supported, only `diffuseTexture` read | `LoadModel.cpp:45` vs `:344-347` | renders as metallic-roughness with no warning |
| `TEXCOORD_1..7` silently ignored | `LoadModelGeometry.cpp:165` | no warning |
| meshopt decode failure zero-fills and continues | `MeshoptDecode.cpp:148-154` | corrupt data yields silent degenerate geometry instead of a load error |
| Mip-selection filters lost — the `SamplerMipmapMode` comment references a field that does not exist | `LoadModel.cpp:240` | `NEAREST_MIPMAP_NEAREST` silently becomes `NEAREST` |
| Texture dedup scoped to one `loadModel` call | `LoadModel.cpp:333` | two models sharing a texture upload it twice |

### Assets

| Bug | Effect |
|---|---|
| `AssetManager` has zero call sites | the whole system is dead code |
| No watcher; `last_write_time` polled on the main thread under `m_Mutex` | hot reload cannot run off-thread as the docs claim |
| `AssetID` header claims stability across renames; IDs are re-minted each run | no manifest exists |
| Two independent refcounts (`shared_ptr` + manual `atomic<u32>`), kept in step by hand | can silently diverge |
| VFS "layering" documented as a feature; `Mount` replaces instead | mod overrides cannot work |
| ~10 engine sites still hand-build `root/assets/...`, bypassing the VFS | the old scheme is still dominant |

### Animation

| Bug | Location | Effect |
|---|---|---|
| `CalculateBoneTransform` is recursive | `Animator.cpp:361-362` | stack depth per bone per frame; a Mixamo chain several hundred deep costs kilobytes of stack per animating entity |
| Missing brace in `Seek` — `m_FinalBoneMatrices.resize` is unconditional | `Animator.cpp:242-244` | truncates after `SetSkin` to a smaller skin; self-healing today, latent |
| Unchecked bone indices in `ComputeFinalMatrices` | `:271, :279` | safe for glTF-loaded models, out-of-bounds for any programmatically built model |
| Compressor is O(N²) | `AnimationCompression.cpp:82-110` | header claims "walks the track once"; a 2000-key track is ~2M evaluations |
| `ExtractRootMotion` scans all tracks every frame to find a pointer that never changes | `Animator.cpp:424-431` | O(T) per frame for a constant |
| `m_CurrentPose = m_RestPose` / `m_PrevPose = m_RestPose` per frame | `:200, :214` | 2–3 O(N) allocations per frame |
| FSM reports a state the animator is not playing when a clip name is unknown | `AnimationStateMachine.cpp:70-72` + `Animator.cpp:93-101` | silent no-op |
| `LoadModelAnimation.cpp` is untested — the harness stubs out `Model::~Model` | `tests/animation/CMakeLists.txt:12-23` | the whole glTF animation parser, CUBICSPLINE tangent decode and skin parsing are unpinned |

Upstream's animation harness uses a **3-bone linear chain with identity bind
poses**, tracks of **≤21 keys**, and calls `Seek` only on identity models.
That is why the brace bug, the recursion depth and the O(N²) are all invisible.
The Rust tests must not have that shape: use a branching hierarchy, a
non-identity bind pose, a multi-segment cubic track, and a long track for the
compressor.

---

## 5. Not parity targets

Upstream has these as empty stubs or unreferenced. Do not port them, and do not
spend time looking for them:

- `ShaderManager::Init()` — empty body
- `FrameGraph` frontend — **deliberately cancelled** by upstream decision D6
- `VulkanComputePipeline` — exists, nothing calls it
- `SSAO.h` — a stub
- `usesSharedGeometry` (always true), `Mesh::ownsGeometryBuffers` (never touched)
- LOD generation — does not exist anywhere, despite meshopt `simplify` being vendored
- terrain, water, SSR — deferred by upstream's own naming