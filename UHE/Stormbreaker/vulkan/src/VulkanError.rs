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
        /// How many objects `vkCreate*` managed to create before failing.
        /// `vkCreateGraphicsPipelines` can leave some of them alive, and
        /// silently leaking them is worse than saying so.
        partial: usize,
    },
    /// The instance enumerated no device with a graphics queue.
    NoGraphicsQueue,
    /// A required instance or device extension is not present.
    ExtensionUnavailable {
        name: String,
        required_for: &'static str,
    },
    /// The device has no usable queue family at all.
    NoQueueFamilies,
    /// No memory type satisfies the requested property flags.
    NoMemoryType {
        required: ash::vk::MemoryPropertyFlags,
    },
    /// A zero-sized resource, which Vulkan rejects.
    InvalidSize,
    /// A descriptor asked for a usage or format of nothing.
    EmptyUsage { what: &'static str },
    /// A host-visible operation was attempted on a `DEVICE_LOCAL` allocation.
    NotHostVisible { label: String },
    /// The bytes are not SPIR-V. Caught here so the driver does not report it.
    MalformedSpirv { len: usize },
    /// `forge` rejected a shader. The message is the compiler's own, because a
    /// shader error is only actionable if it points at a line of shader source.
    ShaderCompilation(String),
    /// A binary semaphore was asked for a timeline operation.
    NotTimeline,
    /// The surface is out of date: the window resized or the surface changed.
    /// Not a failure -- the frame is skipped and the swapchain recreated.
    SurfaceOutOfDate,
    /// The upload is larger than the staging ring.
    UploadTooLarge { size: u64, capacity: u64 },
    /// A chassis handle did not resolve: stale generation, or never created.
    InvalidHandle,
    /// A pass recorder returned an error.
    ///
    /// The RHI's error type crossing into the backend's, bridged in one place so
    /// the renderer above sees `chassis::Error` and the backend below sees
    /// `BackendError`, with one conversion rather than one at every call site.
    PassRecording(String),
    /// Descriptor pool creation failed or none exists.
    NoDescriptorPool,
    /// More descriptor bindings than the layout allows.
    TooManyBindings { count: usize },
    /// The device does not support something the caller asked for.
    Unsupported { what: String },
    /// More command lists were asked for than the arena holds. Failing here is
    /// deliberate: silently recording fewer would drop passes.
    ListArenaExhausted { requested: usize, available: usize },
    /// The binary semaphore chain is shorter than the list chain it is supposed
    /// to chain.
    ChainTooShort { semaphores: usize, lists: usize },
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::Loader(why) => write!(f, "vulkan loader: {why}"),
            BackendError::Vk {
                call,
                result,
                partial,
            } => {
                write!(f, "{call} failed: {result:?}")?;
                if *partial > 0 {
                    write!(f, " ({partial} objects created before failing)")?;
                }
                Ok(())
            }
            BackendError::NoGraphicsQueue => {
                write!(f, "no physical device with a graphics queue")
            }
            BackendError::ExtensionUnavailable { name, required_for } => {
                write!(f, "extension {name} is required for {required_for}")
            }
            BackendError::NoQueueFamilies => write!(f, "device reported no queue families"),
            BackendError::NoMemoryType { required } => {
                write!(f, "no memory type satisfies {required:?}")
            }
            BackendError::InvalidSize => write!(f, "resource size must be greater than zero"),
            BackendError::EmptyUsage { what } => write!(f, "{what} needs a non-empty usage"),
            BackendError::NotHostVisible { label } => {
                write!(f, "'{label}' is not host visible and cannot be mapped")
            }
            BackendError::MalformedSpirv { len } => {
                write!(f, "{len} bytes are not a SPIR-V module")
            }
            BackendError::ShaderCompilation(message) => write!(f, "{message}"),
            BackendError::NotTimeline => write!(f, "operation needs a timeline semaphore"),
            BackendError::SurfaceOutOfDate => write!(f, "surface is out of date"),
            BackendError::InvalidHandle => write!(f, "handle did not resolve"),
            BackendError::PassRecording(message) => {
                write!(f, "a pass failed to record: {message}")
            }
            BackendError::NoDescriptorPool => write!(f, "no descriptor pool"),
            BackendError::TooManyBindings { count } => {
                write!(
                    f,
                    "{count} descriptor bindings exceeds the guaranteed minimum"
                )
            }
            BackendError::Unsupported { what } => write!(f, "device does not support {what}"),
            BackendError::UploadTooLarge { size, capacity } => write!(
                f,
                "upload of {size} bytes exceeds the {capacity} byte staging ring"
            ),
            BackendError::ChainTooShort { semaphores, lists } => write!(
                f,
                "{semaphores} semaphores cannot chain {lists} command lists"
            ),
            BackendError::ListArenaExhausted {
                requested,
                available,
            } => write!(
                f,
                "asked for {requested} command lists but the arena holds {available}"
            ),
        }
    }
}

impl std::error::Error for BackendError {}

impl BackendError {
    /// Names the call that failed. Use this wherever the call is known, so the
    /// error says which request the driver rejected; the `From` impl below is
    /// the fallback for `?` at a site that does not.
    pub(crate) fn vk(call: &'static str, result: ash::vk::Result) -> Self {
        BackendError::Vk {
            call,
            result,
            partial: 0,
        }
    }

    /// Records how many objects `vkCreate*` created before it failed.
    pub(crate) fn with_partial(mut self, count: usize) -> Self {
        if let BackendError::Vk { partial, .. } = &mut self {
            *partial = count;
        }
        self
    }
}

impl From<ash::vk::Result> for BackendError {
    fn from(result: ash::vk::Result) -> Self {
        BackendError::Vk {
            call: "vulkan",
            result,
            partial: 0,
        }
    }
}

pub type Result<T> = std::result::Result<T, BackendError>;
