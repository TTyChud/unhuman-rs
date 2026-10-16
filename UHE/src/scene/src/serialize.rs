//! The `.unhuman` serializer: the C++ YAML format, read and written.
//!
//! Field names are the C++ ones (`SceneSerializer.cpp`), so the scenes in
//! `UNHUMAN/UHE_EDITOR/assets/scene/*.unhuman` load unchanged and a file this
//! port writes is one the C++ editor can still read. The one addition is a
//! `*Path` string beside every asset handle, because a `vault::AssetId` cannot
//! be rebuilt from a file without the registry that issued it.

use std::collections::HashSet;
use std::path::Path;

use glam::{Vec2, Vec3, Vec4};
use serde_yaml::{Mapping, Value};

use crate::camera::{Camera, ProjectionType, SceneCamera};
use crate::components::{
    Animator, AudioSource, Collider, ColliderShape, Light, LightKind, MeshRenderer, RigidBody,
    RigidBodyKind, SpriteAnimation, SpriteRenderer, Tag, Text, Transform,
};
use crate::entity::Entity;
use crate::scene::Scene;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The bytes are not YAML. `line` is the line libyaml stopped on, when it
    /// knew one.
    Yaml {
        line: Option<usize>,
        message: String,
    },
    /// The bytes are YAML but not a scene.
    Format {
        line: Option<usize>,
        message: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Yaml { line, message } | Error::Format { line, message } => match line {
                Some(line) => write!(f, "line {line}: {message}"),
                None => write!(f, "{message}"),
            },
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

pub fn save(scene: &Scene, path: &Path) -> Result<()> {
    std::fs::write(path, to_yaml(scene)?)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Scene> {
    from_yaml(&std::fs::read_to_string(path)?)
}

pub fn to_yaml(scene: &Scene) -> Result<String> {
    let mut root = Mapping::new();
    root.insert(key("Scene"), Value::String(scene.name().to_string()));

    // Roots and their subtrees first, then a sweep for anything the hierarchy
    // did not reach — the C++ serializer's two passes, so no live entity can
    // silently vanish from a save.
    let mut entities = Vec::new();
    let mut written = HashSet::new();
    for entity in scene.entities() {
        if scene.parent(entity).is_none() {
            write_subtree(scene, entity, &mut entities, &mut written);
        }
    }
    for entity in scene.entities() {
        if !written.contains(&entity) {
            write_subtree(scene, entity, &mut entities, &mut written);
        }
    }

    root.insert(key("Entities"), Value::Sequence(entities));
    serde_yaml::to_string(&Value::Mapping(root)).map_err(yaml_error)
}

pub fn from_yaml(text: &str) -> Result<Scene> {
    let root: Value = serde_yaml::from_str(text).map_err(yaml_error)?;
    let root = root.as_mapping().ok_or_else(|| Error::Format {
        line: None,
        message: "the file is not a mapping".to_string(),
    })?;
    if !root.contains_key("Scene") {
        return Err(Error::Format {
            line: None,
            message: "no `Scene` key: this is not an .unhuman scene".to_string(),
        });
    }

    let mut scene = Scene::new();
    scene.set_name(
        root.get("Scene")
            .and_then(Value::as_str)
            .unwrap_or("Untitled"),
    );

    let Some(entities) = root.get("Entities") else {
        return Ok(scene);
    };
    let entities = entities.as_sequence().ok_or_else(|| Error::Format {
        line: None,
        message: "`Entities` is not a sequence".to_string(),
    })?;

    // Pass 1: create everything, restoring serialized ids, so a child can find
    // a parent that appears later in the file.
    let mut created: Vec<Entity> = Vec::with_capacity(entities.len());
    for node in entities {
        let tag = node
            .get("TagComponent")
            .and_then(|tag| tag.get("Tag"))
            .and_then(Value::as_str)
            .unwrap_or("Entity");
        let entity = scene.spawn(tag);
        if let Some(id) = node.get("Entity").and_then(Value::as_u64) {
            if id != 0 {
                if scene.id_is_taken(id) {
                    // The shipped C++ scenes repeat one id on every entity;
                    // refusing the file would make them unloadable, and the
                    // hierarchy below needs distinct ids to mean anything.
                    log::warn!("meats: duplicate Entity id {id}; a fresh id was assigned");
                } else {
                    scene.set_id(entity, id);
                }
            }
        }
        created.push(entity);
    }

    // Pass 2: components, then the parent links. `set_parent` refuses a cycle
    // and ignores a dangling parent, which is the C++ post-load sanitizing.
    for (node, entity) in entities.iter().zip(&created) {
        read_components(&mut scene, *entity, node);
        if let Some(parent) = node
            .get("RelationshipComponent")
            .and_then(|relationship| relationship.get("Parent"))
            .and_then(Value::as_u64)
        {
            if parent != 0 {
                if let Some(parent) = scene.entity_of_id(parent) {
                    scene.set_parent(*entity, Some(parent));
                }
            }
        }
    }

    Ok(scene)
}

fn write_subtree(
    scene: &Scene,
    entity: Entity,
    out: &mut Vec<Value>,
    written: &mut HashSet<Entity>,
) {
    // Exactly-once guard: an id reachable through two paths must serialize once.
    if !written.insert(entity) {
        return;
    }
    out.push(entity_value(scene, entity));
    for child in scene.children(entity).to_vec() {
        // A child entry is followed only when the child points back, so a
        // stale entry cannot serialize twice.
        if scene.parent(child) == Some(entity) {
            write_subtree(scene, child, out, written);
        }
    }
}

fn entity_value(scene: &Scene, entity: Entity) -> Value {
    let mut map = Mapping::new();
    map.insert(
        key("Entity"),
        Value::Number(scene.id_of(entity).unwrap_or(0).into()),
    );

    if let Some(tag) = component::<Tag>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("Tag"), Value::String(tag.tag.clone()));
        map.insert(key("TagComponent"), Value::Mapping(c));
    }

    if let Some(transform) = component::<Transform>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("Translation"), v3(transform.translation));
        c.insert(key("Rotation"), v3(transform.rotation));
        c.insert(key("Scale"), v3(transform.scale));
        map.insert(key("TransformComponent"), Value::Mapping(c));
    }

    if let Some(camera) = component::<Camera>(scene, entity) {
        let mut c = Mapping::new();
        let projection = match camera.camera.projection_type() {
            ProjectionType::Perspective => 0,
            ProjectionType::Orthographic => 1,
        };
        c.insert(key("ProjectionType"), int(projection));
        c.insert(key("PerspectiveFOV"), f32_value(camera.camera.fov()));
        c.insert(
            key("PerspectiveNear"),
            f32_value(camera.camera.perspective_near()),
        );
        c.insert(
            key("PerspectiveFar"),
            f32_value(camera.camera.perspective_far()),
        );
        c.insert(
            key("OrthographicSize"),
            f32_value(camera.camera.orthographic_size()),
        );
        c.insert(
            key("OrthographicNear"),
            f32_value(camera.camera.orthographic_near()),
        );
        c.insert(
            key("OrthographicFar"),
            f32_value(camera.camera.orthographic_far()),
        );
        c.insert(key("Primary"), Value::Bool(camera.primary));
        c.insert(
            key("FixedAspectRatio"),
            Value::Bool(camera.fixed_aspect_ratio),
        );
        map.insert(key("CameraComponent"), Value::Mapping(c));
    }

    if let Some(renderer) = component::<MeshRenderer>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("MeshPath"), Value::String(renderer.mesh_path.clone()));
        c.insert(
            key("MaterialPath"),
            Value::String(renderer.material_path.clone()),
        );
        c.insert(key("Tint"), v4(renderer.tint));
        c.insert(key("Visible"), Value::Bool(renderer.visible));
        c.insert(key("CastShadows"), Value::Bool(renderer.cast_shadows));
        map.insert(key("MeshRendererComponent"), Value::Mapping(c));
    }

    if let Some(sprite) = component::<SpriteRenderer>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("Color"), v4(sprite.color));
        c.insert(
            key("TexturePath"),
            Value::String(sprite.texture_path.clone()),
        );
        c.insert(key("TilingFactor"), f32_value(sprite.tiling_factor));
        c.insert(key("UseSubTexture"), Value::Bool(sprite.use_sub_texture));
        c.insert(key("SubTextureCoords"), v2(sprite.sub_texture_coords));
        c.insert(key("SubTextureCellSize"), v2(sprite.sub_texture_cell_size));
        c.insert(
            key("SubTextureSpriteSize"),
            v2(sprite.sub_texture_sprite_size),
        );
        map.insert(key("SpriteRendererComponent"), Value::Mapping(c));
    }

    if let Some(text) = component::<Text>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("TextString"), Value::String(text.text.clone()));
        c.insert(key("FontAssetPath"), Value::String(text.font_path.clone()));
        c.insert(key("Color"), v4(text.color));
        c.insert(key("Kerning"), f32_value(text.kerning));
        c.insert(key("LineSpacing"), f32_value(text.line_spacing));
        map.insert(key("TextComponent"), Value::Mapping(c));
    }

    if let Some(light) = component::<Light>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("Color"), v3(light.color));
        c.insert(key("Intensity"), f32_value(light.intensity));
        c.insert(key("Radius"), f32_value(light.radius));
        // Radians in the file, degrees in the component — the C++ header's
        // KHR_lights_punctual parity.
        c.insert(
            key("InnerConeAngle"),
            f32_value(light.inner_cone_angle.to_radians()),
        );
        c.insert(
            key("OuterConeAngle"),
            f32_value(light.outer_cone_angle.to_radians()),
        );
        c.insert(key("Enabled"), Value::Bool(light.enabled));
        let name = match light.kind {
            LightKind::Directional => "DirectionalLightComponent",
            LightKind::Point => "PointLightComponent",
            LightKind::Spot => "SpotLightComponent",
        };
        map.insert(key(name), Value::Mapping(c));
    }

    if let Some(body) = component::<RigidBody>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(
            key("BodyType"),
            int(match body.kind {
                RigidBodyKind::Static => 0,
                RigidBodyKind::Kinematic => 1,
                RigidBodyKind::Dynamic => 2,
            }),
        );
        c.insert(key("Mass"), f32_value(body.mass));
        c.insert(key("LinearDamping"), f32_value(body.damping));
        c.insert(key("AngularDamping"), f32_value(body.damping));
        c.insert(key("IsSensor"), Value::Bool(false));
        c.insert(
            key("AllowedDOFs"),
            Value::Sequence(vec![Value::Bool(true); 6]),
        );
        c.insert(key("Velocity"), v3(body.velocity));
        c.insert(key("GravityScale"), f32_value(body.gravity_scale));
        map.insert(key("RigidBody3DComponent"), Value::Mapping(c));
    }

    if let Some(collider) = component::<Collider>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("Offset"), v3(collider.offset));
        c.insert(key("Friction"), f32_value(collider.friction));
        c.insert(key("Restitution"), f32_value(collider.restitution));
        let name = match collider.shape {
            ColliderShape::Box { half_extent } => {
                c.insert(key("HalfExtent"), v3(half_extent));
                "BoxCollider3DComponent"
            }
            ColliderShape::Sphere { radius } => {
                c.insert(key("Radius"), f32_value(radius));
                "SphereCollider3DComponent"
            }
            ColliderShape::Capsule {
                radius,
                half_height,
            } => {
                c.insert(key("Radius"), f32_value(radius));
                c.insert(key("HalfHeight"), f32_value(half_height));
                "CapsuleCollider3DComponent"
            }
        };
        map.insert(key(name), Value::Mapping(c));
    }

    if let Some(audio) = component::<AudioSource>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(key("ClipPath"), Value::String(audio.clip_path.clone()));
        c.insert(key("Playing"), Value::Bool(audio.playing));
        c.insert(key("Looping"), Value::Bool(audio.looping));
        c.insert(key("Gain"), f32_value(audio.gain));
        c.insert(key("Pitch"), f32_value(audio.pitch));
        c.insert(key("Spatial"), Value::Bool(audio.spatial));
        c.insert(key("Range"), f32_value(audio.range));
        map.insert(key("AudioSourceComponent"), Value::Mapping(c));
    }

    if let Some(animator) = component::<Animator>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(
            key("CurrentAnimationName"),
            Value::String(animator.clip_path.clone()),
        );
        c.insert(key("PlaybackSpeed"), f32_value(animator.speed));
        c.insert(key("IsPlaying"), Value::Bool(animator.playing));
        // `RD3d::LoopMode`: None = 0, Loop = 1, PingPong = 2.
        c.insert(key("LoopMode"), int(if animator.looping { 1 } else { 0 }));
        c.insert(key("Time"), f32_value(animator.time));
        map.insert(key("AnimatorComponent"), Value::Mapping(c));
    }

    if let Some(animation) = component::<SpriteAnimation>(scene, entity) {
        let mut c = Mapping::new();
        c.insert(
            key("SpriteSheetPath"),
            Value::String(animation.sprite_sheet_path.clone()),
        );
        c.insert(key("FrameSizeX"), f32_value(animation.frame_size.x));
        c.insert(key("FrameSizeY"), f32_value(animation.frame_size.y));
        c.insert(
            key("FrameCount"),
            Value::Number(u64::from(animation.frame_count).into()),
        );
        c.insert(key("FrameDuration"), f32_value(animation.frame_duration));
        c.insert(key("Loop"), Value::Bool(animation.looping));
        c.insert(key("Color"), v4(animation.color));
        c.insert(key("Time"), f32_value(animation.time));
        map.insert(key("SpriteAnimationComponent"), Value::Mapping(c));
    }

    let mut relationship = Mapping::new();
    relationship.insert(
        key("Parent"),
        Value::Number(
            scene
                .parent(entity)
                .and_then(|p| scene.id_of(p))
                .unwrap_or(0)
                .into(),
        ),
    );
    relationship.insert(
        key("Children"),
        Value::Sequence(
            scene
                .children(entity)
                .iter()
                .map(|child| Value::Number(scene.id_of(*child).unwrap_or(0).into()))
                .collect(),
        ),
    );
    map.insert(key("RelationshipComponent"), Value::Mapping(relationship));

    Value::Mapping(map)
}

fn read_components(scene: &mut Scene, entity: Entity, node: &Value) {
    if let Some(node) = node.get("TransformComponent") {
        if let Some(mut view) = scene.get_mut(entity) {
            if let Some(transform) = view.get_mut::<Transform>() {
                if let Some(value) = node.get("Translation").and_then(read_v3) {
                    transform.translation = value;
                }
                if let Some(value) = node.get("Rotation").and_then(read_v3) {
                    transform.rotation = value;
                }
                if let Some(value) = node.get("Scale").and_then(read_v3) {
                    transform.scale = value;
                }
            }
        }
    }

    if let Some(node) = node.get("CameraComponent") {
        let mut camera = SceneCamera::default();
        if let Some(value) = node.get("ProjectionType").and_then(Value::as_i64) {
            camera.set_projection_type(if value == 1 {
                ProjectionType::Orthographic
            } else {
                ProjectionType::Perspective
            });
        }
        if let Some(value) = node.get("PerspectiveFOV").and_then(as_f32) {
            camera.set_fov(value);
        }
        if let Some(value) = node.get("PerspectiveNear").and_then(as_f32) {
            camera.set_perspective_near(value);
        }
        if let Some(value) = node.get("PerspectiveFar").and_then(as_f32) {
            camera.set_perspective_far(value);
        }
        if let Some(value) = node.get("OrthographicSize").and_then(as_f32) {
            camera.set_orthographic_size(value);
        }
        if let Some(value) = node.get("OrthographicNear").and_then(as_f32) {
            camera.set_orthographic_near(value);
        }
        if let Some(value) = node.get("OrthographicFar").and_then(as_f32) {
            camera.set_orthographic_far(value);
        }
        let default = Camera::default();
        scene.insert(
            entity,
            Camera {
                camera,
                primary: node
                    .get("Primary")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.primary),
                fixed_aspect_ratio: node
                    .get("FixedAspectRatio")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.fixed_aspect_ratio),
            },
        );
    }

    if let Some(node) = node.get("MeshRendererComponent") {
        let default = MeshRenderer::default();
        scene.insert(
            entity,
            MeshRenderer {
                mesh: None,
                mesh_path: node
                    .get("MeshPath")
                    .and_then(read_string)
                    .unwrap_or(default.mesh_path),
                material: None,
                material_path: node
                    .get("MaterialPath")
                    .and_then(read_string)
                    .unwrap_or(default.material_path),
                tint: node.get("Tint").and_then(read_v4).unwrap_or(default.tint),
                visible: node
                    .get("Visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.visible),
                cast_shadows: node
                    .get("CastShadows")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.cast_shadows),
            },
        );
    }

    if let Some(node) = node.get("SpriteRendererComponent") {
        let default = SpriteRenderer::default();
        scene.insert(
            entity,
            SpriteRenderer {
                color: node.get("Color").and_then(read_v4).unwrap_or(default.color),
                texture: None,
                texture_path: node
                    .get("TexturePath")
                    .and_then(read_string)
                    .unwrap_or(default.texture_path),
                tiling_factor: node
                    .get("TilingFactor")
                    .and_then(as_f32)
                    .unwrap_or(default.tiling_factor),
                use_sub_texture: node
                    .get("UseSubTexture")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.use_sub_texture),
                sub_texture_coords: node
                    .get("SubTextureCoords")
                    .and_then(read_v2)
                    .unwrap_or(default.sub_texture_coords),
                sub_texture_cell_size: node
                    .get("SubTextureCellSize")
                    .and_then(read_v2)
                    .unwrap_or(default.sub_texture_cell_size),
                sub_texture_sprite_size: node
                    .get("SubTextureSpriteSize")
                    .and_then(read_v2)
                    .unwrap_or(default.sub_texture_sprite_size),
            },
        );
    }

    if let Some(node) = node.get("TextComponent") {
        // The C++ key is `FontAssetPath`; `FontAsset` is accepted too because the
        // renderer contract named it that way.
        let font_path = node
            .get("FontAssetPath")
            .or_else(|| node.get("FontAsset"))
            .and_then(read_string);
        let default = Text::default();
        scene.insert(
            entity,
            Text {
                text: node
                    .get("TextString")
                    .and_then(read_string)
                    .unwrap_or(default.text),
                font: None,
                font_path: font_path.unwrap_or(default.font_path),
                color: node.get("Color").and_then(read_v4).unwrap_or(default.color),
                kerning: node
                    .get("Kerning")
                    .and_then(as_f32)
                    .unwrap_or(default.kerning),
                line_spacing: node
                    .get("LineSpacing")
                    .and_then(as_f32)
                    .unwrap_or(default.line_spacing),
            },
        );
    }

    for (name, kind) in [
        ("DirectionalLightComponent", LightKind::Directional),
        ("PointLightComponent", LightKind::Point),
        ("SpotLightComponent", LightKind::Spot),
    ] {
        let Some(node) = node.get(name) else {
            continue;
        };
        let default = Light::default();
        scene.insert(
            entity,
            Light {
                kind,
                color: node.get("Color").and_then(read_v3).unwrap_or(default.color),
                intensity: node
                    .get("Intensity")
                    .and_then(as_f32)
                    .unwrap_or(default.intensity),
                radius: node
                    .get("Radius")
                    .and_then(as_f32)
                    .unwrap_or(default.radius),
                inner_cone_angle: node
                    .get("InnerConeAngle")
                    .and_then(as_f32)
                    .map(f32::to_degrees)
                    .unwrap_or(default.inner_cone_angle),
                outer_cone_angle: node
                    .get("OuterConeAngle")
                    .and_then(as_f32)
                    .map(f32::to_degrees)
                    .unwrap_or(default.outer_cone_angle),
                enabled: node
                    .get("Enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.enabled),
            },
        );
    }

    if let Some(node) = node.get("RigidBody3DComponent") {
        let default = RigidBody::default();
        let kind = match node.get("BodyType").and_then(Value::as_i64) {
            Some(1) => RigidBodyKind::Kinematic,
            Some(2) => RigidBodyKind::Dynamic,
            Some(_) => RigidBodyKind::Static,
            None => default.kind,
        };
        scene.insert(
            entity,
            RigidBody {
                kind,
                mass: node.get("Mass").and_then(as_f32).unwrap_or(default.mass),
                velocity: node
                    .get("Velocity")
                    .and_then(read_v3)
                    .unwrap_or(default.velocity),
                damping: node
                    .get("LinearDamping")
                    .and_then(as_f32)
                    .unwrap_or(default.damping),
                gravity_scale: node
                    .get("GravityScale")
                    .and_then(as_f32)
                    .unwrap_or(default.gravity_scale),
            },
        );
    }

    if let Some(node) = node.get("BoxCollider3DComponent") {
        let default = Collider::default();
        let shape = match node.get("HalfExtent").and_then(read_v3) {
            Some(half_extent) => ColliderShape::Box { half_extent },
            None => default.shape,
        };
        scene.insert(
            entity,
            Collider {
                shape,
                offset: node
                    .get("Offset")
                    .and_then(read_v3)
                    .unwrap_or(default.offset),
                friction: node
                    .get("Friction")
                    .and_then(as_f32)
                    .unwrap_or(default.friction),
                restitution: node
                    .get("Restitution")
                    .and_then(as_f32)
                    .unwrap_or(default.restitution),
            },
        );
    } else if let Some(node) = node.get("SphereCollider3DComponent") {
        let default = Collider::default();
        scene.insert(
            entity,
            Collider {
                shape: ColliderShape::Sphere {
                    // The C++ `SphereCollider3DComponent` default radius.
                    radius: node.get("Radius").and_then(as_f32).unwrap_or(0.5),
                },
                offset: node
                    .get("Offset")
                    .and_then(read_v3)
                    .unwrap_or(default.offset),
                friction: node
                    .get("Friction")
                    .and_then(as_f32)
                    .unwrap_or(default.friction),
                restitution: node
                    .get("Restitution")
                    .and_then(as_f32)
                    .unwrap_or(default.restitution),
            },
        );
    } else if let Some(node) = node.get("CapsuleCollider3DComponent") {
        let default = Collider::default();
        scene.insert(
            entity,
            Collider {
                shape: ColliderShape::Capsule {
                    // The C++ `CapsuleCollider3DComponent` defaults.
                    radius: node.get("Radius").and_then(as_f32).unwrap_or(0.5),
                    half_height: node.get("HalfHeight").and_then(as_f32).unwrap_or(1.0),
                },
                offset: node
                    .get("Offset")
                    .and_then(read_v3)
                    .unwrap_or(default.offset),
                friction: node
                    .get("Friction")
                    .and_then(as_f32)
                    .unwrap_or(default.friction),
                restitution: node
                    .get("Restitution")
                    .and_then(as_f32)
                    .unwrap_or(default.restitution),
            },
        );
    }

    if let Some(node) = node.get("AudioSourceComponent") {
        let default = AudioSource::default();
        scene.insert(
            entity,
            AudioSource {
                clip: None,
                clip_path: node
                    .get("ClipPath")
                    .and_then(read_string)
                    .unwrap_or(default.clip_path),
                playing: node
                    .get("Playing")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.playing),
                looping: node
                    .get("Looping")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.looping),
                gain: node.get("Gain").and_then(as_f32).unwrap_or(default.gain),
                pitch: node.get("Pitch").and_then(as_f32).unwrap_or(default.pitch),
                spatial: node
                    .get("Spatial")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.spatial),
                range: node.get("Range").and_then(as_f32).unwrap_or(default.range),
            },
        );
    }

    if let Some(node) = node.get("AnimatorComponent") {
        let default = Animator::default();
        scene.insert(
            entity,
            Animator {
                clip: None,
                clip_path: node
                    .get("CurrentAnimationName")
                    .and_then(read_string)
                    .unwrap_or(default.clip_path),
                time: node.get("Time").and_then(as_f32).unwrap_or(default.time),
                speed: node
                    .get("PlaybackSpeed")
                    .and_then(as_f32)
                    .unwrap_or(default.speed),
                // `RD3d::LoopMode`: None = 0, Loop = 1, PingPong = 2.
                looping: match node.get("LoopMode").and_then(Value::as_i64) {
                    Some(mode) => mode == 1,
                    None => default.looping,
                },
                playing: node
                    .get("IsPlaying")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.playing),
            },
        );
    }

    if let Some(node) = node.get("SpriteAnimationComponent") {
        let default = SpriteAnimation::default();
        scene.insert(
            entity,
            SpriteAnimation {
                sprite_sheet_path: node
                    .get("SpriteSheetPath")
                    .and_then(read_string)
                    .unwrap_or(default.sprite_sheet_path),
                sheet: None,
                frame_size: Vec2::new(
                    node.get("FrameSizeX")
                        .and_then(as_f32)
                        .unwrap_or(default.frame_size.x),
                    node.get("FrameSizeY")
                        .and_then(as_f32)
                        .unwrap_or(default.frame_size.y),
                ),
                frame_count: node
                    .get("FrameCount")
                    .and_then(Value::as_u64)
                    .map(|count| count as u32)
                    .unwrap_or(default.frame_count),
                frame_duration: node
                    .get("FrameDuration")
                    .and_then(as_f32)
                    .unwrap_or(default.frame_duration),
                looping: node
                    .get("Loop")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.looping),
                color: node.get("Color").and_then(read_v4).unwrap_or(default.color),
                time: node.get("Time").and_then(as_f32).unwrap_or(default.time),
            },
        );
    }

    // A component name this port does not know is skipped, not fatal: the C++
    // tree writes several (`Model3DComponent`, `ModelNodeComponent`,
    // `RigidBody2DComponent`, `BoxColliderComponent`) that are deliberately not
    // ported, and an unknown name from a newer editor must not make a scene
    // unloadable.
    if let Some(map) = node.as_mapping() {
        for name in map.keys() {
            if let Some(name) = name.as_str() {
                if name.ends_with("Component") && !KNOWN_COMPONENTS.contains(&name) {
                    log::debug!("meats: skipping unknown component `{name}`");
                }
            }
        }
    }
}

const KNOWN_COMPONENTS: &[&str] = &[
    "TagComponent",
    "TransformComponent",
    "CameraComponent",
    "MeshRendererComponent",
    "SpriteRendererComponent",
    "TextComponent",
    "SpriteAnimationComponent",
    "DirectionalLightComponent",
    "PointLightComponent",
    "SpotLightComponent",
    "RigidBody3DComponent",
    "BoxCollider3DComponent",
    "SphereCollider3DComponent",
    "CapsuleCollider3DComponent",
    "AudioSourceComponent",
    "AnimatorComponent",
    "RelationshipComponent",
];

fn component<'a, T: hecs::Component>(scene: &'a Scene, entity: Entity) -> Option<hecs::Ref<'a, T>> {
    scene.get(entity)?.get::<T>()
}

fn key(name: &str) -> Value {
    Value::String(name.to_string())
}

fn f32_value(value: f32) -> Value {
    // `Number` stores an f64, so going through `From<f32>` would print
    // `0.05000000074505806` for `0.05f32`. Re-parsing the shortest decimal that
    // reproduces the f32 keeps the file the size and shape the C++ emitter
    // wrote; the fallback covers the spellings YAML does not share with Rust
    // (`inf`, `NaN`).
    match value.to_string().parse::<serde_yaml::Number>() {
        Ok(number) => Value::Number(number),
        Err(_) => Value::Number(value.into()),
    }
}

fn int(value: i32) -> Value {
    Value::Number(value.into())
}

fn v2(value: Vec2) -> Value {
    Value::Sequence(vec![f32_value(value.x), f32_value(value.y)])
}

fn v3(value: Vec3) -> Value {
    Value::Sequence(vec![
        f32_value(value.x),
        f32_value(value.y),
        f32_value(value.z),
    ])
}

fn v4(value: Vec4) -> Value {
    Value::Sequence(vec![
        f32_value(value.x),
        f32_value(value.y),
        f32_value(value.z),
        f32_value(value.w),
    ])
}

fn as_f32(value: &Value) -> Option<f32> {
    value.as_f64().map(|value| value as f32)
}

fn read_string(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

fn read_v2(value: &Value) -> Option<Vec2> {
    let values = value.as_sequence()?;
    Some(Vec2::new(as_f32(values.first()?)?, as_f32(values.get(1)?)?))
}

fn read_v3(value: &Value) -> Option<Vec3> {
    let values = value.as_sequence()?;
    Some(Vec3::new(
        as_f32(values.first()?)?,
        as_f32(values.get(1)?)?,
        as_f32(values.get(2)?)?,
    ))
}

fn read_v4(value: &Value) -> Option<Vec4> {
    let values = value.as_sequence()?;
    Some(Vec4::new(
        as_f32(values.first()?)?,
        as_f32(values.get(1)?)?,
        as_f32(values.get(2)?)?,
        as_f32(values.get(3)?)?,
    ))
}

fn yaml_error(err: serde_yaml::Error) -> Error {
    Error::Yaml {
        line: err.location().map(|location| location.line()),
        message: err.to_string(),
    }
}
