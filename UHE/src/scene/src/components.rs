//! The component set. Names follow the crate-map contract (`Transform`, not
//! `TransformComponent`); the `.unhuman` field names stay the C++ ones and live
//! in [`crate::serialize`].

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::entity::Entity;
use crate::scene::Scene;

/// Port of `TransformComponent`. `rotation` is Euler **degrees** — the C++
/// header says so, and `glm::quat(Rotation)` treats them that way.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::default()
        }
    }

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation_quat(), self.translation)
    }

    /// Inverse of [`matrix`](Self::matrix) — what a camera's view matrix is.
    pub fn view_matrix(&self) -> Mat4 {
        self.matrix().inverse()
    }

    /// `glm::quat(vec3)` from `glm/gtx/quaternion.hpp`: the C++ builds it in the
    /// X-then-Y-then-Z order, which glam's `EulerRot` sequences do not
    /// reproduce, so the formula is written out.
    pub fn rotation_quat(&self) -> Quat {
        let (x, y, z) = (
            self.rotation.x.to_radians() * 0.5,
            self.rotation.y.to_radians() * 0.5,
            self.rotation.z.to_radians() * 0.5,
        );
        let (sx, cx) = x.sin_cos();
        let (sy, cy) = y.sin_cos();
        let (sz, cz) = z.sin_cos();
        Quat::from_xyzw(
            sx * cy * cz - cx * sy * sz,
            cx * sy * cz + sx * cy * sz,
            cx * cy * sz - sx * sy * cz,
            cx * cy * cz + sx * sy * sz,
        )
    }
}

/// Port of `TagComponent`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Tag {
    pub tag: String,
}

impl Tag {
    pub fn new(tag: impl Into<String>) -> Self {
        Self { tag: tag.into() }
    }
}

/// Port of `IDComponent`: one real unique id per entity. There is deliberately
/// no `Default` — the C++ comment records that defaulting to 0 made every
/// entity share identity 0, so every id-based lookup silently returned the
/// first entity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Id(pub u64);

/// The 3D mesh on an entity. The C++ spread this over `Model3DComponent` and
/// `ModelNodeComponent`; the loader expands glTF nodes into one entity per node,
/// so a handle, an optional material and how it shades is the whole component.
#[derive(Clone, PartialEq, Debug)]
pub struct MeshRenderer {
    /// Filled by [`Scene::resolve_assets`]; `None` until then, and `None`
    /// forever for a path the registry does not have.
    pub mesh: Option<vault::AssetId>,
    /// What `.unhuman` stores. A handle cannot be rebuilt from a file without
    /// the registry that issued it.
    pub mesh_path: String,
    pub material: Option<vault::AssetId>,
    pub material_path: String,
    pub tint: Vec4,
    pub visible: bool,
    pub cast_shadows: bool,
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            mesh: None,
            mesh_path: String::new(),
            material: None,
            material_path: String::new(),
            tint: Vec4::ONE,
            visible: true,
            cast_shadows: true,
        }
    }
}

/// Port of `SpriteRendererComponent`.
#[derive(Clone, PartialEq, Debug)]
pub struct SpriteRenderer {
    pub color: Vec4,
    pub texture: Option<vault::AssetId>,
    pub texture_path: String,
    pub tiling_factor: f32,
    pub use_sub_texture: bool,
    pub sub_texture_coords: Vec2,
    pub sub_texture_cell_size: Vec2,
    pub sub_texture_sprite_size: Vec2,
}

impl Default for SpriteRenderer {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            texture: None,
            texture_path: String::new(),
            tiling_factor: 1.0,
            use_sub_texture: false,
            sub_texture_coords: Vec2::ZERO,
            sub_texture_cell_size: Vec2::splat(16.0),
            sub_texture_sprite_size: Vec2::ONE,
        }
    }
}

/// Port of `TextComponent`. `font_path` is the C++ `FontAssetPath`; the handle
/// beside it is filled by [`Scene::resolve_assets`].
#[derive(Clone, PartialEq, Debug)]
pub struct Text {
    pub text: String,
    pub font: Option<vault::AssetId>,
    pub font_path: String,
    pub color: Vec4,
    pub kerning: f32,
    pub line_spacing: f32,
}

impl Default for Text {
    fn default() -> Self {
        Self {
            text: "Text Node".to_string(),
            font: None,
            font_path: String::new(),
            color: Vec4::ONE,
            kerning: 0.0,
            line_spacing: 0.0,
        }
    }
}

/// Port of `DirectionalLightComponent` / `PointLightComponent` /
/// `SpotLightComponent` as one component; the C++ allowed an entity to carry
/// several light components at once, which nothing did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

/// Port of the three light components. Cone angles are **degrees** here, where
/// the C++ stored radians for the KHR_lights_punctual parity.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Light {
    pub kind: LightKind,
    pub color: Vec3,
    pub intensity: f32,
    /// Point and spot only.
    pub radius: f32,
    pub inner_cone_angle: f32,
    pub outer_cone_angle: f32,
    pub enabled: bool,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            kind: LightKind::Directional,
            color: Vec3::ONE,
            intensity: 1.0,
            radius: 10.0,
            // 0.2617994 rad and 0.5235988 rad in the C++ header.
            inner_cone_angle: 15.0,
            outer_cone_angle: 30.0,
            enabled: true,
        }
    }
}

/// Port of the `BodyType` nested enum shared by `RigidBody2DComponent` and
/// `RigidBody3DComponent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RigidBodyKind {
    Static,
    Kinematic,
    Dynamic,
}

/// Port of `RigidBody3DComponent` — the 3D path is the product, so the 2D body
/// is not carried over. `velocity` and `gravity_scale` are the fields the
/// physics backend owns; `damping` is the C++ `LinearDamping`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RigidBody {
    pub kind: RigidBodyKind,
    pub mass: f32,
    pub velocity: Vec3,
    pub damping: f32,
    pub gravity_scale: f32,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            kind: RigidBodyKind::Static,
            mass: 1.0,
            velocity: Vec3::ZERO,
            damping: 0.05,
            gravity_scale: 1.0,
        }
    }
}

/// Port of `BoxCollider3DComponent` / `SphereCollider3DComponent` /
/// `CapsuleCollider3DComponent` as one component.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ColliderShape {
    Box { half_extent: Vec3 },
    Sphere { radius: f32 },
    /// Total height is `2 * (radius + half_height)`.
    Capsule { radius: f32, half_height: f32 },
}

impl Default for ColliderShape {
    fn default() -> Self {
        ColliderShape::Box {
            half_extent: Vec3::splat(0.5),
        }
    }
}

/// Port of the three 3D collider components.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Collider {
    pub shape: ColliderShape,
    pub offset: Vec3,
    pub friction: f32,
    pub restitution: f32,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::default(),
            offset: Vec3::ZERO,
            friction: 0.2,
            restitution: 0.0,
        }
    }
}

/// The C++ tree has no audio component — audio was driven from code. This is
/// what the port gives `wire` to read.
#[derive(Clone, PartialEq, Debug)]
pub struct AudioSource {
    pub clip: Option<vault::AssetId>,
    pub clip_path: String,
    pub playing: bool,
    pub looping: bool,
    pub gain: f32,
    pub pitch: f32,
    pub spatial: bool,
    /// Distance at which a spatial source has attenuated to silence.
    pub range: f32,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self {
            clip: None,
            clip_path: String::new(),
            playing: false,
            looping: false,
            gain: 1.0,
            pitch: 1.0,
            spatial: false,
            range: 10.0,
        }
    }
}

/// Port of `AnimatorComponent`, flattened to the fields a renderer needs. The
/// C++ kept a `Ref<RD3d::Animator>` runtime object and mirrored eight
/// parameters onto it; the runtime object is `bifrost`'s, so only the state
/// lives here.
#[derive(Clone, PartialEq, Debug)]
pub struct Animator {
    pub clip: Option<vault::AssetId>,
    /// The C++ `CurrentAnimationName`.
    pub clip_path: String,
    pub time: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            clip: None,
            clip_path: String::new(),
            time: 0.0,
            speed: 1.0,
            looping: true,
            playing: false,
        }
    }
}

/// What a renderer is handed: the animator without the path string, so a sample
/// stays `Copy`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AnimatorState {
    pub clip: Option<vault::AssetId>,
    pub time: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
}

impl From<&Animator> for AnimatorState {
    fn from(animator: &Animator) -> Self {
        Self {
            clip: animator.clip,
            time: animator.time,
            speed: animator.speed,
            looping: animator.looping,
            playing: animator.playing,
        }
    }
}

/// Port of `SpriteAnimationComponent` plus the playback clock the C++
/// `SpriteAnimation` kept internally.
#[derive(Clone, PartialEq, Debug)]
pub struct SpriteAnimation {
    pub sprite_sheet_path: String,
    pub sheet: Option<vault::AssetId>,
    pub frame_size: Vec2,
    pub frame_count: u32,
    pub frame_duration: f32,
    pub looping: bool,
    pub color: Vec4,
    pub time: f32,
}

impl Default for SpriteAnimation {
    fn default() -> Self {
        Self {
            sprite_sheet_path: String::new(),
            sheet: None,
            frame_size: Vec2::ONE,
            frame_count: 1,
            frame_duration: 0.1,
            looping: true,
            color: Vec4::ONE,
            time: 0.0,
        }
    }
}

impl SpriteAnimation {
    /// Frame index for the current [`time`](Self::time).
    pub fn current_frame(&self) -> u32 {
        if self.frame_count == 0 || self.frame_duration <= 0.0 {
            return 0;
        }
        let frame = (self.time / self.frame_duration) as u32;
        if self.looping {
            frame % self.frame_count
        } else {
            frame.min(self.frame_count - 1)
        }
    }
}

/// Behaviour attached to an entity. Replaces the C++ `NativeScriptComponent`'s
/// `InstantiateScript` / `DestroyScript` function pointers and the raw
/// `ScriptableEntity*` they owned — the one place this port deliberately
/// changes shape, because that ownership was the C++'s unsafe part.
pub trait Script: Send + Sync {
    fn on_create(&mut self, entity: Entity, scene: &mut Scene);
    fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: f32);
    fn on_destroy(&mut self, entity: Entity, scene: &mut Scene);
}

/// Port of `NativeScriptComponent`.
pub struct NativeScript {
    pub script: Box<dyn Script>,
    /// `on_create` fires on the first [`Scene::update_scripts`], which is where
    /// the C++ instantiated lazily.
    created: bool,
}

impl NativeScript {
    pub fn new(script: impl Script + 'static) -> Self {
        Self {
            script: Box::new(script),
            created: false,
        }
    }

    pub(crate) fn take_created(&mut self) -> bool {
        let created = self.created;
        self.created = true;
        created
    }
}

impl std::fmt::Debug for NativeScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeScript")
            .field("created", &self.created)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_order_matches_glm() {
        // glm::quat(vec3(90, 90, 0)) for the X-then-Y-then-Z order the C++
        // header's `glm::quat(Rotation)` uses.
        let transform = Transform {
            rotation: Vec3::new(90.0, 90.0, 0.0),
            ..Transform::default()
        };
        let quat = transform.rotation_quat();
        assert!((quat.x - 0.5).abs() < 1e-6);
        assert!((quat.y - 0.5).abs() < 1e-6);
        assert!((quat.z + 0.5).abs() < 1e-6);
        assert!((quat.w - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sprite_animation_frame_wraps_when_looping() {
        let mut animation = SpriteAnimation {
            frame_count: 4,
            frame_duration: 0.25,
            ..SpriteAnimation::default()
        };
        animation.time = 1.1;
        assert_eq!(animation.current_frame(), 0);
        animation.looping = false;
        assert_eq!(animation.current_frame(), 3);
    }
}
