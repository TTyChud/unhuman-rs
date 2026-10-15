//! The center viewport.
//!
//! ## What it can draw today, and why that is not the engine's image
//!
//! The engine's frame is not reachable from here. The editor window belongs to
//! gpui, so the engine is built headless (`engine::create_gpu_from_process` hands
//! back `WgpuGpu::headless`, and `chassis::Gpu::swapchain()` is `None` by
//! design). Phase one of `bifrost` then declares, compiles and validates its
//! render graph but does not submit it, because `chassis::Gpu` has no entry point
//! that takes a compiled frame and a recorder. And there is no readback either:
//! `chassis::Resources` creates and destroys resources, it does not hand pixels
//! back to the CPU.
//!
//! So the viewport draws the theme background and says what is missing. It does
//! **not** draw an image, because there is no image: a stored RGBA buffer here
//! would be a claim about pixels the renderer never produced.
//!
//! When a producer exists, this file gains the frame it hands over — extent plus
//! RGBA8 — and `panels::viewport` draws it instead of the overlay. Nothing else
//! changes; the panel is already the only caller.

/// Shown in the middle of the viewport, in place of an image.
pub const MISSING_FRAME_REASON: &str =
    "no engine image: bifrost compiles its frame but has no executor, and chassis has no readback";

/// The renderer's name, or why there is none.
pub fn backend_label(backend: Option<&str>) -> &str {
    backend.unwrap_or("no backend")
}

/// `60.0 fps`, for the toolbar and the status bar.
pub fn fps_label(fps: f32) -> String {
    format!("{fps:.1} fps")
}

/// `16.61 ms`, the frame the stats were sampled over.
pub fn frame_time_label(frame_ms: f32) -> String {
    format!("{frame_ms:.2} ms")
}

/// `wgpu · 60.0 fps · 16.61 ms` — one line, in the status bar.
pub fn frame_status(backend: Option<&str>, fps: f32, frame_ms: f32) -> String {
    format!(
        "{} · {} · {}",
        backend_label(backend),
        fps_label(fps),
        frame_time_label(frame_ms)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_keeps_one_decimal() {
        assert_eq!(fps_label(60.0), "60.0 fps");
        assert_eq!(fps_label(59.949), "59.9 fps");
        assert_eq!(fps_label(0.0), "0.0 fps");
    }

    #[test]
    fn frame_time_keeps_two_decimals() {
        assert_eq!(frame_time_label(16.606), "16.61 ms");
        assert_eq!(frame_time_label(0.0), "0.00 ms");
    }

    #[test]
    fn status_names_the_backend_or_says_there_is_none() {
        assert_eq!(
            frame_status(Some("wgpu"), 60.0, 16.61),
            "wgpu · 60.0 fps · 16.61 ms"
        );
        assert_eq!(backend_label(None), "no backend");
        assert!(frame_status(None, 0.0, 0.0).starts_with("no backend"));
    }

    #[test]
    fn the_missing_frame_reason_names_a_cause() {
        assert!(MISSING_FRAME_REASON.contains("no engine image"));
    }
}
