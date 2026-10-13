use std::fmt;

pub use chassis::Error as RhiError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// A `chassis` call failed. The boundary owns one error type so callers
    /// treat "device lost" and "bad descriptor" uniformly: log with context,
    /// tear down the frame.
    Rhi(chassis::Error),
    /// A plugin's `build` or `on_start` hook failed.
    Plugin {
        name: &'static str,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rhi(e) => write!(f, "rhi: {e}"),
            Error::Plugin { name, message } => write!(f, "plugin {name}: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<chassis::Error> for Error {
    fn from(e: chassis::Error) -> Self {
        Error::Rhi(e)
    }
}