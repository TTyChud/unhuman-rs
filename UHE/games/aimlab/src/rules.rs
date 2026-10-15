//! The aim lab's rules, with no scene and no input in them.
//!
//! Ported from `UHEGAME/src/AimLabLayer.cpp`: the spawn box, the five targets,
//! and the hit rule. Everything here is a pure function of its arguments, so the
//! rules can be tested without an ECS, a GPU or a window.

use glam::Vec3;

/// `m_Targets.resize(5)` in the C++ layer.
pub const TARGET_COUNT: usize = 5;

/// Seconds a target stays up before it counts as a miss.
pub const TARGET_LIFETIME: f32 = 6.0;

/// Seconds between spawns, while a target slot is free.
pub const SPAWN_INTERVAL: f32 = 1.0;

/// The pick radius. `SphereCollider3DComponent`'s default, and the size
/// `Sphere.glb` is drawn at.
pub const TARGET_RADIUS: f32 = 0.5;

/// Points a hit is worth. The C++ layer displayed hits and an accuracy
/// percentage and never totalled a score; the port scores each hit, and this is
/// that constant.
pub const HIT_POINTS: u32 = 100;

/// The C++ layer's `RandomTargetPosition`: x in `[-8, 8]`, y in `[-1, 5]`, z 10.
pub const SPAWN_X: (f32, f32) = (-8.0, 8.0);
pub const SPAWN_Y: (f32, f32) = (-1.0, 5.0);
pub const SPAWN_Z: f32 = 10.0;

/// Five units off the wall at `z = 10`, looking down -Z, which is the C++'s
/// `SetDistance(5.0f)` with the camera actually facing the targets: it set its
/// focal point to the origin while the wall sits at `+10`, so its crosshair
/// pointed away from every target it spawned.
pub const CAMERA_POSITION: Vec3 = Vec3::new(0.0, 0.0, SPAWN_Z + 5.0);

/// Ground, light and camera: everything `build` adds besides the targets.
pub const STAGE_ENTITIES: usize = 3;

/// The C++ layer's `EditorCamera(45, 16/9, 0.1, 1000)`.
pub const CAMERA_FOV: f32 = 45.0;
pub const CAMERA_NEAR: f32 = 0.1;
pub const CAMERA_FAR: f32 = 1000.0;

/// Hits, misses, shots and points, for the plugin and for a status bar.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Score {
    pub hits: u32,
    pub misses: u32,
    pub shots: u32,
    pub points: u32,
}

impl Score {
    pub fn hit(&mut self) {
        self.hits += 1;
        self.points += HIT_POINTS;
    }

    /// A target that ran out its lifetime unhit.
    pub fn miss(&mut self) {
        self.misses += 1;
    }

    pub fn shot(&mut self) {
        self.shots += 1;
    }

    /// Hits over shots, as a percentage. `0.0` before the first shot, which is
    /// what the C++ layer's dashboard printed.
    pub fn accuracy(&self) -> f32 {
        if self.shots == 0 {
            0.0
        } else {
            self.hits as f32 / self.shots as f32 * 100.0
        }
    }
}

/// A 32-bit xorshift. The C++ layer used a process-wide `std::mt19937` seeded
/// from `std::random_device`, which is exactly what a headless run cannot have;
/// a seed of its own makes a fixed step count reproduce.
pub struct Rng(u32);

impl Rng {
    pub const DEFAULT_SEED: u32 = 0x2545_F491;

    pub fn new(seed: u32) -> Self {
        Self(if seed == 0 { Self::DEFAULT_SEED } else { seed })
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// `[0.0, 1.0)`, from the top 24 bits so every value is exactly
    /// representable.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SEED)
    }
}

/// A position on the target wall. Two draws, x then y, in the C++ order.
pub fn spawn_position(rng: &mut Rng) -> Vec3 {
    let x = SPAWN_X.0 + (SPAWN_X.1 - SPAWN_X.0) * rng.next_f32();
    let y = SPAWN_Y.0 + (SPAWN_Y.1 - SPAWN_Y.0) * rng.next_f32();
    Vec3::new(x, y, SPAWN_Z)
}

/// Distance along `direction` to a sphere, or `None`.
///
/// The C++ layer picked by reading an entity-ID pixel at the screen centre while
/// the cursor was locked, which is the camera's forward ray; with no GPU in the
/// way this is that ray against the target spheres. `direction` need not be
/// normalised, and a ray starting inside the sphere returns the point it leaves
/// rather than a hit behind the eye.
pub fn ray_sphere(origin: Vec3, direction: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let to_origin = origin - center;
    let a = direction.length_squared();
    if a <= f32::EPSILON {
        return None;
    }
    let b = to_origin.dot(direction);
    let c = to_origin.length_squared() - radius * radius;
    let discriminant = b * b - a * c;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let near = (-b - root) / a;
    if near >= 0.0 {
        return Some(near);
    }
    let far = (-b + root) / a;
    if far >= 0.0 {
        Some(far)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rng_repeats_for_a_seed() {
        let mut first = Rng::new(7);
        let mut second = Rng::new(7);
        for _ in 0..8 {
            assert_eq!(first.next_u32(), second.next_u32());
        }
        assert_ne!(Rng::new(1).next_u32(), Rng::new(2).next_u32());
    }

    #[test]
    fn a_spawn_position_stays_inside_the_cpp_box() {
        let mut rng = Rng::default();
        for _ in 0..64 {
            let position = spawn_position(&mut rng);
            assert!((SPAWN_X.0..=SPAWN_X.1).contains(&position.x), "{position}");
            assert!((SPAWN_Y.0..=SPAWN_Y.1).contains(&position.y), "{position}");
            assert_eq!(position.z, SPAWN_Z);
        }
    }

    #[test]
    fn next_f32_stays_in_the_unit_interval() {
        let mut rng = Rng::default();
        for _ in 0..256 {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value), "{value}");
        }
    }
}
