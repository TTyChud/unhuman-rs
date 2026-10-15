//! Camera, frustum, and the one cull the frame does on the CPU.
//!
//! ## What is here and what is Phase 5/6
//!
//! Projection, view, frustum extraction and sphere-versus-planes culling. That is
//! enough to stop submitting meshes that cannot be on screen, which is the cull
//! that pays for itself in the first frame.
//!
//! *Not* here: occlusion culling and light clustering. Both need a depth buffer
//! this crate does not yet produce, and both are listed in `session.md` Phase 5
//! and Phase 6 rather than faked. A frustum that culls nothing is still correct;
//! a depth pyramid that culls the wrong thing is a hole in the image.
//!
//! Shadow fitting lives in [`crate::shadow`]: it is a projection, not a cull, and
//! folding it in here would put two concerns in one file.
//!
//! ## Which projection: `directx`, not `vulkan`, and this one is load-bearing
//!
//! `glam` 0.34 sorts its constructors by *API convention*, not by maths:
//! `camera::rh::proj::opengl` is `[-1, 1]` Y-up, `directx` is `[0, 1]` Y-up, and
//! `vulkan` is `[0, 1]` **Y-down**. All three are right-handed; the only
//! difference is the depth range and, for `vulkan`, a Y flip.
//!
//! This file uses `directx`. Both halves of that are deliberate:
//!
//! - **Depth range `[0, 1]`** is required by Vulkan and by wgpu alike. Using
//!   `opengl`'s does not fail loudly: the driver clamps, everything past the near
//!   plane collapses onto `z = 0`, and the result is indistinguishable from a
//!   model with no depth precision.
//! - **No Y flip**, because `chassis::CommandList::set_viewport` takes an extent
//!   and both backends build the standard viewport, where NDC `+Y` is up on
//!   screen. A camera that flipped Y in its projection would render correctly on
//!   one backend and upside down on the other, which is the whole failure mode
//!   this crate exists to prevent — and the mirror image of the C++'s
//!   `dynamic_cast` problem: the same call behaving differently underneath
//!   rather than looking different at the seam.
//!
//! ## What the C++ does here, checked rather than assumed
//!
//! `uhepch.h:5-6` defines `GLM_FORCE_DEPTH_ZERO_TO_ONE` before including `glm`,
//! which rewrites GLM's default `perspective` from the `[-1, 1]` variant to the
//! `[0, 1]` one. So the C++'s projection *is* correct for Vulkan, and the obvious
//! "it uses `glm::perspective`, which is `[-1, 1]`" finding is wrong. Worth
//! recording: the header is the contract, and anything including `glm` without it
//! gets a silently different matrix.
//!
//! Its view matrix, though, is a real cost: `EditorCamera.cpp:31-34` builds
//! `translate * orientation` and inverts it *every frame*, for a matrix whose
//! inverse is known by construction.
//!
//! ## Why the frustum is six planes and not a bounding box
//!
//! A view-space AABB test needs the transform into view space per item and then
//! six comparisons against the view bounds: the same work, in a place where it
//! cannot be shared between items. Extracting the planes from the combined matrix
//! once per camera makes the per-item cost six plane distances against a
//! world-space sphere, which is what makes culling worth paying for.
//!
//! ## The degenerate-matrix rule
//!
//! A plane whose normal has zero length, or a matrix containing a NaN, must not
//! cull anything. NaN comparisons are false, so a NaN plane fails
//! `distance < -radius` for *every* item and the whole scene disappears — the
//! opposite of the intended failure, and one that presents as "the renderer works
//! everywhere except this camera".

use glam::camera::rh::proj::directx::{orthographic, perspective};
use glam::camera::rh::view::look_at_mat4;
use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

use crate::bounds::Sphere;
use crate::drawlist::DrawItem;
use crate::stats::CullingStats;

/// How a camera projects.
///
/// An enum rather than a flag: an orthographic shadow camera and a perspective
/// scene camera are different objects that happen to share a frustum extractor,
/// and a bool would let a caller set one and pass the other.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Projection {
    Perspective {
        /// Vertical field of view, radians.
        fov_y: f32,
        aspect: f32,
        near: f32,
        far: f32,
    },
    Orthographic {
        half_width: f32,
        half_height: f32,
        near: f32,
        far: f32,
    },
}

impl Projection {
    pub fn matrix(&self) -> Mat4 {
        match *self {
            Projection::Perspective {
                fov_y,
                aspect,
                near,
                far,
            } => perspective(fov_y, aspect, near, far),
            Projection::Orthographic {
                half_width,
                half_height,
                near,
                far,
            } => orthographic(
                -half_width,
                half_width,
                -half_height,
                half_height,
                near,
                far,
            ),
        }
    }
}

/// A right-handed camera looking from `position` at `target`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub projection: Projection,
}

impl Camera {
    pub fn new(position: Vec3, target: Vec3, up: Vec3, projection: Projection) -> Self {
        Self {
            position,
            target,
            up,
            projection,
        }
    }

    pub fn perspective(
        position: Vec3,
        target: Vec3,
        fov_y: f32,
        aspect: f32,
        near: f32,
        far: f32,
    ) -> Self {
        Self::new(
            position,
            target,
            Vec3::Y,
            Projection::Perspective {
                fov_y,
                aspect,
                near,
                far,
            },
        )
    }

    pub fn orthographic(
        position: Vec3,
        target: Vec3,
        half_width: f32,
        half_height: f32,
        near: f32,
        far: f32,
    ) -> Self {
        Self::new(
            position,
            target,
            Vec3::Y,
            Projection::Orthographic {
                half_width,
                half_height,
                near,
                far,
            },
        )
    }

    /// The view matrix.
    ///
    /// `look_at_mat4` — right-handed, Y-up view space, `-Z` forward — rather than
    /// a translation by the camera's inverse transform, which is what the C++
    /// builds and inverts *every frame* (`EditorCamera.cpp:31-34`) to arrive at
    /// the same matrix.
    pub fn view(&self) -> Mat4 {
        look_at_mat4(self.position, self.target, self.up)
    }

    pub fn projection_matrix(&self) -> Mat4 {
        self.projection.matrix()
    }

    pub fn view_projection(&self) -> Mat4 {
        self.projection.matrix() * self.view()
    }

    /// Forward vector, for a camera that moves rather than one that is aimed.
    pub fn forward(&self) -> Vec3 {
        (self.target - self.position).normalize_or_zero()
    }

    /// Distance to a world-space sphere's centre — the number the depth sort uses.
    ///
    /// The centre rather than the nearest point on the sphere: the nearest point
    /// is marginally better for ordering and costs a subtract and a clamp per item,
    /// and behind a depth test the difference is not visible.
    pub fn distance_to(&self, world: &Sphere) -> f32 {
        self.position.distance(world.centre)
    }

    pub fn frustum(&self) -> Frustum {
        Frustum::from_view_projection(self.view_projection())
    }
}

/// Six half-spaces, each `dot(n, p) + w >= 0` inside.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Gribb–Hartmann: the six planes are sums and differences of the combined
    /// matrix's rows.
    ///
    /// Rows 0 and 1 give left, right, bottom and top. Rows 2 and 3 give the near
    /// and far planes **because the depth range is `[0, 1]`**; with GL's `[-1, 1]`
    /// range the near plane is `row3 + row2` instead — the same code with one sign
    /// different, which is why the range is decided once in
    /// [`Projection::matrix`] and never at a call site.
    pub fn from_view_projection(view_projection: Mat4) -> Self {
        let row = |i: usize| {
            Vec4::new(
                view_projection.col(0)[i],
                view_projection.col(1)[i],
                view_projection.col(2)[i],
                view_projection.col(3)[i],
            )
        };
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        let raw = [
            r3 + r0, // left
            r3 - r0, // right
            r3 + r1, // bottom
            r3 - r1, // top
            r2,      // near:  z >= 0
            r3 - r2, // far:   w - z >= 0
        ];
        let mut planes = [Vec4::ZERO; 6];
        for (plane, value) in planes.iter_mut().zip(raw) {
            *plane = normalise_plane(value);
        }
        Self { planes }
    }

    pub fn planes(&self) -> &[Vec4; 6] {
        &self.planes
    }

    /// True when any part of the sphere can be on screen.
    pub fn contains_sphere(&self, sphere: &Sphere) -> bool {
        self.planes.iter().all(|plane| {
            let distance = plane.xyz().dot(sphere.centre) + plane.w;
            !distance.is_nan() && distance >= -sphere.radius
        })
    }
}

/// Unit normal, or a plane that always passes.
///
/// `Vec4::ZERO` gives `dot = 0` and `0 >= -radius`, so the item survives — which is
/// the right answer for a camera nobody can describe, and the only safe one.
/// Normalising a zero or NaN vector produces NaN, and NaN fails the comparison
/// above, culling the entire scene.
fn normalise_plane(plane: Vec4) -> Vec4 {
    let length = plane.xyz().length();
    if !length.is_finite() || length < 1e-8 {
        return Vec4::ZERO;
    }
    plane / length
}

/// Drop everything the frustum cannot see, in place.
///
/// `retain` rather than rebuild, so survivors keep their relative order and the
/// draw list's allocations are reused. The returned counters are what make a cull
/// debuggable: a scene that renders nothing and a culler that drops everything
/// look identical on screen and completely different here.
///
/// Frustum culling only. A sphere inside the frustum may still be behind a wall;
/// catching that needs a depth pyramid or occlusion queries (Phase 5).
pub fn cull(items: &mut Vec<DrawItem>, frustum: &Frustum) -> CullingStats {
    let tested = items.len() as u32;
    items.retain(|item| frustum.contains_sphere(&item.world_bounds()));
    let culled = tested - items.len() as u32;
    CullingStats { tested, culled }
}
