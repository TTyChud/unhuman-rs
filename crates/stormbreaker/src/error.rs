use std::fmt;

/// Errors raised by the backend itself.
///
/// Separate from `chassis::Error` on purpose: `chassis` describes what a
/// frontend can recover from, and it should not have to know that Vulkan has a
/// `VkResult`. This is the layer where the two meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The Vulkan loader could not be found or loaded.
    Loader(String),
    /// A Vulkan call failed.
    /// A Vulkan call failed. The call is named because `ERROR_INCOMPATIBLE_DRIVER`
    /// from `vkCreateDevice` is almost always a feature or extension request the
    /// device did not expect, and the request is what you need to see.
    Vk {
        call: &'static str,
        result: ash::vk::Result,
    },
    /// The instance enumerated no device with a graphics queue.
    NoGraphicsQueue,
    /// A required instance or device extension is not present.
    ExtensionUnavailable { name: String, required_for: &'static str },
    /// The device has no usable queue family at all.
    NoQueueFamilies,
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::Loader(why) => write!(f, "vulkan loader: {why}"),
            BackendError::Vk { call, result } => {
                write!(f, "{call} failed: {result:?}")
            }
            BackendError::NoGraphicsQueue => {
                write!(f, "no physical device with a graphics queue")
            }
            BackendError::ExtensionUnavailable { name, required_for } => {
                write!(f, "extension {name} is required for {required_for}")
            }
            BackendError::NoQueueFamilies => write!(f, "device reported no queue families"),
        }
    }
}

impl std::error::Error for BackendError {}

impl BackendError {
    pub(crate) fn vk(call: &'static str, result: ash::vk::Result) -> Self {
        BackendError::Vk { call, result }
    }
}

pub type Result<T> = std::result::Result<T, BackendError>;