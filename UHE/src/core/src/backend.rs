//! The substitutable backends the engine owns as trait objects.
//!
//! The traits live here, in `adam`, and not in `femur` or `wire`, for the same
//! reason `chassis::Gpu` lives in the RHI: the engine names an interface and a
//! plugin supplies one implementation, so `adam` has no dependency on rapier or
//! rodio and a build with no physics still compiles.
//!
//! `Send` because a backend is owned by a plugin, and plugins are `Send` — the
//! engine may be moved between threads, and a backend that quietly cannot be is
//! a crash waiting for the first person who does it.

use glam::Vec3;
use meats::Scene;

/// A handle to a playing sound.
pub type VoiceId = u64;

/// Where the ear is and which way it faces, in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
}

impl Listener {
    pub fn new(position: Vec3, forward: Vec3, up: Vec3) -> Self {
        Self {
            position,
            forward,
            up,
        }
    }
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            up: Vec3::Y,
        }
    }
}

/// Rigid-body simulation.
///
/// One method, because everything else is a question about the scene: bodies,
/// colliders and transforms are `meats` components, and a backend's job is to
/// bring the simulation in line with them and write the result back.
pub trait PhysicsBackend: Send {
    fn name(&self) -> &'static str;

    /// Advance `dt` seconds and write the moved transforms back into `scene`.
    fn step(&mut self, scene: &mut Scene, dt: f32);

    /// Bodies currently in the simulation. Reported, not guessed, so a bench
    /// or a status bar can say zero and mean it.
    fn body_count(&self) -> usize;
}

/// Playback.
pub trait AudioBackend: Send {
    fn name(&self) -> &'static str;

    fn listener(&self) -> Listener;
    fn set_listener(&mut self, listener: Listener);

    /// Reconcile the playing voices with the scene's `AudioSource` components.
    ///
    /// Takes `Assets` because a clip is a handle and the bytes live in the
    /// registry: without it a backend can name a sound and never decode one.
    /// The alternative — a shared table the plugin fills in and the backend
    /// reads — is the second-owner pattern the spine exists to remove.
    fn update(&mut self, scene: &Scene, assets: &vault::Assets, dt: f32);

    fn play(&mut self, assets: &vault::Assets, clip: vault::AssetId, gain: f32) -> Option<VoiceId>;
    fn stop(&mut self, voice: VoiceId);

    fn active_voices(&self) -> usize;
    fn master_gain(&self) -> f32;
    fn set_master_gain(&mut self, gain: f32);
}
