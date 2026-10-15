//! The UHEGAME demo, as a plugin.
//!
//! Ported from `UHEGAME/src/{GameApp.cpp,AimLabLayer.cpp,AimLabLayer.h}`. The
//! C++ pushed an `AimLabLayer` onto the application and read input from the
//! static `UHE::Input`; here the plugin owns the same state and reads the engine
//! — the cursor and the buttons come from `Engine::input`, which the window
//! layer fills in, and the entities live in `Engine::scene`.
//!
//! Kept from the C++ layer: the five targets, the spawn box, the hit rule and the
//! look sensitivity. Added by the port, and named as such: a lifetime (the C++
//! targets lived until they were hit), a spawn cadence, and a point total.

mod rules;
mod scene;

use adam::{Engine, FrameCtx, Plugin, Result};
use glam::Vec3;
use meats::{Entity, Scene};
use pings::MouseButton;

pub use rules::{
    ray_sphere, spawn_position, Rng, Score, CAMERA_FAR, CAMERA_FOV, CAMERA_NEAR, CAMERA_POSITION,
    HIT_POINTS, SPAWN_INTERVAL, SPAWN_X, SPAWN_Y, SPAWN_Z, STAGE_ENTITIES, TARGET_COUNT,
    TARGET_LIFETIME, TARGET_RADIUS,
};

/// The C++ layer's `Sphere.glb` targets and `Box.gltf` ground.
const TARGET_MESH: &str = "models/Sphere.glb";
const GROUND_MESH: &str = "models/Box.gltf";

/// The `physics_test.unhuman` floor: scale 20x1x20, top face at y = -1.5, which
/// is the bottom of the lowest target.
const GROUND_TRANSLATION: Vec3 = Vec3::new(0.0, -2.0, SPAWN_Z);
const GROUND_SCALE: Vec3 = Vec3::new(20.0, 1.0, 20.0);

/// The C++ layer's 0.0024 radians per pixel, and the 200-pixel jump it ignored
/// because a re-captured cursor warps.
const MOUSE_SENSITIVITY: f32 = 0.0024;
const MAX_MOUSE_DELTA: f32 = 200.0;
const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 * 0.995;

struct Target {
    entity: Entity,
    life: f32,
}

/// The aim lab.
pub struct AimLabPlugin {
    score: Score,
    targets: Vec<Target>,
    spawned: u32,
    spawn_timer: f32,
    rng: Rng,
    yaw: f32,
    pitch: f32,
    mouse_was_down: bool,
    built: bool,
}

impl AimLabPlugin {
    pub fn new() -> Self {
        Self {
            score: Score::default(),
            targets: Vec::new(),
            spawned: 0,
            spawn_timer: SPAWN_INTERVAL,
            rng: Rng::default(),
            yaw: 0.0,
            pitch: 0.0,
            mouse_was_down: false,
            built: false,
        }
    }

    /// Hits, misses, shots and points, for a status bar.
    pub fn score(&self) -> Score {
        self.score
    }

    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    pub fn target_entities(&self) -> Vec<Entity> {
        self.targets.iter().map(|target| target.entity).collect()
    }

    fn spawn_target(&mut self, eng: &mut Engine, life: f32) {
        let name = format!("Target_{}", self.spawned);
        self.spawned += 1;
        let position = spawn_position(&mut self.rng);
        let entity = scene::spawn(&mut eng.scene, &name, position, Vec3::ONE);
        scene::add_mesh(&mut eng.scene, entity, TARGET_MESH);
        self.targets.push(Target { entity, life });
    }

    /// The nearest target the crosshair is over. The C++ read an entity-ID pixel
    /// at the screen centre; with no GPU in the way this is that ray.
    fn pick(&self, scene: &Scene) -> Option<Entity> {
        let (origin, direction) = scene::camera_pose(scene)?;
        let mut nearest: Option<(f32, Entity)> = None;
        for target in &self.targets {
            let center = scene::translation(scene, target.entity);
            if let Some(distance) = ray_sphere(origin, direction, center, TARGET_RADIUS) {
                if nearest.map_or(true, |(best, _)| distance < best) {
                    nearest = Some((distance, target.entity));
                }
            }
        }
        nearest.map(|(_, entity)| entity)
    }

    fn expire_targets(&mut self, eng: &mut Engine, dt: f32) {
        let mut expired: Vec<Entity> = Vec::new();
        for target in &mut self.targets {
            target.life -= dt;
            if target.life <= 0.0 {
                expired.push(target.entity);
            }
        }
        for entity in expired {
            eng.scene.destroy(entity);
            self.targets.retain(|target| target.entity != entity);
            self.score.miss();
        }
    }

    /// Mouse look, the C++ layer's yaw and pitch in radians.
    fn look(&mut self, eng: &mut Engine) {
        let (dx, dy) = eng.input.cursor_delta();
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        if (dx * dx + dy * dy).sqrt() > MAX_MOUSE_DELTA {
            return;
        }
        self.yaw -= dx * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch + dy * MOUSE_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        scene::aim(&mut eng.scene, self.yaw.to_degrees(), self.pitch.to_degrees());
    }

    /// One shot per press, the C++ layer's `mouseDown && !m_MouseWasPressed`.
    /// Held rather than pressed, because a frame can run several substeps and
    /// "pressed" stays true for all of them.
    fn shoot(&mut self, eng: &mut Engine) {
        let down = eng.input.is_mouse_held(MouseButton::Left);
        let fired = down && !self.mouse_was_down;
        self.mouse_was_down = down;
        if !fired {
            return;
        }

        self.score.shot();
        let Some(entity) = self.pick(&eng.scene) else {
            return;
        };

        self.score.hit();
        let position = spawn_position(&mut self.rng);
        scene::set_translation(&mut eng.scene, entity, position);
        if let Some(target) = self.targets.iter_mut().find(|t| t.entity == entity) {
            target.life = TARGET_LIFETIME;
        }
    }
}

impl Default for AimLabPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for AimLabPlugin {
    fn name(&self) -> &'static str {
        "aimlab"
    }

    fn build(&mut self, eng: &mut Engine) -> Result<()> {
        let ground = scene::spawn(&mut eng.scene, "Ground", GROUND_TRANSLATION, GROUND_SCALE);
        scene::add_mesh(&mut eng.scene, ground, GROUND_MESH);

        let light = scene::spawn(&mut eng.scene, "DirectionalLight", Vec3::ZERO, Vec3::ONE);
        scene::add_directional_light(&mut eng.scene, light);

        let camera = scene::spawn(&mut eng.scene, "Camera", CAMERA_POSITION, Vec3::ONE);
        scene::add_primary_camera(&mut eng.scene, camera, CAMERA_FOV, CAMERA_NEAR, CAMERA_FAR);

        // Staggered by the cadence so the five do not expire in one tick.
        for index in 0..TARGET_COUNT {
            self.spawn_target(eng, TARGET_LIFETIME + index as f32 * SPAWN_INTERVAL);
        }
        self.spawn_timer = SPAWN_INTERVAL;
        self.built = true;

        log::info!(
            "aimlab: {} entities, {} targets",
            eng.scene.entities().len(),
            self.targets.len()
        );
        Ok(())
    }

    fn on_update(&mut self, eng: &mut Engine, ctx: &mut FrameCtx) {
        if !self.built {
            return;
        }
        let dt = ctx.step_seconds();

        self.spawn_timer -= dt;
        if self.spawn_timer <= 0.0 {
            self.spawn_timer += SPAWN_INTERVAL;
            if self.targets.len() < TARGET_COUNT {
                self.spawn_target(eng, TARGET_LIFETIME);
            }
        }

        self.expire_targets(eng, dt);
        self.look(eng);
        self.shoot(eng);
    }
}
