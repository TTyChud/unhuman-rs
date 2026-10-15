# unhuman-rs

The Rust rewrite of [Unhuman Engine](https://github.com/unhuman-engine/UNHUMAN) — a
real-time engine for robotics simulation, synthetic data generation and games.

The C++ tree is the specification; this repository is the port, and it is a port with
opinions: the five structural failures `docs/architecture/00-spine.md` documents in the
C++ (ambient globals, a hollow RHI abstraction, two owners of synchronization, a render
graph whose parallelism was thrown away by one shared command buffer) are the reason
each subsystem here has the shape it has.

## Layout

```
UHE/src/core/          adam       Engine, App, the plugin registry, the loop
UHE/src/rhi/           chassis    the backend-neutral RHI and the render graph
UHE/src/jobs/          grind      job system and task graph
UHE/src/shaders/       forge      Slang and WGSL in, SPIR-V out
UHE/src/events/        pings      event bus, key and mouse codes
UHE/src/platform/      eyes       window and input (winit)
UHE/src/scene/         meats      ECS, components, hierarchy, .unhuman serialisation
UHE/src/assets/        vault      asset registry, VFS, loaders
UHE/src/renderer/      bifrost    the renderer frontend
UHE/src/audio/         wire       audio
UHE/src/physics/       femur      rigid bodies and colliders
UHE/src/editor/        mirror     the editor model
UHE/Stormbreaker/wgpu/    stormglass    the wgpu backend (default)
UHE/Stormbreaker/vulkan/  stormbreaker  the Vulkan backend (opt-in)
UHE/bins/               nocap (headless), sandbox (windowed), editor (gpui)
UHE/games/aimlab/       the UHEGAME demo, as a plugin
```

`docs/architecture/01-crate-map.md` is the interface contract every crate is written
against: the seams, the dependency direction, the things deliberately not ported, and
why. `docs/WORK.md` is the state of the work and the rules this repository runs by.

## Build

```bash
cargo build --workspace            # logic crates only; no GPU, no Vulkan SDK needed
./scripts/gate.sh                  # the whole gate: build, clippy -D warnings, tests
./scripts/gate.sh --no-vulkan      # on a machine with no driver
```

`scripts/gate.sh` is the gate: a green run locally means a green run in CI. Two rules it
encodes are there because getting them wrong produces a green run that means nothing —
validation is enabled only through `DeviceOptions::from_env`, and the suites run with
`--test-threads=1` because they share one device and one scratch directory.

Backends: **wgpu by default, Vulkan opt-in**, chosen by `--backend`, `UHE_BACKEND` or a
`backend` key in the settings file. That is a stability decision, not a preference: when
a frame is wrong the first question is "the backend or the engine".

## Not ported

Listed with a reason each in `docs/architecture/01-crate-map.md` — the C++ render graph's
legacy `BeginRenderPass`, cached graph compilation, the excluded 2D renderer, the
vestigial Linux platform layer, `basis_universal` transcoding, runtime MSDF atlas
generation (the runtime path is a coverage atlas; the MSDF path reads a pre-baked one),
and Tracy/spdlog.

## Status

The whole C++ tree outside `Platform/Vulkan` has a Rust counterpart. **This tree has not
been compiled since the port landed**: the environment it was written in had no
compiler available, so the next step is `cargo build --workspace` and then
`./scripts/gate.sh`, and anything that fails there is a fix, not a redesign — the seams
are documented and the tests exist.
