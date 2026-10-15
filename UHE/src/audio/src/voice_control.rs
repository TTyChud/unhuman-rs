use crate::device::PlayRequest;
use crate::error::Result;
use crate::gain::sanitise_delta;
use crate::instance::{PlayHandle, SoundInstance};
use crate::play_params::PlayInstanceParams;

use super::AudioEngine;

impl AudioEngine {
    /// Start a sound with no position. Fire-and-forget unless you keep the
    /// handle.
    ///
    /// The replacement for `AudioEngine::PlaySound2D(filepath, volume, loop)`,
    /// with the same defaults. Unlike the C++, it returns something: a
    /// [`PlayHandle`] that can stop it, pause it, or find out it has already
    /// finished.
    pub fn play_2d(
        &mut self,
        path: impl AsRef<std::path::Path>,
        params: PlayInstanceParams,
    ) -> Result<PlayHandle> {
        self.play(path.as_ref(), params)
    }

    /// Start a sound at a world position, relative to the listener.
    ///
    /// The replacement for `AudioEngine::PlaySound3D(filepath, position,
    /// volume, loop)` — the function `AimLabLayer.cpp:215` calls for a reload
    /// and `:255` for a gunshot — except that the position now means something
    /// testable.
    pub fn play_3d(
        &mut self,
        path: impl AsRef<std::path::Path>,
        position: glam::Vec3,
        params: PlayInstanceParams,
    ) -> Result<PlayHandle> {
        self.play(path.as_ref(), params.at_position(position))
    }

    /// Start a sound. What [`Self::play_2d`] and [`Self::play_3d`] are.
    pub fn play(
        &mut self,
        path: &std::path::Path,
        params: PlayInstanceParams,
    ) -> Result<PlayHandle> {
        let request = PlayRequest {
            path: path.to_path_buf(),
            volume: params.volume,
            pitch: params.pitch,
            looping: params.looping,
            // A 3D sound is one with a position, and that is the whole test. The
            // device builds a per-channel path only for those.
            spatial: params.position.is_some(),
        };

        // The slot is reserved *before* the open, because the slot number is the
        // device's argument. A failed open has to give it back.
        let index = self.voices.reserve()?;
        let info = match self.device.open(index, &request) {
            Ok(info) => info,
            Err(error) => {
                self.voices.abandon(index);
                return Err(error);
            }
        };

        // The instance carries its own handle, so it is built before it is
        // stored; `handle_at` is the one ordering wrinkle in the whole crate.
        let handle = self.voices.handle_at(index);
        let instance = SoundInstance::new(handle, params, info.duration);
        Ok(self.voices.insert(index, instance))
    }

    /// The sound a handle names, if it is still that sound.
    pub fn instance(&self, handle: PlayHandle) -> Option<&SoundInstance> {
        self.voices.get(handle)
    }

    /// The sound a handle names, if it is still that sound.
    pub fn instance_mut(&mut self, handle: PlayHandle) -> Option<&mut SoundInstance> {
        self.voices.get_mut(handle)
    }

    /// Stop a sound. Returns whether it was still playing.
    ///
    /// `false` means the handle is stale — the slot has been given to a different
    /// sound — and is the whole reason [`PlayHandle`] carries a generation.
    pub fn stop(&mut self, handle: PlayHandle) -> bool {
        if self.voices.get(handle).is_none() {
            return false;
        }
        self.close_slot(handle.index());
        true
    }

    /// Stop everything. The shutdown path.
    pub fn stop_all(&mut self) {
        for index in 0..self.voices.len() as u32 {
            self.close_slot(index);
        }
    }

    /// Pause a sound where it is. Not in the C++ at all.
    pub fn pause(&mut self, handle: PlayHandle) -> bool {
        self.set_paused(handle, true)
    }

    /// Let a paused sound play on from where it stopped. Not a restart.
    pub fn resume(&mut self, handle: PlayHandle) -> bool {
        self.set_paused(handle, false)
    }

    /// Change one sound's volume. The missing `SetVolume`.
    ///
    /// A pre-gain, allowed above unity, and **not** the bus fader — see
    /// [`Mixer`] for why those are different controls.
    pub fn set_volume(&mut self, handle: PlayHandle, volume: f32) -> bool {
        self.edit(handle, |instance| instance.set_volume(volume))
    }

    /// Change one sound's pitch, and therefore how long it takes to finish.
    pub fn set_pitch(&mut self, handle: PlayHandle, pitch: f32) -> bool {
        self.edit(handle, |instance| instance.set_pitch(pitch))
    }

    /// Move a sound, or make a 2D sound 3D by passing a position.
    pub fn set_position(&mut self, handle: PlayHandle, position: Option<glam::Vec3>) -> bool {
        self.edit(handle, |instance| instance.set_position(position))
    }

    /// One frame. Run this once per frame, after the game's calls.
    ///
    /// `dt` is the frame time in seconds. Three things happen, in this order,
    /// and the order matters:
    ///
    /// 1. **Mix.** Every live voice gets its gain for this instant: its volume
    ///    times its bus times the master, and for a 3D voice the attenuation and
    ///    pan are folded into its per-channel gains. This is the only place that
    ///    product is taken — see [`Mixer::mix_instance`].
    /// 2. **Advance.** Every voice's clock moves, and a one-shot past its length
    ///    is retired. The engine's clock is the authority; see
    ///    [`SoundInstance::advance`].
    /// 3. **Reap.** Whatever finished — whether the engine's clock said so or
    ///    the device did — is closed and its slot freed.
    ///
    /// A voice that finishes mid-frame is closed at the *end* of the frame, not
    /// partway through it. Retiring one while walking the table would invalidate
    /// the walk.
    ///
    /// A non-finite `dt` is treated as zero. It is the classic arrival route for
    /// a `NaN` — a first frame whose timestamp is its own denominator — and it
    /// must not become a `NaN` elapsed time that makes every voice look finished
    /// at once.
    pub fn update(&mut self, dt: f32) {
        self.frames += 1;
        let dt = sanitise_delta(dt);

        let mut retired: Vec<u32> = Vec::new();
        for (index, instance) in self.voices.instances_mut() {
            let mix = self.mixer.mix_instance(instance, &self.listener);
            self.device.set_volume(index, mix.gain);
            if let Some(pan) = mix.pan {
                self.device.set_pan(index, pan);
            }
            if instance.advance(dt) {
                retired.push(index);
            }
        }

        // A device may retire a voice the engine's clock cannot know about: a
        // stream that ended, a sink that was stolen. Deduped because both paths
        // can name the same voice in the same frame.
        retired.extend(self.device.poll(dt));
        retired.sort_unstable();
        retired.dedup();
        for index in retired {
            self.close_slot(index);
        }
    }

    /// Change one voice, and report whether the handle was live.
    ///
    /// Every setter above is this. Four copies of the same match-and-refuse
    /// would be four chances to get the stale-handle rule subtly different.
    fn edit(&mut self, handle: PlayHandle, f: impl FnOnce(&mut SoundInstance)) -> bool {
        match self.voices.get_mut(handle) {
            Some(instance) => {
                f(instance);
                true
            }
            None => false,
        }
    }

    /// Pause or resume, on both the voice and the device.
    ///
    /// Both sides, in that order: the engine's state is the authority and the
    /// device is told what it is. A device that ignored the pause would then
    /// still be silenced, because [`Mixer::mix_instance`] gives a paused voice a
    /// gain of zero.
    fn set_paused(&mut self, handle: PlayHandle, paused: bool) -> bool {
        let Some(instance) = self.voices.get_mut(handle) else {
            return false;
        };
        if paused {
            instance.pause();
            self.device.pause(handle.index());
        } else {
            instance.resume();
            self.device.resume(handle.index());
        }
        true
    }

    /// Release a slot and tell the device.
    fn close_slot(&mut self, index: u32) {
        self.voices.close(index);
        self.device.close(index);
    }
}

#[cfg(test)]
mod tests {
    use crate::{AudioEngine, PlayInstanceParams};

    #[test]
    fn instance_lifecycle_no_device() {
        let mut audio = AudioEngine::headless();
        let handle = audio
            .play_2d("x.wav", PlayInstanceParams::default())
            .unwrap();
        assert_eq!(audio.live_count(), 1);
        assert!(audio.instance(handle).is_some());
        audio.update(0.5);
        assert!(audio.instance(handle).is_some());
        audio.update(0.6);
        assert!(audio.instance(handle).is_none());
        assert_eq!(audio.live_count(), 0);
        assert!(!audio.stop(handle));
    }
}
