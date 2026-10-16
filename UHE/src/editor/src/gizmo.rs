//! Transform-gizmo math: axis picking and dragging in screen space against an
//! editor camera. Pure `glam`, no renderer and no toolkit — the panel measures
//! the gizmo against the same view/projection the renderer draws the scene
//! with, so what is picked is what is on screen.
//!
//! Mirror does not depend on `bifrost`, so the camera is three matrices here
//! rather than `bifrost::EditorCamera`.

use glam::{Mat4, Quat, Vec2, Vec3};

/// World length of the drawn axis arrows. Only the *screen* length matters to
/// picking, which is why this can be any unit.
const AXIS_LENGTH: f32 = 1.0;
/// How close to the projected axis, in pixels, counts as a pick.
const PICK_RADIUS: f32 = 6.0;
/// Pixels of drag that scale an axis by one.
const SCALE_PIXELS_PER_UNIT: f32 = 100.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

impl GizmoMode {
    pub fn name(self) -> &'static str {
        match self {
            GizmoMode::Translate => "Translate",
            GizmoMode::Rotate => "Rotate",
            GizmoMode::Scale => "Scale",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }
}

/// The C++ panel held Ctrl to snap: 0.5 for translate and scale, 45 degrees for
/// rotate.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Snap {
    pub enabled: bool,
    pub translate: f32,
    pub rotate_degrees: f32,
    pub scale: f32,
}

impl Default for Snap {
    fn default() -> Self {
        Self {
            enabled: false,
            translate: 0.5,
            rotate_degrees: 45.0,
            scale: 0.5,
        }
    }
}

fn snap_scalar(value: f32, step: f32) -> f32 {
    if step <= 0.0 {
        value
    } else {
        (value / step).round() * step
    }
}

pub fn snap_translate(delta: f32, step: f32) -> f32 {
    snap_scalar(delta, step)
}

pub fn snap_angle(radians: f32, step_degrees: f32) -> f32 {
    snap_scalar(radians, step_degrees.to_radians())
}

pub fn snap_scale(value: f32, step: f32) -> f32 {
    snap_scalar(value, step)
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub view: Mat4,
    pub projection: Mat4,
    pub viewport_size: Vec2,
}

impl Camera {
    /// World to window pixels, top-left origin, y down. `None` behind the eye.
    pub fn project(&self, world: Vec3) -> Option<Vec2> {
        let clip = self.projection * self.view * world.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * self.viewport_size.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * self.viewport_size.y,
        ))
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GizmoTransform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl GizmoTransform {
    pub const IDENTITY: GizmoTransform = GizmoTransform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Local axes: the C++ called `ImGuizmo::Manipulate` with `LOCAL`.
    pub fn axis(&self, axis: Axis) -> Vec3 {
        (self.rotation * axis.unit()).normalize_or_zero()
    }
}

impl Default for GizmoTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    axis: Axis,
    axis_dir: Vec3,
    start: GizmoTransform,
    origin_screen: Vec2,
    axis_screen: Vec2,
    start_mouse: Vec2,
    pixels_per_world: f32,
}

pub struct TransformGizmo {
    pub mode: GizmoMode,
    pub snap: Snap,
    drag: Option<Drag>,
}

impl Default for TransformGizmo {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformGizmo {
    pub fn new() -> Self {
        Self {
            mode: GizmoMode::Translate,
            snap: Snap::default(),
            drag: None,
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn axis(&self) -> Option<Axis> {
        self.drag.map(|drag| drag.axis)
    }

    /// Nearest axis whose projected arrow passes within `PICK_RADIUS` of the
    /// cursor. Picking follows the target's own axes, because the gizmo does
    /// (`LOCAL` in the C++): a rotated entity's X is not the world's X. An axis
    /// pointing at the camera projects to a point and cannot be picked, which
    /// is the same thing the user sees.
    pub fn pick_axis(cam: &Camera, target: &GizmoTransform, mouse: Vec2) -> Option<Axis> {
        let origin_screen = cam.project(target.translation)?;
        let mut best: Option<(f32, Axis)> = None;
        for axis in Axis::ALL {
            let tip = target.translation + target.axis(axis) * AXIS_LENGTH;
            let Some(tip_screen) = cam.project(tip) else {
                continue;
            };
            let segment = tip_screen - origin_screen;
            let length = segment.length();
            if length < 1e-3 {
                continue;
            }
            let along = ((mouse - origin_screen).dot(segment) / (length * length)).clamp(0.0, 1.0);
            let distance = (mouse - (origin_screen + segment * along)).length();
            if distance > PICK_RADIUS {
                continue;
            }
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, axis));
            }
        }
        best.map(|(_, axis)| axis)
    }

    /// Starts a drag. Fails when the axis is degenerate on screen; there is no
    /// meaningful mapping from pixels to world units then.
    pub fn begin(
        &mut self,
        cam: &Camera,
        target: &GizmoTransform,
        mouse: Vec2,
        axis: Axis,
    ) -> bool {
        let Some(origin_screen) = cam.project(target.translation) else {
            return false;
        };
        let axis_dir = target.axis(axis);
        if axis_dir == Vec3::ZERO {
            return false;
        }
        let Some(tip_screen) = cam.project(target.translation + axis_dir * AXIS_LENGTH) else {
            return false;
        };
        let segment = tip_screen - origin_screen;
        let pixels_per_world = segment.length();
        if pixels_per_world < 1e-4 {
            return false;
        }
        self.drag = Some(Drag {
            axis,
            axis_dir,
            start: *target,
            origin_screen,
            axis_screen: segment / pixels_per_world,
            start_mouse: mouse,
            pixels_per_world,
        });
        true
    }

    /// The transform the current drag produces. `None` when not dragging.
    pub fn update(&self, mouse: Vec2) -> Option<GizmoTransform> {
        let drag = self.drag?;
        let mut result = drag.start;
        let pixels = (mouse - drag.start_mouse).dot(drag.axis_screen);
        match self.mode {
            GizmoMode::Translate => {
                let mut delta = pixels / drag.pixels_per_world;
                if self.snap.enabled {
                    delta = snap_translate(delta, self.snap.translate);
                }
                result.translation = drag.start.translation + drag.axis_dir * delta;
            }
            GizmoMode::Rotate => {
                let from = drag.start_mouse - drag.origin_screen;
                let to = mouse - drag.origin_screen;
                let mut angle = signed_angle(from, to);
                if self.snap.enabled {
                    angle = snap_angle(angle, self.snap.rotate_degrees);
                }
                result.rotation = Quat::from_axis_angle(drag.axis_dir, angle) * drag.start.rotation;
            }
            GizmoMode::Scale => {
                let factor = (1.0 + pixels / SCALE_PIXELS_PER_UNIT).max(0.0);
                let mut value = drag.start.scale[drag.axis.index()] * factor;
                if self.snap.enabled {
                    value = snap_scale(value, self.snap.scale);
                }
                result.scale[drag.axis.index()] = value.max(0.0);
            }
        }
        Some(result)
    }

    pub fn end(&mut self) {
        self.drag = None;
    }
}

/// Screen angles grow clockwise, because window y points down.
fn signed_angle(from: Vec2, to: Vec2) -> f32 {
    let cross = from.x * to.y - from.y * to.x;
    cross.atan2(from.dot(to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_4;

    fn flat_camera() -> Camera {
        Camera {
            view: glam::camera::rh::view::look_at_mat4(
                Vec3::new(0.0, 0.0, 5.0),
                Vec3::ZERO,
                Vec3::Y,
            ),
            projection: glam::camera::rh::proj::directx::orthographic(
                -1.0, 1.0, -1.0, 1.0, 0.1, 100.0,
            ),
            viewport_size: Vec2::new(800.0, 600.0),
        }
    }

    #[test]
    fn projection_maps_the_origin_to_the_centre() {
        let cam = flat_camera();
        let screen = cam.project(Vec3::ZERO).unwrap();
        assert!((screen.x - 400.0).abs() < 1e-3);
        assert!((screen.y - 300.0).abs() < 1e-3);
    }

    #[test]
    fn picks_the_axis_under_the_cursor() {
        let cam = flat_camera();
        let target = GizmoTransform::IDENTITY;
        assert_eq!(
            TransformGizmo::pick_axis(&cam, &target, Vec2::new(700.0, 300.0)),
            Some(Axis::X)
        );
        assert_eq!(
            TransformGizmo::pick_axis(&cam, &target, Vec2::new(400.0, 100.0)),
            Some(Axis::Y)
        );
        assert_eq!(
            TransformGizmo::pick_axis(&cam, &target, Vec2::new(100.0, 500.0)),
            None
        );
        // Z points straight at the camera and projects to a point, so it is
        // never what the cursor is over.
        assert_ne!(
            TransformGizmo::pick_axis(&cam, &target, Vec2::new(400.0, 300.0)),
            Some(Axis::Z)
        );
    }

    #[test]
    fn picking_follows_the_local_axes() {
        let cam = flat_camera();
        let mut target = GizmoTransform::IDENTITY;
        target.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        // A quarter turn about Y puts local Z where world X was.
        assert_eq!(
            TransformGizmo::pick_axis(&cam, &target, Vec2::new(700.0, 300.0)),
            Some(Axis::Z)
        );
        assert_eq!(
            TransformGizmo::pick_axis(&cam, &target, Vec2::new(400.0, 100.0)),
            Some(Axis::Y)
        );
    }

    #[test]
    fn dragging_translate_moves_by_the_drag_distance() {
        let cam = flat_camera();
        let mut gizmo = TransformGizmo::new();
        let target = GizmoTransform::IDENTITY;
        let start_mouse = Vec2::new(400.0, 300.0);
        assert!(gizmo.begin(&cam, &target, start_mouse, Axis::X));
        assert!(gizmo.is_dragging());
        assert_eq!(gizmo.axis(), Some(Axis::X));

        // One world unit spans 400 px here, so 100 px is a quarter unit.
        let moved = gizmo.update(Vec2::new(500.0, 300.0)).unwrap();
        assert!((moved.translation.x - 0.25).abs() < 1e-4);
        assert!(moved.translation.y.abs() < 1e-6);

        // Dragging across the axis does not move along it.
        let sideways = gizmo.update(Vec2::new(500.0, 400.0)).unwrap();
        assert!((sideways.translation.x - 0.25).abs() < 1e-4);

        gizmo.end();
        assert!(!gizmo.is_dragging());
        assert!(gizmo.update(Vec2::new(500.0, 300.0)).is_none());
    }

    #[test]
    fn dragging_against_the_camera_is_refused() {
        let cam = flat_camera();
        let mut gizmo = TransformGizmo::new();
        assert!(!gizmo.begin(
            &cam,
            &GizmoTransform::IDENTITY,
            Vec2::new(400.0, 300.0),
            Axis::Z
        ));
        assert!(!gizmo.is_dragging());
    }

    #[test]
    fn snapping_quantises_the_translation() {
        let cam = flat_camera();
        let mut gizmo = TransformGizmo::new();
        let target = GizmoTransform::IDENTITY;
        gizmo.snap.enabled = false;
        assert!(gizmo.begin(&cam, &target, Vec2::new(400.0, 300.0), Axis::X));
        // 120 px is 0.3 world units, which the 0.5 grid rounds to 0.5.
        let loose = gizmo.update(Vec2::new(520.0, 300.0)).unwrap();
        assert!((loose.translation.x - 0.3).abs() < 1e-4);

        gizmo.snap.enabled = true;
        gizmo.snap.translate = 0.5;
        let snapped = gizmo.update(Vec2::new(520.0, 300.0)).unwrap();
        assert!((snapped.translation.x - 0.5).abs() < 1e-4);
        assert_eq!(snap_translate(0.3, 0.0), 0.3);
    }

    #[test]
    fn snaps_a_right_angle_at_45_degrees() {
        assert!((snap_angle(0.01, 45.0) - 0.0).abs() < 1e-6);
        assert!((snap_angle(FRAC_PI_4 * 0.9, 45.0) - FRAC_PI_4).abs() < 1e-6);
        assert!(
            (snap_angle(std::f32::consts::FRAC_PI_2 * 0.9, 45.0) - std::f32::consts::FRAC_PI_2)
                .abs()
                < 1e-6
        );
        assert_eq!(snap_angle(0.9, 0.0), 0.9);
    }

    #[test]
    fn dragging_scale_scales_one_axis() {
        let cam = flat_camera();
        let mut gizmo = TransformGizmo::new();
        gizmo.mode = GizmoMode::Scale;
        let target = GizmoTransform::IDENTITY;
        assert!(gizmo.begin(&cam, &target, Vec2::new(400.0, 300.0), Axis::Y));
        let scaled = gizmo.update(Vec2::new(400.0, 100.0)).unwrap();
        // Up the screen is the +Y axis, so dragging up grows it.
        assert!((scaled.scale.y - 3.0).abs() < 1e-4);
        assert!((scaled.scale.x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rotating_follows_the_mouse_around_the_origin() {
        let cam = flat_camera();
        let mut gizmo = TransformGizmo::new();
        gizmo.mode = GizmoMode::Rotate;
        gizmo.snap.enabled = true;
        // The Z axis points at the camera, so it cannot be grabbed on screen.
        assert!(!gizmo.begin(
            &cam,
            &GizmoTransform::IDENTITY,
            Vec2::new(400.0, 300.0),
            Axis::Z
        ));

        let axis_dir = GizmoTransform::IDENTITY.axis(Axis::X);
        assert!(gizmo.begin(
            &cam,
            &GizmoTransform::IDENTITY,
            Vec2::new(500.0, 300.0),
            Axis::X
        ));
        let rotated = gizmo.update(Vec2::new(490.0, 400.0)).unwrap();
        let angle = rotated.rotation.angle_between(Quat::IDENTITY);
        assert!((angle - FRAC_PI_4).abs() < 1e-4, "{angle}");
        assert!((rotated.rotation * axis_dir - axis_dir).length() < 1e-4);
    }

    #[test]
    fn local_axes_follow_the_rotation() {
        let mut target = GizmoTransform::IDENTITY;
        target.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!((target.axis(Axis::X) - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);
    }
}
