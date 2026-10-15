//! Buses, the master fader, and the per-frame mix.
//!
//! ## What "the mix" means here, because it is not a sample sum
//!
//! The C++ has no mixer. `MiniAudioBackend.cpp` hands every sound straight to
//! miniaudio's `ma_engine` and gets no control surface back: no master, no
//! category, no ducking (`docs/architecture/assets-and-audio.md`, "No mixing").
//!
//! A real engine cannot sum samples on the frame thread either -- that is a
//! resampler, a clock and a ring buffer, i.e. reimplementing rodio, and it would
//! put a 48 kHz stereo sum on the same thread that is trying to hold 120 fps.
//! So this "mix" is a **control-rate** mix: once per frame, for each voice,
//! decide what that voice's gain should be and tell the device. The device is
//! what mixes samples, on the audio thread, at the sample rate. That is how
//! FMOD, Wwise and every other engine in this position do it too.
//!
//! The consequence worth writing down: **the spatial maths is evaluated at
//! frame rate, not at sample rate.** A listener that turns faster than the frame
//! rate updates its pan once per frame, which is why a source sweeping past the
//! camera can be heard to step between stereo positions on a slow machine. The
//! fix is to run the same [`crate::listener::Listener`] maths inside the audio
//! thread (rodio's `source::periodic` exists for exactly this, and
//! [`crate::device::RodioDevice`] already uses it to push pans) -- but then the
//! maths is no longer testable without a device, which is the trade this crate
//! deliberately makes the other way.
//!
//! ## Two levels, and why not a tree
//!
//! One master over four child buses. Upstream's design doc asks for "master to
//! music, SFX, voice and ambient, providing group volume and ducking". That is
//! the shape, and the shape is two levels. Ducking -- music ducking under a
//! voice line -- needs a bus that *sums* to another bus and can have its own
//! gain changed while its sources are playing, which is not a third level so
//! much as a second, parallel one. It is not implemented; see the crate docs.
//!
//! Why the master is a member of [`Bus`] rather than a separate field: because
//! the obvious alternative -- `set_master_volume(v)` next to
//! `set_bus_volume(Bus::Master, v)` -- is two names for one value, and
//! `session.md` lists duplicated mappings as this codebase's recurring bug.

use crate::bus::Bus;
use crate::gain::UNITY;
use crate::instance::{InstanceState, PlayHandle, SoundInstance};
use crate::listener::Listener;
use crate::pan::{rear_attenuation, stereo_pan, StereoPan};

/// Bus volumes, and the master fader that sits on top of them.
///
/// Every volume is in `[0, 1]` and every gain is a product, so "master at
/// zero" is silence and is *the* mute. Clamping to one is the right call for a
/// bus fader -- a bus above unity is a settings-file bug and it clips -- while
/// the per-sound volume in [`crate::instance::PlayInstanceParams`] is
/// deliberately allowed above unity, because a sound louder than its bus is
/// legitimate and a bus louder than full is not.
#[derive(Clone, PartialEq, Debug)]
pub struct Mixer {
    volumes: [f32; Bus::COUNT],
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer::new()
    }
}

/// What the mix decided for one voice this frame.
///
/// Deliberately *not* the final device state. The device applies `gain` as a
/// scalar and, for a 3D sound, `pan.left` and `pan.right` as the per-channel
/// gains -- and `pan` already carries the distance attenuation and the rear
/// cue, so `gain` does not. One multiplication, applied once, in one place.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VoiceMix {
    /// Which voice this is.
    pub handle: PlayHandle,
    /// Which bus it is routed to.
    pub bus: Bus,
    /// Scalar gain: the sound's own volume times its bus times the master.
    pub gain: f32,
    /// Per-channel gains for a 3D sound, `None` for a 2D one.
    ///
    /// `None` and `Some(StereoPan::CENTRE)` are different answers on purpose. A
    /// 2D sound goes to the device as one scalar and the device does not build
    /// a stereo path for it at all; a 3D sound at the listener's own position
    /// is a stereo path with equal gains. Making both `Some` would cost every
    /// music track a downmix to mono and a per-channel gain write.
    pub pan: Option<StereoPan>,
    /// The distance attenuation alone, for diagnostics and for a debug overlay.
    ///
    /// Not multiplied into `gain` -- see the type docs.
    pub distance_gain: f32,
}

impl VoiceMix {
    /// Whether this voice should be silent whatever the device says.
    ///
    /// Not a shortcut -- it is what a culling pass and a test both need to ask,
    /// and a voice at `gain == 0` that is still being told about every frame is
    /// a voice that is costing a `set_volume` call per frame for no output.
    pub fn is_silent(&self) -> bool {
        self.gain <= 0.0
    }

    /// The loudest channel this voice would play at.
    pub fn peak(&self) -> f32 {
        self.gain * self.pan.map_or(1.0, |pan| pan.left.max(pan.right))
    }
}

impl Mixer {
    /// Every bus at full volume.
    pub fn new() -> Self {
        Mixer {
            volumes: [UNITY; Bus::COUNT],
        }
    }

    /// A bus's own fader, before the master.
    ///
    /// For [`Bus::Master`] this is the master volume itself -- there is no
    /// second field to disagree with.
    pub fn volume(&self, bus: Bus) -> f32 {
        self.volumes[bus.index()]
    }

    /// Set a bus fader, or the master fader.
    ///
    /// Clamped to `[0, 1]`; a non-finite value is treated as "set to full",
    /// because a `NaN` read out of a settings file should not silence the
    /// game.
    pub fn set_volume(&mut self, bus: Bus, volume: f32) {
        self.volumes[bus.index()] = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            UNITY
        };
    }

    /// Nudge a bus by a relative amount, clamped.
    ///
    /// This is what a settings UI slider calls. It goes through
    /// [`Self::set_volume`] rather than writing the array so the clamp cannot
    /// be bypassed.
    pub fn adjust_volume(&mut self, bus: Bus, delta: f32) {
        if delta.is_finite() {
            self.set_volume(bus, self.volume(bus) + delta);
        }
    }

    /// The effective gain for a bus: its own fader times the master.
    pub fn gain(&self, bus: Bus) -> f32 {
        self.effective_gains()[bus.index()]
    }

    /// Every bus's effective gain, in [`Bus::index`] order.
    ///
    /// This is what a debug overlay draws and what a test asserts on. It is
    /// deliberately *not* a thing the device is told about: the engine pushes
    /// the composed per-voice gain every frame from [`Self::mix_instance`], so
    /// there is exactly one place where volume meets bus meets master.
    pub fn effective_gains(&self) -> [f32; Bus::COUNT] {
        let master = self.volumes[Bus::Master.index()];
        let mut gains = [master; Bus::COUNT];
        for bus in Bus::routable() {
            gains[bus.index()] = self.volumes[bus.index()] * master;
        }
        gains
    }

    /// Every bus back to full volume.
    pub fn reset(&mut self) {
        self.volumes = [UNITY; Bus::COUNT];
    }

    /// Silence one bus without changing the others.
    pub fn mute(&mut self, bus: Bus) {
        self.set_volume(bus, 0.0);
    }

    /// Decide what one voice should play at this instant.
    ///
    /// The whole spatial chain, in the order it has to be applied:
    ///
    /// ```text
    /// sample  = listener.sample_at(sound)        listener.rs
    /// spatial = attenuation.gain_at(distance)    attenuation.rs
    ///         * rear_attenuation(frontality)     pan.rs
    /// pan     = stereo_pan(lateral) * spatial    pan.rs
    /// gain    = volume * bus_gain * master       this file
    /// ```
    ///
    /// `spatial` appears in `pan` and **not** in `gain`. Putting it in both
    /// would square it, which sounds like a falloff curve that is slightly too
    /// steep and is exactly the kind of double-mapping `session.md` warns about.
    ///
    /// A paused or stopped voice gets `gain = 0.0` and no pan: the device has
    /// already been told to pause it, and the point of zeroing the gain here is
    /// that a device which ignores `pause` still produces nothing.
    pub fn mix_instance(&self, instance: &SoundInstance, listener: &Listener) -> VoiceMix {
        let mut mix = VoiceMix {
            handle: instance.handle(),
            bus: instance.bus(),
            gain: 0.0,
            pan: None,
            distance_gain: UNITY,
        };

        if instance.state() != InstanceState::Playing {
            return mix;
        }

        let bus_gain = self.gain(instance.bus());
        let volume = instance.volume();

        match instance.position() {
            None => {
                mix.gain = volume * bus_gain;
                mix.distance_gain = UNITY;
            }
            Some(position) => {
                let sample = listener.sample_at(position);
                let distance_gain = attenuation_of(instance, sample.distance);
                let spatial = distance_gain * rear_attenuation(sample.frontality);
                mix.pan = Some(stereo_pan(sample.lateral).scaled(spatial));
                mix.gain = volume * bus_gain;
                mix.distance_gain = distance_gain;
            }
        }
        mix
    }
}

/// The instance's own attenuation, or unity if it has none.
fn attenuation_of(instance: &SoundInstance, distance: f32) -> f32 {
    match instance.attenuation() {
        Some(attenuation) => attenuation.gain_at(distance),
        None => UNITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_scales_child_gains() {
        let mut mixer = Mixer::new();
        mixer.set_volume(Bus::Master, 0.5);
        assert!((mixer.gain(Bus::Sfx) - 0.5).abs() < 1e-6);
        assert!((mixer.gain(Bus::Music) - 0.5).abs() < 1e-6);
    }
}
