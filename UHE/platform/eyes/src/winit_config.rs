//! Turning a [`WindowConfig`] into winit's window attributes.
//!
//! Its own file because construction is three separate platform questions --
//! what a size means, how fullscreen works, and which thread may read the
//! handle -- and none of them has anything to do with running a frame.

use winit::dpi::{LogicalSize, PhysicalSize};
use winit::window::WindowAttributes;

use crate::config::{WindowConfig, WindowMode, WindowSize};

/// Build the attributes a window opens with.
pub(super) fn build_attributes(config: &WindowConfig) -> WindowAttributes {
    let (width, height) = config.size.logical();

    let attributes = WindowAttributes::default().with_title(&config.title);

    // Physical and logical are different requests and the difference is all of
    // display scaling. `Physical` promises a framebuffer of exactly this size;
    // `Logical` asks for this many logical units and lets the OS apply the scale
    // factor on top. Getting it backwards is invisible at 100% and doubles
    // every window on a HiDPI display.
    let attributes = match config.size {
        WindowSize::Default | WindowSize::Logical { .. } => {
            attributes.with_inner_size(LogicalSize::new(f64::from(width), f64::from(height)))
        }
        WindowSize::Physical { width, height } => {
            attributes.with_inner_size(PhysicalSize::new(width, height))
        }
    };

    if config.transparent {
        attributes.with_transparent(true)
    } else {
        attributes
    }
}

/// Apply the window mode.
///
/// Separate from [`build_attributes`] because a monitor is only reachable from
/// an existing window. Unlike the size, the mode cannot be settled before
/// creation -- which is a platform constraint, not a choice.
pub(super) fn apply_mode(window: &winit::window::Window, mode: WindowMode) {
    use winit::window::Fullscreen;

    // Built and handed over in one expression rather than bound to a local:
    // `Fullscreen` carries a platform handle and is not `Copy`, so a local would
    // have to be cloned back out.
    //
    // Infallible as far as the caller is concerned -- winit leaves the window
    // alone when the platform declines. A `None` from the exclusive arm means
    // "no video mode to ask for", which is also a decline, and staying windowed
    // beats a panic during startup.
    match mode {
        // `None` is "not fullscreen", not "no monitor". A different absence.
        WindowMode::Windowed => window.set_fullscreen(None),
        // Borderless with no monitor means "the one this window is on", which
        // is the right default and needs no lookup.
        WindowMode::Borderless => window.set_fullscreen(Some(Fullscreen::Borderless(None))),
        // Exclusive needs a specific video mode, so it needs a real lookup --
        // and it takes the first mode the monitor reports, because asking a
        // user to choose a resolution is a different feature.
        WindowMode::Fullscreen => window.set_fullscreen(
            window
                .primary_monitor()
                .and_then(|monitor| monitor.video_modes().next())
                .map(Fullscreen::Exclusive),
        ),
    }
}

/// Let the raw window handle be read from a thread other than the one that
/// created the window.
///
/// winit 0.30 takes this at event-loop construction and has no per-call
/// variant, so it cannot be set where [`crate::Window::raw_handle`] is
/// implemented.
///
/// It is not optional. Both backends build their surface from this handle, and
/// both do so from the frame callback, which is not the constructing thread.
/// Without the flag winit *panics* inside `create_surface`, naming neither
/// winit's rule nor this crate -- and only on Windows, and only when the two
/// threads differ. So a Linux test run passes every time and the shipping
/// Windows build does not.
///
/// A no-op on macOS, which has no such restriction. That asymmetry is the whole
/// reason it is set unconditionally rather than per-platform.
pub(super) fn allow_raw_handle_off_thread(builder: &mut winit::event_loop::EventLoopBuilder<()>) {
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        EventLoopBuilderExtWindows::with_any_thread(builder, true);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        use winit::platform::x11::EventLoopBuilderExtX11;
        // Fully qualified, because X11 and Wayland both expose a
        // `with_any_thread` on `EventLoopBuilder` and importing both traits
        // makes the bare call ambiguous. A session can be on either, so both
        // are set rather than guessing from the display server at runtime --
        // and a guess would have to be made before the window exists.
        EventLoopBuilderExtX11::with_any_thread(builder, true);
        EventLoopBuilderExtWayland::with_any_thread(builder, true);
    }
    #[cfg(target_os = "macos")]
    {
        // macOS puts no restriction on which thread may read the handle. The
        // parameter exists so the signature is the same on every platform.
        let _ = builder;
    }
}
