//! Engine assembly: the one place both backends are visible.
//!
//! ## Why this crate exists
//!
//! `adam` (core) must not depend on a driver, so the factory cannot live there.
//! Each backend crate depends on the other only through `chassis`, so neither can
//! see the other either. This crate depends on both and does exactly two things:
//! turn a [`BackendSelection`] into a [`Gpu`], and prove that the two backends
//! are actually interchangeable.
//!
//! That second reason is the real one. "Both backends implement `chassis::Gpu`"
//! is a claim about types. Running the *same* assertions against `Box<dyn Gpu>`
//! from either backend is a claim about behaviour, and only code that can see
//! both can make it.
//!
//! ## The cost, stated plainly
//!
//! Linking this crate pulls in both drivers. A shipped build wanting one backend
//! wants a build of this crate without the other, which today means editing
//! `[dependencies]` -- so this is assembly for development and for the test
//! suite, not yet for shipping. The alternative (a `cfg` feature per backend)
//! would put a feature flag on every backend-crate dependency, and a backend
//! that compiles out cleanly is worth more once there is a shipping story to
//! break. Until then this is the honest shape: one binary, two drivers, one
//! test proving they behave the same.

use std::ffi::CStr;

use chassis::backend::{BackendChoice, BackendSelection, UnknownBackend};
use chassis::Gpu;

/// Everything the engine needs that is not the backend choice itself.
pub struct GpuRequest {
    /// The resolved backend, its source, and therefore who to blame on failure.
    pub selection: BackendSelection,
    pub engine_name: &'static CStr,
    /// Headless extent. Ignored once a swapchain is attached.
    pub extent: chassis::Extent2d,
}

/// Build the backend that was asked for.
///
/// The selection has already been through
/// [`BackendSelection::resolve`](chassis::backend::BackendSelection::resolve),
/// so the name here is known-good and any failure is the driver, not a typo --
/// except that a name can be perfectly valid and still not be compiled in, which
/// is the one error this can still produce from bad input.
pub fn create_gpu(request: GpuRequest) -> Result<Box<dyn Gpu>, CreateError> {
    match request.selection.choice {
        BackendChoice::Wgpu => stormglass::wgpu_gpu::WgpuGpu::headless(request.extent)
            .map(|gpu| Box::new(gpu) as Box<dyn Gpu>)
            .map_err(CreateError::Wgpu),
        BackendChoice::Vulkan => stormbreaker::vulkan_gpu::VulkanGpu::create_headless(
            request.engine_name,
            request.extent,
        )
        .map(|gpu| Box::new(gpu) as Box<dyn Gpu>)
        .map_err(CreateError::Vulkan),
    }
}

/// Read the settings file, if there is one.
///
/// ## Where it looks
///
/// `$UHE_SETTINGS` if set, otherwise `./unhuman.settings`.
///
/// A **missing** file is not an error and says nothing. Every game ships with
/// settings nobody wrote and runs on the defaults, and an engine that refuses to
/// start without a config file is an engine nobody runs before they have one.
///
/// A file that *cannot be read* -- permissions, a directory, invalid UTF-8 --
/// is also not fatal here, and it prints why. The alternative is a launcher that
/// says "backend: vulkan" in the log and quietly used wgpu because the file was
/// unreadable, which is precisely the bug the `BackendSource` exists to prevent.
/// So the file is unreadable but the *answer* is still knowable: it falls through
/// to the next source and the log says the file was skipped.
pub fn load_settings() -> (Option<chassis::settings::Settings>, Option<String>) {
    let path = std::env::var_os(chassis::settings::PATH_VARIABLE)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(chassis::settings::FILE_NAME));

    match std::fs::read_to_string(&path) {
        Ok(text) => (Some(chassis::settings::Settings::parse(&text)), None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(error) => (None, Some(format!("{}: {error}", path.display()))),
    }
}

/// Resolve and build in one call, from the process's own arguments, environment
/// and settings file.
///
/// The seam that `main` should call and nothing else should.
pub fn create_gpu_from_process(
    arguments: &[String],
    environment: Option<&str>,
    engine_name: &'static CStr,
    extent: chassis::Extent2d,
) -> Result<(Box<dyn Gpu>, BackendSelection), CreateError> {
    let (settings, skipped) = load_settings();
    if let Some(reason) = skipped {
        eprintln!(
            "unhuman: ignoring settings file ({reason}); falling through to the \
             next backend source"
        );
    }
    let selection = BackendSelection::resolve(
        arguments,
        environment,
        settings
            .as_ref()
            .and_then(chassis::settings::Settings::backend),
    )
    .map_err(CreateError::Name)?;
    let gpu = create_gpu(GpuRequest {
        selection: BackendSelection {
            choice: selection.choice,
            source: selection.source,
        },
        engine_name,
        extent,
    })?;
    Ok((gpu, selection))
}

#[derive(Debug)]
pub enum CreateError {
    /// The name was not one of ours. Points at the argument.
    Name(UnknownBackend),
    /// wgpu could not give us a device.
    ///
    /// `chassis::Error`, not `Unsupported`, so an adapter that was absent and an
    /// adapter that refused us stay distinguishable -- different problems, and
    /// the message should not merge them.
    Wgpu(chassis::Error),
    Vulkan(chassis::Error),
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(unknown) => write!(formatter, "{unknown}"),
            Self::Wgpu(error) => write!(
                formatter,
                "wgpu: {error} (a missing or unpermissioned DRM render node is \
                 the usual cause)"
            ),
            Self::Vulkan(error) => write!(formatter, "vulkan: {error}"),
        }
    }
}

impl std::error::Error for CreateError {}

/// Re-exported so callers do not need a second dependency to name the types they
/// are handed.
pub use stormbreaker::vulkan_gpu::VulkanGpu;
pub use stormglass::wgpu_gpu::WgpuGpu;

/// The extent a headless run uses when nothing else is specified.
pub fn default_headless_extent() -> chassis::Extent2d {
    chassis::Extent2d {
        width: 64,
        height: 64,
    }
}
