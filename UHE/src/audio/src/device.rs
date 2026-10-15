//! The device seam: the trait everything above this crate is written against.
//!
//! [`AudioDevice`] is the whole of it. Five files in this crate -- the
//! listener, the attenuation, the panning, the instances and the mixer --
//! contain no reference to a sound card, a file format or an audio thread, and
//! all of their tests run on a machine with no sound hardware, which is every
//! build machine and every CI runner.
//!
//! This is the same seam `eyes` puts between `Window` and `winit` and `chassis`
//! puts between `Gpu` and `ash`, and it earns its keep for the same reason: a
//! concrete device type is untestable anywhere without the hardware, and a
//! trait with a second implementation is the only thing that changes that.
//!
//! ## What is on the other side of it
//!
//! | implementation | what it is |
//! |---|---|
//! | [`crate::HeadlessDevice`] | opens nothing, plays nothing, remembers everything |
//! | [`crate::RodioDevice`] | the real thing; the only file that names `rodio` |
//!
//! Three files rather than one is the [`eyes`] layout exactly -- trait, headless
//! implementation, real implementation -- and the split is not cosmetic: the
//! headless device is what makes the rest of the crate testable, and folding it
//! into the same file as the rodio backend produces one file that is half a
//! simulation and half a driver wrapper.
//!
//! ## The Linux exception, stated here rather than hidden in a manifest
//!
//! This workspace has a Rust-first policy and rodio is the one dependency that
//! does not meet it. rodio does **not** use miniaudio and neither does kira;
//! both go through `cpal`, and on Linux `cpal` resolves to `alsa-sys`, which is
//! a **C binding to a C system library**. On Windows and macOS `cpal` is pure
//! Rust, and so is rodio. There is no pure-Rust Linux audio output path,
//! because ALSA is a C system library and the kernel interface underneath it
//! has no Rust API. This is the single documented exception, and it is written
//! down in the module that names the dependency rather than being quietly
//! grandfathered in.
//!
//! ## Why the trait is this narrow
//!
//! It has no bus table, no listener, no frame clock and no handle identity. It
//! is told "play this file into this slot at this gain", and everything else --
//! where the listener is, what the master is, whether a handle is stale -- is
//! decided above. A device that could reach for the listener would be a second
//! place for the spatial maths to happen, and a second place for it to happen is
//! a second place for it to be wrong in a way nothing tests.

use std::path::PathBuf;

use crate::pan::StereoPan;
use crate::Result;

/// What a device can tell us about itself.
///
/// Deliberately not a `rodio` type. A public API that leaked `cpal::StreamConfig`
/// would force every caller to link cpal to read an audio setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceSpec {
    /// Frames per second the device runs at.
    pub sample_rate: u32,
    /// Output channels. 2 is what every spatialiser here assumes.
    pub channel_count: u16,
}

/// What the device found out when it opened a sound.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct SoundInfo {
    /// The sound's length in seconds, if it could be determined.
    ///
    /// `None` is a real answer for a stream, and the engine turns it into a
    /// voice that only the device can retire.
    pub duration: Option<f32>,
}

/// Everything a device needs to start one voice.
///
/// A narrower thing than [`crate::instance::PlayInstanceParams`], on purpose: a
/// device must not be able to reach for the listener, the bus table or the
/// frame clock. It is told what to play and where it is, and nothing about the
/// world.
#[derive(Clone, PartialEq, Debug)]
pub struct PlayRequest {
    /// The sound file.
    pub path: PathBuf,
    /// Starting gain.
    pub volume: f32,
    /// Starting playback rate.
    pub pitch: f32,
    /// Whether to restart at the end.
    pub looping: bool,
    /// Whether to build a per-channel path for a pan.
    pub spatial: bool,
}

/// Something that plays sounds.
///
/// ## The contract
///
/// - **Every method takes `&mut self` except [`Self::spec`].** An audio device
///   is a mutable resource and pretending otherwise is how a mixer ends up
///   racing an audio thread. The trait is `Send` but not `Sync`, so the whole
///   engine can be moved to a dedicated audio thread but cannot be shared
///   across threads without the owner synchronising -- which is exactly the
///   fix for `MiniAudioBackend.cpp:18`, where `s_ActiveSounds` is a bare
///   `std::vector` mutated from both `Update` and the play functions.
/// - **Slots are the engine's.** [`Self::open`] is handed a slot number the
///   engine has already reserved, and every other method takes that same
///   number. The device does not validate it and does not know about
///   generations: there is one owner of handle identity, and it is
///   [`crate::AudioEngine`]. A device that allocated its own slots would be a
///   second place for a stale handle to hide.
/// - **Gains arrive already composed.** A device is told one scalar gain per
///   voice and, for a 3D voice, one gain per channel. It has no bus table and no
///   listener, so `volume × bus × master` happens in exactly one place.
pub trait AudioDevice: Send {
    /// What the device is.
    fn spec(&self) -> DeviceSpec;

    /// Decode and start a sound into `slot`.
    ///
    /// Reports how long it is, so the engine can retire a one-shot without
    /// having to decode it a second time to find out.
    fn open(&mut self, slot: u32, request: &PlayRequest) -> Result<SoundInfo>;

    /// Stop and release a slot.
    fn close(&mut self, slot: u32);

    /// Hold a slot where it is.
    fn pause(&mut self, slot: u32);

    /// Let a paused slot play on.
    fn resume(&mut self, slot: u32);

    /// The voice's total gain, already composed. Written every frame.
    fn set_volume(&mut self, slot: u32, volume: f32);

    /// The voice's playback rate, and therefore its pitch and its length.
    fn set_pitch(&mut self, slot: u32, pitch: f32);

    /// The voice's per-channel gains. Ignored for a 2D voice.
    fn set_pan(&mut self, slot: u32, pan: StereoPan);

    /// Retire voices that have ended for reasons the engine cannot see.
    ///
    /// `dt` is the frame time, given because a device with no clock of its own
    /// needs one to advance a voice that has a known length. A device that
    /// plays on its own thread returns an empty vector: rodio's players simply
    /// stop when their source ends, and the engine's own clock already knows
    /// about it.
    fn poll(&mut self, dt: f32) -> Vec<u32>;
}
