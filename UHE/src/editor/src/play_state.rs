//! Stopped / playing / paused, and the snapshot that makes stopping safe.
//!
//! The C++ toolbar swapped in a `Scene::Copy` for play and threw it away on
//! stop, so anything the running scene changed was lost *and* anything the user
//! changed while playing was lost with it. The snapshot here is the authored
//! scene: play runs on a copy, and stop puts the copy back.

use meats::Scene;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayStatus {
    Stopped,
    Playing,
    Paused,
}

pub struct PlayState {
    pub status: PlayStatus,
    /// The fixed step the simulation runs at, in seconds.
    pub fixed_dt: f32,
    /// The authored scene, held from play until stop restores it.
    snapshot: Option<Scene>,
    accumulator: f32,
    step_requested: bool,
}

impl Default for PlayState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayState {
    pub fn new() -> Self {
        Self {
            status: PlayStatus::Stopped,
            fixed_dt: 1.0 / 60.0,
            snapshot: None,
            accumulator: 0.0,
            step_requested: false,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.status == PlayStatus::Playing
    }

    pub fn is_paused(&self) -> bool {
        self.status == PlayStatus::Paused
    }

    pub fn is_stopped(&self) -> bool {
        self.status == PlayStatus::Stopped
    }

    /// Whether stop has an authored scene to go back to.
    pub fn has_snapshot(&self) -> bool {
        self.snapshot.is_some()
    }

    /// Snapshot and run. Called while paused this resumes instead, so the
    /// snapshot is still the authored scene.
    pub fn play(&mut self, scene: &mut Scene) {
        match self.status {
            PlayStatus::Paused => self.status = PlayStatus::Playing,
            PlayStatus::Playing => {}
            PlayStatus::Stopped => {
                self.snapshot = Some(scene.clone());
                self.accumulator = 0.0;
                self.step_requested = false;
                self.status = PlayStatus::Playing;
            }
        }
    }

    pub fn pause(&mut self) {
        if self.status == PlayStatus::Playing {
            self.status = PlayStatus::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.status == PlayStatus::Paused {
            self.status = PlayStatus::Playing;
        }
    }

    /// Asks for exactly one fixed step on the next `tick`.
    pub fn step_once(&mut self) {
        if self.status == PlayStatus::Paused {
            self.step_requested = true;
        }
    }

    /// Back to the authored scene. A no-op when already stopped, so a second
    /// click on stop cannot restore a snapshot twice.
    pub fn stop(&mut self, scene: &mut Scene) {
        if let Some(authored) = self.snapshot.take() {
            *scene = authored;
        }
        self.status = PlayStatus::Stopped;
        self.accumulator = 0.0;
        self.step_requested = false;
    }

    /// Seconds to simulate this frame: a whole number of fixed steps while
    /// playing, one while stepping, none while paused or stopped.
    pub fn tick(&mut self, dt: f32) -> f32 {
        match self.status {
            PlayStatus::Stopped => 0.0,
            PlayStatus::Paused => {
                if self.step_requested {
                    self.step_requested = false;
                    self.accumulator = 0.0;
                    self.fixed_dt
                } else {
                    0.0
                }
            }
            PlayStatus::Playing => {
                self.accumulator += dt.max(0.0);
                let steps = (self.accumulator / self.fixed_dt).floor();
                if steps < 1.0 {
                    return 0.0;
                }
                self.accumulator -= steps * self.fixed_dt;
                steps * self.fixed_dt
            }
        }
    }
}
