//! The clock: a fixed timestep, an accumulator, and a clamp.
//!
//! This was [`crate::solver`]'s third concern and became its own file for the
//! same reason the rest of this crate is split by concern rather than by C++
//! translation unit: "what may this body do" and "when does the world step"
//! change for different reasons and are tested for different reasons. A change
//! to the substep ceiling should not have to reason about per-axis locks.
//!
//! # What the C++ does, and why it is not a timestep
//!
//! ```text
//! m_PhysicsSystem3D.Update(ts);                      // Jolt, ts = frame delta
//! b2World_Step(m_PhysicsWorldId, ts, subStepCount); // Box2D, ts = frame delta
//! ```
//!
//! A variable timestep is not a simulation strategy, it is an assumption that
//! frames are constant, and it is wrong in both directions. On a 144 Hz
//! monitor the solver runs at 144 Hz, so a stack of boxes behaves differently
//! on two machines running the same build. On a 15 fps hitch one step moves
//! every body 4.5x as far as a normal one, which is exactly how a body ends up
//! inside a wall and gets ejected. The Jolt side additionally hardcodes
//! `collision_steps` to 1, so there is no sub-stepping underneath the variable
//! step to soften any of it.
//!
//! # The accumulator, and the failure mode that needs a clamp
//!
//! A fixed step fed by an accumulator is the only arrangement that makes a
//! simulation reproducible: the same sequence of frame deltas gives the same
//! result whether they arrived evenly at 60 fps or in clumps after a stall.
//!
//! It has one famous failure mode. Simulate more slowly than real time, spend
//! longer simulating, fall further behind, owe even more next frame: the
//! **spiral of death**. An unclamped accumulator turns one slow frame into a
//! permanently slower simulation, and the frame rate never recovers on its own
//! — the simulation is *trying* to catch up and cannot afford to.
//!
//! [`FixedStep`] clamps, and both halves of the clamp are deliberate:
//!
//! - **A hard ceiling on substeps per frame.** A frame never runs more than
//!   [`DEFAULT_MAX_SUBSTEPS`] solver steps, however much time it owes.
//! - **The unpaid debt is dropped, not carried.** When the ceiling binds, the
//!   accumulator is zeroed rather than trimmed to `fixed_dt`. Carrying it *is*
//!   the spiral — "simulate the backlog" is the decision that causes it.
//!   Dropping it costs a moment of slow motion and then the simulation is
//!   exactly caught up, which is the right trade for a real-time engine and the
//!   wrong trade for a replay, where the honest answer is to step the whole
//!   backlog. [`StepStats::dropped`] makes the loss a number, so "it stutters
//!   after a level load" is measurable rather than a mystery.
//!
//! A non-finite or negative frame delta is **rejected outright**, leaving the
//! accumulator untouched. One `NaN` would make every later `accumulator >=
//! fixed_dt` false, silently stopping the simulation for the rest of the
//! process — a failure whose only symptom is "physics stopped", with nothing
//! to log and nothing to grep.

/// 60 Hz, the rate the C++ effectively assumed and never stated.
pub const DEFAULT_FIXED_DT: f32 = 1.0 / 60.0;

/// How many solver steps one frame may run, however much time it owes.
///
/// Eight at 60 Hz is 133 ms of simulated time per frame. At a steady 60 fps the
/// accumulator asks for one step per frame and this never binds — it exists for
/// the frames where that stops being true. Eight rather than two, so that a
/// single dropped frame at 30 fps (two steps owed) is caught up normally, and
/// only a genuine stall — a load, a shader compile, a window drag — reaches the
/// ceiling and gets clamped.
pub const DEFAULT_MAX_SUBSTEPS: u32 = 8;

/// The smallest timestep [`FixedStep`] accepts, in seconds.
///
/// A zero or denormal fixed step makes `accumulator / fixed_dt` infinite. The
/// saturating float-to-int cast keeps that from panicking, but it would make
/// every frame report the ceiling and drop its whole delta — a configuration
/// mistake expressed as "physics runs at a nonsense rate". Clamping at
/// construction turns it into a merely wrong timestep, which is debuggable.
const MIN_FIXED_DT: f32 = 1.0e-6;

/// How much time one frame is owed, and what to do about the part that cannot
/// be paid.
///
/// Produced by [`FixedStep::plan`], consumed by a world's `step`. Distinct from
/// [`StepStats`], which is the *report* written after the work and is what
/// tests assert on.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct StepPlan {
    /// Solver steps to run now: `0..=max_substeps`.
    pub steps: u32,
    /// Simulated seconds those steps cover: `steps * fixed_dt`.
    pub simulated: f32,
    /// Seconds of accumulated time being discarded because the ceiling bound.
    pub dropped: f32,
    /// `true` if the ceiling bound. A condition to count or log, not to absorb
    /// silently.
    pub clamped: bool,
    /// The rejected frame delta, if it was not finite or was not positive.
    pub rejected: Option<f32>,
}

/// What one call to `step` did.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct StepStats {
    /// Solver steps run.
    pub steps: u32,
    /// Simulated seconds covered this frame.
    pub simulated: f32,
    /// Seconds discarded by the clamp this frame.
    pub dropped: f32,
    /// Whether the clamp bound this frame.
    pub clamped: bool,
}

/// A fixed timestep, fed by an accumulator, with a ceiling.
///
/// Holds no reference to a world: it decides *how many* steps are owed, and the
/// world decides how to run them. That split is what lets one clock drive both
/// the 3D and the 2D world without either re-implementing the spiral guard.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FixedStep {
    fixed_dt: f32,
    max_substeps: u32,
    accumulator: f32,
    dropped_total: f32,
    clamp_events: u64,
}

impl FixedStep {
    /// A clock stepping every `fixed_dt` seconds, at most `max_substeps` times
    /// per frame.
    ///
    /// Both arguments are clamped into a usable range rather than rejected.
    /// This is built once from engine configuration, and a configuration value
    /// of zero should not be able to produce a world where physics silently
    /// does not run.
    pub fn new(fixed_dt: f32, max_substeps: u32) -> Self {
        let fixed_dt = if fixed_dt.is_finite() && fixed_dt > MIN_FIXED_DT {
            fixed_dt
        } else {
            DEFAULT_FIXED_DT
        };
        Self {
            fixed_dt,
            max_substeps: max_substeps.max(1),
            accumulator: 0.0,
            dropped_total: 0.0,
            clamp_events: 0,
        }
    }

    /// The default clock: [`DEFAULT_FIXED_DT`], [`DEFAULT_MAX_SUBSTEPS`].
    pub fn default_rate() -> Self {
        Self::new(DEFAULT_FIXED_DT, DEFAULT_MAX_SUBSTEPS)
    }

    /// The solver timestep, in seconds.
    pub fn fixed_dt(&self) -> f32 {
        self.fixed_dt
    }

    /// The substep ceiling.
    pub fn max_substeps(&self) -> u32 {
        self.max_substeps
    }

    /// Time accumulated but not yet simulated, in seconds. Always below
    /// `fixed_dt` unless a call was rejected.
    pub fn leftover(&self) -> f32 {
        self.accumulator
    }

    /// Total seconds discarded by the clamp since this clock was created.
    pub fn dropped_total(&self) -> f32 {
        self.dropped_total
    }

    /// How many frames have reached the ceiling. A long-run health number: a
    /// non-zero value on a machine holding a steady frame rate means the frame
    /// time is worse than it looks.
    pub fn clamp_events(&self) -> u64 {
        self.clamp_events
    }

    /// Throw away unspent time and the clamp tally.
    ///
    /// For a scene reload or a resume from pause, where a long gap is not a
    /// hitch and paying it back would be wrong. Skipping a paused minute at
    /// eight substeps per frame would otherwise either spend a second of CPU
    /// catching up, or drop 59.8 seconds as a "clamp event" and make the
    /// clamp counter useless for the rest of the session.
    pub fn reset(&mut self) {
        self.accumulator = 0.0;
        self.dropped_total = 0.0;
        self.clamp_events = 0;
    }

    /// Add a frame's elapsed time and decide how many steps to run.
    ///
    /// Charges the accumulator up front: the returned count is owed
    /// unconditionally, and a caller that fails to run all of them has simply
    /// lost that time. Failing in that direction is the safe one — running
    /// fewer steps than owed makes the world slow down, running more makes it
    /// fast-forward.
    pub fn plan(&mut self, frame_dt: f32) -> StepPlan {
        if !frame_dt.is_finite() || frame_dt <= 0.0 {
            return StepPlan {
                rejected: Some(frame_dt),
                ..StepPlan::default()
            };
        }

        self.accumulator += frame_dt;

        // The cast saturates, so an absurd `frame_dt` yields `u32::MAX` rather
        // than a wrapped small number. Saturating is the right failure: the
        // `min` below clamps it and the whole absurd delta is dropped.
        let wanted = (self.accumulator / self.fixed_dt).floor() as u32;
        let clamped = wanted > self.max_substeps;
        let steps = wanted.min(self.max_substeps);
        let simulated = self.fixed_dt * steps as f32;

        // Non-negative by construction: `steps <= floor(accumulator / fixed_dt)`.
        let dropped = if clamped {
            self.accumulator - simulated
        } else {
            0.0
        };

        if clamped {
            self.accumulator = 0.0;
            self.dropped_total += dropped;
            self.clamp_events += 1;
        } else {
            self.accumulator -= simulated;
        }

        StepPlan {
            steps,
            simulated,
            dropped,
            clamped,
            rejected: None,
        }
    }
}

impl Default for FixedStep {
    fn default() -> Self {
        Self::default_rate()
    }
}
