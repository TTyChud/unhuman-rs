//! What can go wrong in `wire`.
//!
//! One file for one enum, because the interesting part is not the enum but the
//! note on [`Error`]: **no variant holds a `rodio` or `cpal` type.** A public
//! error that leaked `cpal::StreamConfig` would force every caller to link cpal
//! to read an audio setting, which is the same mistake as naming `vk::` in
//! `bifrost`, one layer earlier. Everything here is a `String`, which is also
//! why these variants carry the underlying message: "no audio device" and "the
//! device is in use" have opposite fixes, and cpal reports both as an opaque
//! string.

/// `Result` for anything in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything that can go wrong in `wire`.
///
/// No variant holds a `rodio` or `cpal` type. A public error that leaked
/// `cpal::StreamConfig` would force every caller to link cpal to read an audio
/// setting, which is the same mistake as naming `vk::` in `bifrost`, one layer
/// earlier.
#[derive(Debug)]
pub enum Error {
    /// The platform refused to open an output device.
    ///
    /// **This is the expected error on a machine with no sound card**, which
    /// includes CI, so it is not exceptional -- it is what
    /// [`AudioEngine::headless`] is for. The message is kept because "no audio
    /// device" and "the device is in use" have opposite fixes and cpal reports
    /// both as an opaque string.
    NoDevice(String),
    /// A sound file could not be opened or decoded.
    ///
    /// The decoder's own message is kept for the same reason. A missing file
    /// and an unsupported codec are both "no sound played" from the game's point
    /// of view and are completely different bugs.
    Decode {
        /// The file that failed.
        path: std::path::PathBuf,
        /// What the OS or the decoder said.
        message: String,
    },
    /// More voices than a `u32` slot table can index.
    ///
    /// Unreachable in practice — a game with four billion simultaneous sounds
    /// has other problems — but `reserve` returns a `Result` rather than a
    /// truncating `as u32`, because a silently wrapped slot number aliases an
    /// existing voice.
    TooManyVoices(usize),
    /// A file opened and decoded to nothing.
    ///
    /// Separate from [`Error::Decode`] because it is a different bug: an empty
    /// file, or a header that decodes to zero frames. rodio accepts it, and
    /// starting a voice on it would be a voice that never makes a sound and
    /// never finishes.
    EmptySound(std::path::PathBuf),
}

impl Error {
    pub(crate) fn no_device(message: String) -> Self {
        Error::NoDevice(message)
    }

    pub(crate) fn decode(path: &std::path::Path, cause: impl std::fmt::Display) -> Self {
        Error::Decode {
            path: path.to_path_buf(),
            message: cause.to_string(),
        }
    }

    pub(crate) fn empty_sound(path: &std::path::Path) -> Self {
        Error::EmptySound(path.to_path_buf())
    }

    pub(crate) fn too_many_voices(count: usize) -> Self {
        Error::TooManyVoices(count)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoDevice(message) => write!(f, "no audio output device: {message}"),
            Error::Decode { path, message } => {
                write!(f, "could not decode sound '{}': {message}", path.display())
            }
            Error::EmptySound(path) => {
                write!(f, "sound '{}' decoded to no samples", path.display())
            }
            Error::TooManyVoices(count) => {
                write!(f, "{count} voices is more than a slot index can hold")
            }
        }
    }
}

impl std::error::Error for Error {}
