//! `femur` — physics.
//!
//! rapier3d for 3D, rapier2d for 2D. Both are the same codebase with a
//! different dimension, so both appear here; neither is `jolt-physics`, and the
//! reason is not taste.
//!
//! # Why rapier and not jolt-physics
//!
//! `jolt-physics` on crates.io is 0.1.x with every earlier version yanked, and
//! its `jolt-sys` build needs `cmake`, `bindgen` and a C++17 compiler, with no
//! prebuilt escape. Adopting it breaks the one invariant this engine has held
//! since the first commit: **`cargo test` works on a machine with no cmake and
//! no Vulkan SDK.** A physics engine is not worth that. See `docs/TRANSFER.md`
//! §1.
//!
//! # Two impedance mismatches, both designed around rather than wished away
//!
//! **rapier is a `nalgebra` library. We are a `glam` library.** rapier does
//! offer an opt-in `glamx` integration and it must not be enabled: `glamx`
//! 0.3 resolves glam **0.33** against our **0.34**, so turning it on does not
//! make the types agree — it puts a second, older glam in the build graph and
//! still leaves a `glam::Vec3` from `bifrost` unable to typecheck as a rapier
//! vector. That is the same trap `egui_gizmo` sets (glam 0.25, egui 0.26), and
//! it is why [`convert`] exists: `glam` is canonical, conversion happens at
//! this crate's boundary, in **one module**, and `tests/boundary.rs` greps the
//! sources to keep it that way. Nothing in `femur`'s public API mentions
//! `nalgebra`.
//!
//! **The C++ keeps physics inside `Scene.cpp`.** About 250 of its 1,458 lines,
//! interleaved with rendering and scripting, mutating `TransformComponent` in
//! place. Here physics is extracted: the port's job was never transliteration,
//! it was getting the solver out from behind the scene graph.
//!
//! # The three upstream bugs, and what was done about each
//!
//! | Upstream | Here |
//! |---|---|
//! | `if (!bodyInterface->IsActive(bodyID)) continue;` — a sleeping body's pose is never read back, so its transform freezes wherever the ECS last had it | [`sync`] reads every pose unconditionally and reports `sleeping` as data. See that module for the full argument. |
//! | `RigidBody3DComponent::AllowedDOFs[6]` is serialized, deserialized, copied in `Scene::Copy`, and never applied | [`solver::Dofs`] is applied per axis at spawn and on demand, and locking scrubs the velocity on the locked axes. |
//! | `BoxColliderComponent::RuntimeShape` is declared and never written | It has no counterpart: runtime handles live in [`registry`], keyed by entity, so a scene file contains nothing solver-specific. See [`colliders`]. |
//!
//! Plus two that TRANSFER.md names rather than PARITY.md:
//!
//! - **Variable timestep with no accumulator, `collision_steps` hardcoded to
//!   1.** [`timestep::FixedStep`] is a fixed step with an accumulator and a
//!   documented substep clamp, so a hitch cannot spiral.
//! - **2D rotation in degrees fed to a radians API.** Correct only at `z = 0`,
//!   which is why nobody noticed. femur's 2D API is radians throughout.
//!
//! # Layout
//!
//! One concern per file, ~300 lines each, split by responsibility rather than
//! mirroring C++ translation units.
//!
//! | file | what it owns |
//! |---|---|
//! | [`convert`] | the only place `glam` and `nalgebra` meet |
//! | [`timestep`] | the fixed step, the accumulator, the substep clamp |
//! | [`solver`] | constraints: per-axis DOF locks, kinematic targets, gravity |
//! | [`bodies`] | what a body is, how one is built, how mass is divided |
//! | [`colliders`] | box / sphere / capsule in 3D, box in 2D |
//! | [`registry`] | key → solver handle; femur's only owner of runtime state |
//! | [`sync`] | reading poses back, and the sleeping-body rule |
//! | [`world`] | the 3D façade: spawn, step, read |
//! | [`world2`] | the 2D façade, same policy over rapier2d |
//!
//! [`timestep`] and [`solver`] began as one file and [`world2`] began inside
//! [`world`]; both were split because the file that held the fixed-step clock
//! and the per-axis locks was over 500 lines, and "the ceiling is 8" and "X
//! rotation is locked" are not the same concern.
//!
//! # Deliberately not here
//!
//! - **Collision filtering / interaction groups.** Upstream's two Jolt object
//!   layers collide with everything, which is rapier's default too. There is no
//!   layer scheme to map onto yet.
//! - **Continuous collision detection as a per-body option.** rapier's
//!   `max_ccd_substeps` defaults to 1 — the same number the C++ hardcodes — so
//!   leaving it alone is parity, not an omission.
//! - **Joints.** The C++ creates none.
//! - **A scene graph.** A physics pose is world space. Who composes it with a
//!   parent is not a physics question, and answering it here would make femur
//!   know what a scene is.

#![deny(missing_docs)]

pub mod bodies;
pub mod colliders;
pub mod convert;
pub mod registry;
pub mod solver;
pub mod sync;
pub mod timestep;
pub mod world;
pub mod world2;

pub use bodies::{BodyDesc, BodyDesc2, BodyId, BodyKind, SpawnError};
pub use colliders::{ColliderDesc, ColliderDesc2, Shape};
pub use convert::{Pose, Pose2};
pub use solver::{Dofs, Dofs2};
pub use sync::{BodyState, BodyState2, SyncStats};
pub use timestep::{FixedStep, StepPlan, StepStats};
pub use world::{PhysicsWorld, ReparentError};
pub use world2::PhysicsWorld2;
