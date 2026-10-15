//! Renderer frontend. Backend-neutral: never names `vk::` or `wgpu::`.
//!
//! # The rule this crate exists to enforce
//!
//! **`bifrost` never names `vk::` or `wgpu::`, and never depends on
//! `stormbreaker` or `stormglass`.** It talks to `chassis` and nothing else.
//!
//! This is not a style preference. The C++'s RHI layer is
//! `dynamic_cast`-straight-through to the driver (`RHITypes.h:37-41` has `void*`
//! handles, and `ImGuiLayer.cpp:11` casts through the "backend-agnostic" layer), so
//! every renderer bug in it is ambiguous between "the renderer" and "the backend".
//! Two backends cannot be built on that, because there is only one to abstract.
//! The dependency list below is the whole argument, and it is what keeps the
//! argument true rather than aspirational: adding `ash` here would make the rule a
//! matter of review instead of a matter of the compiler.
//!
//! The mirror of the rule is one layer down — `eyes` depends on
//! `raw-window-handle` but not on `ash` or `wgpu`, and hands the same handle to
//! both backends (`docs/TRANSFER.md` §1). Put a `wgpu::Surface` in `eyes` and the
//! same mistake appears one layer earlier.
//!
//! # What is here, and in what order to read it
//!
//! | file | what it decides |
//! |---|---|
//! | [`frame`] | one frame: the graph, the imports, begin and end |
//! | [`pass`] | a pass: a closure **and** the read/write set that declares what it touches |
//! | [`drawlist`] | what to submit, in what order, batched |
//! | [`sort`] | opaque front-to-back, transparent back-to-front |
//! | [`replay`] | issuing those draws without the redundant binds |
//! | [`cull`] | camera, frustum, and the CPU cull |
//! | [`shadow`] | orthographic fitting for a directional cascade |
//! | [`bounds`] | the sphere every one of those tests uses |
//! | [`stats`] | the counters that make any of it debuggable |
//!
//! The one idea that ties them together: **the declaration is the contract.** A pass
//! declares what it reads and writes, and the graph derives every ordering, every
//! transition and every barrier from those declarations — there is no barrier API
//! to reach past them with. A draw list declares its order and its batching, and
//! the counters say how much of that the driver actually had to act on. Every
//! hazard this crate could have is one where something was used but not declared,
//! which is why [`pass`] spends more words on the declaration than on the closure.
//!
//! # Phase status
//!
//! Phase 1 and Phase 5's frontend half. Present and unit-tested: the frame, the
//! pass set, import helpers, draw lists, sorting, batching, culling, counters.
//!
//! Not present, and not faked: mesh upload and `ResourceCache` (Phase 2/3, so a
//! `DrawResolver` is a trait and every handle a test double), materials and shader
//! variants (Phase 4), lighting (Phase 6), post (Phase 7), and the cross-backend
//! proof — the same frame recorded by both backends producing the same image —
//! which needs a backend to hand this frame a recorder at all, and
//! [`chassis::Gpu`] has no method for that. See [`frame`]'s module docs.

pub mod bounds;
pub mod cull;
pub mod drawlist;
pub mod frame;
pub mod pass;
pub mod replay;
pub mod shadow;
pub mod sort;
pub mod stats;

pub use bounds::Sphere;
pub use cull::{cull, Camera, Frustum, Projection};
pub use drawlist::{BatchMode, DrawCommand, DrawItem, DrawList, MaterialId, MeshId, PipelineId};
pub use frame::{Frame, FrameError, FrameOutput};
pub use pass::{Pass, PassBuilder};
pub use replay::{DrawCount, DrawResolver};
pub use shadow::{directional_shadow, Cascade, ShadowCamera};
pub use sort::{sort, sort_opaque, sort_transparent, DepthOrder};
pub use stats::{CullingStats, FrameStats, StateChanges, StateTracker};
