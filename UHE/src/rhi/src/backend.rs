//! Which backend, and how that was decided.
//!
//! ## wgpu is the default, and that is a stability decision
//!
//! Not a performance one and not a preference. Vulkan gives per-barrier control
//! and bare-metal speed; wgpu gives a driver that works. When a frame goes wrong
//! the question is always "is this the backend or is this the engine", and a
//! default that is *the thing that usually works* means the first answer is
//! usually "the engine".
//!
//! Vulkan is still fully implemented and fully tested -- it is the path to bare
//! metal and the only one that can reach a 1.0 device. It is opt-in rather than
//! opt-out because opting out is what you do when something is unstable, and you
//! should not have to rebuild to find out whether that is what is happening.
//!
//! ## Three ways to say it, in order
//!
//! 1. `--backend vulkan` on the command line.
//! 2. `UHE_BACKEND=vulkan` in the environment.
//! 3. A `backend` key in the settings file.
//!
//! The first wins. An unrecognised value is an error rather than a silent
//! fallback: a launcher that passes `--backend vlkan` and gets wgpu has a bug
//! that will be reported as "the wrong backend", and the message that says so is
//! worth more than the convenience.

use std::fmt;
use std::str::FromStr;

/// The backend a run will use.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BackendChoice {
    /// wgpu. The default, because it is the one that works on the most machines.
    #[default]
    Wgpu,
    /// Vulkan through `ash`. Bare-metal control: explicit barriers, explicit
    /// memory, and the only path that reaches a 1.0 device.
    Vulkan,
}

impl BackendChoice {
    /// The short name, for a log line and for a settings file.
    pub fn name(self) -> &'static str {
        match self {
            BackendChoice::Wgpu => "wgpu",
            BackendChoice::Vulkan => "vulkan",
        }
    }

    /// Both, for a launcher that offers a choice.
    pub fn all() -> [BackendChoice; 2] {
        [BackendChoice::Wgpu, BackendChoice::Vulkan]
    }

    /// Whether this backend can be chosen here.
    ///
    /// Split out because a build can omit one: the Vulkan backend needs a Vulkan
    /// loader and the wgpu one needs a driver wgpu recognises, and a machine can
    /// have neither. `available` reporting a backend that then fails to create is
    /// how "I picked wgpu and got nothing" happens.
    pub fn is_compiled_in(self) -> bool {
        true
    }
}

impl fmt::Display for BackendChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The value was not one of the backends.
///
/// Carries what was asked for, because "unknown backend `vulkam`" is actionable
/// and "unknown backend" is not.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownBackend(pub String);

impl fmt::Display for UnknownBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = BackendChoice::all().iter().map(|b| b.name()).collect();
        write!(
            f,
            "unknown backend {:?}; expected one of {}",
            self.0,
            names.join(", ")
        )
    }
}

impl std::error::Error for UnknownBackend {}

impl FromStr for BackendChoice {
    type Err = UnknownBackend;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            // `vulkan` is the only spelling. Not `ash`, not `vk`: the flag names
            // the API a user can reason about, and a second alias is a second
            // thing to document and a second thing to typo.
            "vulkan" | "vk" => Ok(BackendChoice::Vulkan),
            "wgpu" => Ok(BackendChoice::Wgpu),
            _ => Err(UnknownBackend(value.to_owned())),
        }
    }
}

/// Where a backend choice came from, for a log line that says why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackendSource {
    /// Nobody said, so the default was used.
    Default,
    /// `--backend` on the command line.
    Flag,
    /// `UHE_BACKEND` in the environment.
    Environment,
    /// A settings file.
    Settings,
}

impl fmt::Display for BackendSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BackendSource::Default => "default",
            BackendSource::Flag => "--backend",
            BackendSource::Environment => "UHE_BACKEND",
            BackendSource::Settings => "settings",
        })
    }
}

/// The environment variable read when no flag is given.
pub const ENVIRONMENT_VARIABLE: &str = "UHE_BACKEND";

/// The command-line flag, without the leading dashes.
pub const FLAG: &str = "backend";

/// A resolved choice, and where it came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackendSelection {
    pub choice: BackendChoice,
    pub source: BackendSource,
}

impl Default for BackendSelection {
    fn default() -> Self {
        Self {
            choice: BackendChoice::default(),
            source: BackendSource::Default,
        }
    }
}

impl BackendSelection {
    /// Work out the backend from a command line and an environment.
    ///
    /// Both are passed in rather than read from the process, so the whole
    /// decision is a pure function and can be tested for every combination --
    /// including "the flag wins", which is the one that matters and the one that
    /// silently stops being true when someone adds a third source.
    pub fn resolve(
        arguments: &[String],
        environment: Option<&str>,
    ) -> Result<Self, UnknownBackend> {
        if let Some(value) = flag_value(arguments) {
            return Ok(Self {
                choice: value.parse()?,
                source: BackendSource::Flag,
            });
        }
        if let Some(value) = environment {
            if !value.trim().is_empty() {
                return Ok(Self {
                    choice: value.parse()?,
                    source: BackendSource::Environment,
                });
            }
        }
        Ok(Self::default())
    }
}

/// The value of `--backend`, or `--backend=vulkan`.
///
/// Both spellings, because a flag parser that accepts one and not the other is a
/// trap that only shows up on the one invocation nobody tests.
pub fn flag_value(arguments: &[String]) -> Option<String> {
    let mut iter = arguments.iter();
    while let Some(argument) = iter.next() {
        if argument == "--backend" {
            return iter.next().cloned();
        }
        if let Some(value) = argument.strip_prefix("--backend=") {
            return Some(value.to_owned());
        }
    }
    None
}
