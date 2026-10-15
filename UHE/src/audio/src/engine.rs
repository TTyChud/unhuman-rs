//! The façade: a listener, a mixer, a voice table and a device.
//!
//! ## The façade rule
//!
//! This is the only type in `wire` holding more than one noun, and that is
//! deliberate rather than convenient — everything else is separately testable,
//! and the thing that binds them is the thing whose whole job is binding them.
//!
//! ## Why not a static entry point
//!
//! Because that is what the C++ has, and it is why the C++ cannot be tested.
//! `AudioEngine` is four `static` functions forwarding to `MiniAudioBackend`'s
//! file-scope `static ma_engine` and `static std::vector<ma_sound*>`. Nothing
//! can be constructed in isolation, nothing can be two-at-once, and every test
//! needs an audio device — so upstream's audio has no tests, and having no tests
//! is why its largest defect went unnoticed for the life of the project. This
//! takes its device as a constructor argument, so [`AudioEngine::headless`] is
//! an ordinary engine with an ordinary lifetime and CI can run it.

use crate::bus::Bus;
use crate::device::AudioDevice;
use crate::error::Result;
use crate::headless::HeadlessDevice;
use crate::listener::Listener;
use crate::mixer::Mixer;
use crate::rodio_device::RodioDevice;
use crate::voice_table::SlotTable;

/// The engine.
///
/// ## The frame loop
///
/// Three calls, once per frame, in this order:
/// [`Self::set_listener`] with the camera's transform, whatever the game wants
/// to play or change, then [`Self::update`] with the frame time.
///
/// ## Not thread-safe by contract
///
/// Every method takes `&mut self` except [`Self::device`], [`Self::listener`]
/// and [`Self::mixer`]. An engine can be moved to a dedicated audio thread —
/// [`AudioDevice`] is `Send` — but two threads cannot drive it at once without
/// the owner synchronising. That is the direct fix for the C++'s unsynchronised
/// `s_ActiveSounds`, and it is a contract rather than an enforcement: nothing
/// stops a caller wrapping an engine in a `Mutex` and doing something
/// incoherent, and pretending otherwise would be worse than saying it.
pub struct AudioEngine {
    pub(crate) listener: Listener,
    pub(crate) mixer: Mixer,
    pub(crate) device: Box<dyn AudioDevice>,
    pub(crate) voices: SlotTable,
    pub(crate) frames: u64,
}

impl AudioEngine {
    /// An engine over an already-built device.
    ///
    /// Use this to supply a device of your own, or [`Self::headless`].
    pub fn new(device: Box<dyn AudioDevice>) -> Self {
        AudioEngine {
            listener: Listener::new(),
            mixer: Mixer::new(),
            device,
            voices: SlotTable::new(),
            frames: 0,
        }
    }

    /// An engine with no audio hardware at all.
    ///
    /// **Not a mock.** The same code runs: the listener, the attenuation, the
    /// panning, the bus arithmetic and the voice lifecycle all execute, and
    /// every value they produce is readable through the [`HeadlessDevice`]. This
    /// is what makes the crate's tests run in CI, and what makes a headless
    /// game server possible.
    pub fn headless() -> Self {
        AudioEngine::new(Box::new(HeadlessDevice::new()))
    }

    /// An engine over a real rodio sink, or [`crate::Error::NoDevice`].
    ///
    /// Falling back to [`Self::headless`] on failure is reasonable for a game
    /// and terrible for a test, which is why this reports the error instead of
    /// swallowing it.
    pub fn new_on_default_device() -> Result<Self> {
        Ok(AudioEngine::new(Box::new(RodioDevice::open_default()?)))
    }

    /// The device, for inspection.
    ///
    /// Also how a caller reaches [`Self::device_spec`], [`Self::live_count`] and
    /// whatever else the device reports. Nothing should *do* anything with it:
    /// reaching through an engine to its device is how a second, divergent copy
    /// of the mix appears.
    pub fn device(&self) -> &dyn AudioDevice {
        self.device.as_ref()
    }

    /// The listener, for reading where the ears are.
    pub fn listener(&self) -> &Listener {
        &self.listener
    }

    /// The mixer, for reading bus gains and the master.
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// The mixer, for moving faders.
    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    /// How many frames [`Self::update`] has run.
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// How many sounds are playing.
    pub fn live_count(&self) -> usize {
        self.voices.live()
    }

    /// Move and turn the listener. Call once per frame with the camera.
    ///
    /// The replacement for the C++'s `SetListenerPosition(position, forward,
    /// up)` (`AudioEngine.h:22`): same three arguments, same per-frame cadence,
    /// and it now feeds arithmetic that can be asserted on with no device
    /// present.
    ///
    /// A degenerate transform produces an arbitrary frame rather than a `NaN`
    /// one; see [`Listener`] for why that is the failure mode worth designing
    /// away.
    pub fn set_listener(&mut self, position: glam::Vec3, forward: glam::Vec3, up: glam::Vec3) {
        self.listener.set_listener(position, forward, up);
    }

    /// The master fader. `0.0` is silence, and is the mute.
    pub fn set_master_volume(&mut self, volume: f32) {
        self.mixer.set_volume(Bus::Master, volume);
    }

    /// A bus's own fader, before the master.
    pub fn set_bus_volume(&mut self, bus: Bus, volume: f32) {
        self.mixer.set_volume(bus, volume);
    }
}
