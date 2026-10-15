//! Window configuration.
//!
//! Data only. No windowing types, so a config is comparable, cloneable and
//! testable with no display, and so a settings file can hold one.

/// How the window sits on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WindowMode {
    /// A normal OS window with a title bar.
    #[default]
    Windowed,
    /// Fills the display, with decoration gone but input still arriving.
    ///
    /// Borderless rather than exclusive, because exclusive needs a video mode
    /// and takes the display away from the compositor -- two window managers
    /// and a screensaver stop working, for no gain anyone asked for.
    Borderless,
    /// Takes the display. Only when something needs it, and only on the
    /// platforms that still support it.
    Fullscreen,
}

/// Size, plus which of its fields mean what.
///
/// Logical and physical are different requests and the difference is all of
/// display scaling. Getting it backwards is invisible on a 100% display and
/// makes every window twice the intended size on a HiDPI one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowSize {
    /// The platform decides.
    Default,
    /// In logical units. The scale factor is applied on top, so this is what a
    /// user-facing setting should be.
    Logical {
        /// Logical units across.
        width: u32,
        /// Logical units down.
        height: u32,
    },
    /// In physical pixels. Nothing is applied on top, which makes this the only
    /// size that can promise an exact framebuffer.
    Physical {
        /// Physical pixels across.
        width: u32,
        /// Physical pixels down.
        height: u32,
    },
}

impl WindowSize {
    /// The logical size this implies, ignoring the display's scale factor.
    ///
    /// The two size variants differ only in what the platform does with the
    /// number, so the number itself is the same.
    pub fn logical(self) -> (u32, u32) {
        match self {
            WindowSize::Default => (1280, 720),
            WindowSize::Logical { width, height } | WindowSize::Physical { width, height } => {
                (width, height)
            }
        }
    }
}

/// Everything needed to open a window.
#[derive(Clone, PartialEq, Debug)]
pub struct WindowConfig {
    /// Shown in the title bar and the task switcher.
    pub title: String,
    /// How big it opens. Not a promise: the platform may disagree, and the user
    /// may resize it. [`crate::Window::extent`] is the truth.
    pub size: WindowSize,
    /// Fullscreen or not.
    pub mode: WindowMode,
    /// Start with vsync requested. See [`crate::Window::set_vsync`] for why
    /// this is a field rather than a call that might do nothing.
    pub vsync: bool,
    /// Transparent background, where the platform supports it.
    pub transparent: bool,
    /// Reserved. A frame loop should always draw into a buffer and present at
    /// the end of the frame; the alternative is a frame that is whatever the
    /// last present happened to be, which cannot be tested and cannot be
    /// screenshotted.
    pub wait_for_present: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "unhuman".to_string(),
            size: WindowSize::Default,
            mode: WindowMode::Windowed,
            vsync: true,
            transparent: false,
            wait_for_present: true,
        }
    }
}

impl WindowConfig {
    /// A config with a title and everything else default.
    pub fn titled(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// Set the opening size.
    pub fn with_size(mut self, size: WindowSize) -> Self {
        self.size = size;
        self
    }

    /// Set the opening mode.
    pub fn with_mode(mut self, mode: WindowMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set whether vsync is requested.
    pub fn with_vsync(mut self, vsync: bool) -> Self {
        self.vsync = vsync;
        self
    }

    /// Set whether the background is transparent.
    pub fn with_transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }
}
