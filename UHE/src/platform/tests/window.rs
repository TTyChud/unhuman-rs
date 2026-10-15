//! The window layer without a display: the descriptor, and the fact that every
//! window operation the engine and the backends were written against is
//! reachable on the `winit` type this crate re-exports.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use eyes::{Window, WindowDesc, WindowExt};

#[test]
fn the_default_window_is_the_one_the_c_plus_plus_created() {
    let desc = WindowDesc::default();

    assert_eq!(desc.title, "UHE ENGINE");
    assert_eq!(desc.width, 1920);
    assert_eq!(desc.height, 1080);
    assert!(!desc.vsync);
}

#[test]
fn a_descriptor_is_what_the_window_is_created_with() {
    let desc = WindowDesc {
        title: String::from("aimlab"),
        width: 800,
        height: 600,
        vsync: true,
    };

    assert_eq!(desc.title, "aimlab");
    assert_eq!((desc.width, desc.height), (800, 600));
    assert!(desc.vsync);
}

#[test]
fn the_window_operations_are_reachable_without_naming_winit() {
    // Signatures, checked at compile time and nothing else: no display is
    // opened here, so this is the one part of the contract that can be held
    // without hardware.
    let _: fn(&Window) -> (u32, u32) = <Window as WindowExt>::size;
    let _: fn(&Window) -> f32 = <Window as WindowExt>::scale_factor;
    let _: fn(&Window) = <Window as WindowExt>::request_close;
    let _: fn(&Window, bool) = <Window as WindowExt>::set_cursor_locked;
    let _: fn(&Window, &str) = Window::set_title;
    let _: fn(&Window) = Window::request_redraw;
}

#[test]
#[ignore = "needs a display: winit opens a real window and runs a real event loop"]
fn a_windowed_run_delivers_frames_until_the_closure_stops() {
    let frames = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&frames);

    let result = eyes::run_windowed(WindowDesc::default(), move |view| {
        assert!(view.window.size().0 > 0);
        assert!(<Window as WindowExt>::scale_factor(view.window) > 0.0);
        counter.fetch_add(1, Ordering::Relaxed) < 2
    });

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(frames.load(Ordering::Relaxed), 3);
}
