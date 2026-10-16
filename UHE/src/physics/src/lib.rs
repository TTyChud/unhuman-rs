//! Physics: the rigid-body backend, as a Plugin.
//!
//! Port of `UHE/src/UHE/Physics/PhysicsSystem3D.cpp,h` and of the two places the C++
//! kept the real work — body creation in `Scene::OnRuntimeStart`
//! (`Scene.cpp:1368-1450`) and the transform write-back in `Scene::OnUpdateRuntime`
//! (`Scene.cpp:1254-1279`) — onto Rapier instead of Jolt.
//!
//! `tests/physics.rs` is the port's own gate: gravity over N steps, a box coming to
//! rest, two identical runs agreeing bit for bit, a static body that does not move, and
//! an entity without a rigid body that `step` does not touch.

mod physics_system3d;
mod plugin;
mod shape;

pub use physics_system3d::{BodyHandle, Physics3D, GRAVITY};
pub use plugin::FemurPlugin;
pub use shape::ColliderShape;
