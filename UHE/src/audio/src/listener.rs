//! Where the listener is, and what "to your left" means.
//!
//! This is the piece the C++ declares and never uses. `AudioEngine.h:22`
//! declares `SetListenerPosition(position, forward, up)`, `AimLabLayer.cpp:316`
//! calls it once per frame with the camera's transform, and
//! `MiniAudioBackend.cpp:118-125` forwards all three straight to
//! `ma_engine_listener_set_*` with no validation and no use of its own. So the
//! C++ has a listener, but it has no *model* of one: nothing in the engine ever
//! asks where a sound is relative to the listener, because nothing in the
//! engine ever does its own attenuation or panning either.
//!
//! Here the listener is the thing that turns a world-space `Vec3` into the two
//! numbers the spatial maths needs -- how far, and which side -- and it is a
//! pure function of a transform, so it is testable with no device, no scene and
//! no frame loop.
//!
//! ## Why the basis is rebuilt and orthonormalised rather than used as given
//!
//! `set_listener` takes a forward and an up, and the obvious implementation is
//! to dot the sound direction against them. That is wrong for every real
//! camera, because a look-at matrix does not hand you orthonormal vectors:
//!
//! - a forward built by subtracting two world positions and not normalised has
//!   length 2.7, and `dot(dir, forward) > 1` -- a source that is not to the
//!   left of you panned hard left, then clamped;
//! - an up that is merely *near* the true up, as it is after a look-at with
//!   roll, tilts the whole frame and every pan in the game shifts with it;
//! - an up exactly parallel to the forward, which a gimbal-locked camera
//!   produces, makes `forward × up` the zero vector. Dividing by it yields
//!   `NaN`, and per [`crate::gain`] a `NaN` in the mix is the one failure that
//!   destroys the output rather than one sound.
//!
//! So [`Listener::set_listener`] orthogonalises once per frame and caches the
//! basis, and the degenerate cases fall back to a frame that is merely
//! *arbitrary* rather than one that is `NaN`. A listener with a broken up-vector
//! is a bug worth seeing on screen; a listener that emits `NaN` is a bug you
//! hear as noise.
//!
//! ## Handedness
//!
//! Right-handed, `+Y` up, `-Z` forward, matching `glam` and every camera in the
//! C++. `right = forward × up`, which for `forward = -Z` and `up = +Y` gives
//! `+X` -- correct. The engine uses its own cross product rather than the
//! renderer matrix so that a sound system can be tested against a hand-written
//! transform without a scene, a pipeline or a GPU.

use glam::Vec3;

use crate::gain::clamp_unit;

/// The result of measuring a point against a listener.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpatialSample {
    /// Distance from the listener in world units. `0.0` for a point at, or
    /// within [`crate::gain::DEGENERATE`] of, the listener.
    pub distance: f32,
    /// Which side, as `dot(direction, right)`: **`1.0` hard right, `0.0` dead
    /// ahead or dead behind, `-1.0` hard left**. In `[-1, 1]`.
    pub lateral: f32,
    /// Which way, as `dot(direction, forward)`: **`1.0` dead ahead, `0.0` to the
    /// side, `-1.0` dead behind**. In `[-1, 1]`.
    pub frontality: f32,
}

impl SpatialSample {
    /// A point on the listener: no distance, no lateral offset, fully forward.
    ///
    /// A distinct value rather than `lateral = 0, frontality = 0`, because at
    /// the listener there is no direction at all and picking one would send the
    /// source through the rear cue for no reason.
    pub const AT_LISTENER: SpatialSample = SpatialSample {
        distance: 0.0,
        lateral: 0.0,
        frontality: 1.0,
    };
}

/// A position and an orthonormal frame of reference.
///
/// Cheap to copy: a `Vec3` and three more. Copy it freely rather than reaching
/// through a reference to a shared listener, because the mix reads it once per
/// voice per frame and a borrow is the same cost as a 48-byte copy.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Listener {
    position: Vec3,
    forward: Vec3,
    world_up: Vec3,
    right: Vec3,
}

impl Default for Listener {
    /// At the origin, looking down `-Z`, `+Y` up -- the `glam`/Vulkan
    /// convention. Not "facing `+X`" or "identity": a listener that happens to
    /// match the renderer's forward is one fewer thing to get wrong.
    fn default() -> Self {
        Listener {
            position: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            world_up: Vec3::Y,
            right: Vec3::X,
        }
    }
}

impl Listener {
    /// A listener at the origin looking down `-Z`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Move the listener, keeping its orientation.
    ///
    /// Separate from [`Self::set_listener`] because a game moves the listener
    /// far more often than it turns it, and re-orthogonalising an unchanged
    /// basis every frame is work done for nothing.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = finite_or(position, self.position);
    }

    /// Place and orient the listener in one call. This is the frame-loop
    /// entry point, and it is the direct replacement for the C++'s
    /// `SetListenerPosition`.
    pub fn set_listener(&mut self, position: Vec3, forward: Vec3, up: Vec3) {
        self.position = finite_or(position, self.position);
        let forward = forward.try_normalize().unwrap_or(self.forward);
        let up = finite_or(up, self.world_up);
        self.rebuild_basis(forward, up);
    }

    /// Orthogonalise once, and cache. See the module docs for why this exists.
    fn rebuild_basis(&mut self, forward: Vec3, up: Vec3) {
        let forward = forward.try_normalize().unwrap_or(Vec3::NEG_Z);

        // `right = forward × up`, with two fallbacks for the case where the
        // cross product is degenerate. Both are needed: `forward × Y` is zero
        // only when `forward` is vertical, which is exactly when a
        // pitch-locked camera points at the sky.
        let mut right = forward.cross(up);
        if right.length_squared() <= crate::gain::DEGENERATE {
            right = forward.cross(Vec3::Y);
        }
        if right.length_squared() <= crate::gain::DEGENERATE {
            right = forward.cross(Vec3::Z);
        }
        let right = right.normalize_or_zero();

        // Re-derive up from the two vectors that are known good rather than
        // keeping the caller's: this is what discards roll and scale.
        self.world_up = right.cross(forward);
        self.right = right;
        self.forward = forward;
    }

    /// The listener's position.
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// The normalised forward direction.
    pub fn forward(&self) -> Vec3 {
        self.forward
    }

    /// The normalised, roll-free up direction.
    ///
    /// Not necessarily what the caller passed in: it is `right × forward`, and
    /// the difference between the two is exactly the roll the caller got wrong.
    pub fn up(&self) -> Vec3 {
        self.world_up
    }

    /// The normalised right direction, `forward × up`.
    pub fn right(&self) -> Vec3 {
        self.right
    }

    /// A point's coordinates in listener space: **`+x` right, `+y` up,
    /// `+z` forward**.
    ///
    /// This is `inverse(view) * world` for a camera-shaped view matrix, written
    /// out. It is what a debug overlay draws and what a test asserts against,
    /// and it exists because the transform is the one piece of "3D audio" that
    /// is genuinely 3D and worth being able to look at directly.
    pub fn to_local(&self, point: Vec3) -> Vec3 {
        let delta = finite_or(point, self.position) - self.position;
        Vec3::new(
            delta.dot(self.right),
            delta.dot(self.world_up),
            delta.dot(self.forward),
        )
    }

    /// Measure a world-space point against this listener.
    ///
    /// A point at, or within [`crate::gain::DEGENERATE`] of, the listener gives
    /// [`SpatialSample::AT_LISTENER`] rather than a direction derived from
    /// dividing by zero. Everything else gives the distance and the two dot
    /// products, both clamped to `[-1, 1]` and both `0.0` if they are not
    /// finite.
    pub fn sample_at(&self, point: Vec3) -> SpatialSample {
        let delta = finite_or(point, self.position) - self.position;
        let length_squared = delta.length_squared();
        if !length_squared.is_finite() || length_squared <= crate::gain::DEGENERATE {
            return SpatialSample::AT_LISTENER;
        }
        let distance = length_squared.sqrt();
        let direction = delta / distance;
        SpatialSample {
            distance,
            lateral: clamp_unit(direction.dot(self.right)),
            frontality: clamp_unit(direction.dot(self.forward)),
        }
    }
}

/// Keep a finite value, or fall back.
///
/// Non-finite positions come from a divide by a zero matrix somewhere upstream.
/// Substituting the previous value turns an unplayable frame into a slightly
/// wrong one, which is the trade this crate makes everywhere.
fn finite_or(value: Vec3, fallback: Vec3) -> Vec3 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}
