//! The `.unhuman` YAML round trip, and the C++ scenes it has to read.

use std::path::Path;

use glam::{Vec2, Vec3, Vec4};
use meats::{
    from_yaml, load, to_yaml, Animator, AudioSource, Camera, Collider, ColliderShape, Entity,
    Light, LightKind, MeshRenderer, ProjectionType, RigidBody, RigidBodyKind, Scene, SceneCamera,
    SpriteAnimation, SpriteRenderer, Text, Transform,
};

fn component<T: Clone + meats::hecs::Component>(scene: &Scene, entity: Entity) -> T {
    (*scene.get(entity).unwrap().get::<T>().unwrap()).clone()
}

fn entity_named(scene: &Scene, tag: &str) -> Entity {
    scene
        .entities()
        .into_iter()
        .find(|entity| scene.get(*entity).unwrap().tag().as_deref() == Some(tag))
        .unwrap_or_else(|| panic!("no entity tagged `{tag}`"))
}

/// One of every component this crate ports. `NativeScript` is absent because a
/// `Box<dyn Script>` has no serialized form — the C++ serializer skips
/// `NativeScriptComponent` for the same reason.
fn every_component_scene() -> Scene {
    let mut scene = Scene::new();
    scene.set_name("Round Trip");

    let root = scene.spawn("Root");
    scene.insert(
        root,
        Camera {
            camera: SceneCamera::new_perspective(0.75, 0.05, 250.0),
            primary: true,
            fixed_aspect_ratio: true,
        },
    );
    scene.insert(
        root,
        MeshRenderer {
            mesh_path: "models/cube.glb".to_string(),
            material_path: "materials/brass.mat".to_string(),
            tint: Vec4::new(1.0, 0.5, 0.25, 0.75),
            visible: true,
            cast_shadows: false,
            ..MeshRenderer::default()
        },
    );
    scene.insert(
        root,
        Light {
            kind: LightKind::Spot,
            color: Vec3::new(1.0, 0.5, 0.25),
            intensity: 2.5,
            radius: 12.5,
            inner_cone_angle: 20.0,
            outer_cone_angle: 40.0,
            enabled: false,
        },
    );
    scene.insert(
        root,
        RigidBody {
            kind: RigidBodyKind::Dynamic,
            mass: 3.5,
            velocity: Vec3::new(0.5, -0.25, 0.125),
            damping: 0.5,
            gravity_scale: 0.25,
        },
    );
    scene.insert(
        root,
        Collider {
            shape: ColliderShape::Capsule {
                radius: 0.25,
                half_height: 1.5,
            },
            offset: Vec3::new(0.0, 1.0, 0.0),
            friction: 0.75,
            restitution: 0.25,
        },
    );
    scene.insert(
        root,
        AudioSource {
            clip_path: "audio/theme.ogg".to_string(),
            playing: true,
            looping: true,
            gain: 0.5,
            pitch: 1.25,
            spatial: true,
            range: 30.0,
            ..AudioSource::default()
        },
    );
    scene.insert(
        root,
        Animator {
            clip_path: "walk".to_string(),
            time: 1.5,
            speed: 2.0,
            looping: false,
            playing: true,
            ..Animator::default()
        },
    );
    scene.insert(
        root,
        SpriteAnimation {
            sprite_sheet_path: "sprites/sheet.png".to_string(),
            frame_size: Vec2::new(32.0, 16.0),
            frame_count: 8,
            frame_duration: 0.125,
            looping: false,
            color: Vec4::new(0.25, 0.5, 0.75, 1.0),
            time: 0.5,
            ..SpriteAnimation::default()
        },
    );
    scene.insert(
        root,
        SpriteRenderer {
            color: Vec4::new(1.0, 0.0, 0.0, 1.0),
            texture_path: "sprites/player.png".to_string(),
            tiling_factor: 2.0,
            use_sub_texture: true,
            sub_texture_coords: Vec2::new(1.0, 2.0),
            sub_texture_cell_size: Vec2::new(8.0, 8.0),
            sub_texture_sprite_size: Vec2::new(2.0, 2.0),
            ..SpriteRenderer::default()
        },
    );
    scene.insert(
        root,
        Text {
            text: "Hello".to_string(),
            font_path: "fonts/inter.ttf".to_string(),
            color: Vec4::new(0.5, 0.5, 0.5, 1.0),
            kerning: 0.5,
            line_spacing: 1.5,
            ..Text::default()
        },
    );

    let child = scene.spawn("Child");
    scene.insert(
        child,
        Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Vec3::new(0.0, 90.0, 0.0),
            scale: Vec3::splat(0.5),
        },
    );
    scene.set_parent(child, Some(root));

    scene
}

fn round_trip(scene: &Scene) -> Scene {
    from_yaml(&to_yaml(scene).expect("serializes")).expect("deserializes")
}

#[test]
fn every_component_survives_a_round_trip() {
    let scene = every_component_scene();
    let reloaded = round_trip(&scene);

    assert_eq!(reloaded.name(), "Round Trip");
    assert_eq!(reloaded.entities().len(), 2);

    let root = entity_named(&reloaded, "Root");
    let child = entity_named(&reloaded, "Child");
    assert_eq!(reloaded.parent(child), Some(root));
    assert_eq!(reloaded.children(root).to_vec(), vec![child]);
    assert_ne!(reloaded.id_of(root), reloaded.id_of(child));

    let transform: Transform = component(&reloaded, child);
    assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(transform.rotation, Vec3::new(0.0, 90.0, 0.0));
    assert_eq!(transform.scale, Vec3::splat(0.5));

    let camera: Camera = component(&reloaded, root);
    assert_eq!(camera.camera.projection_type(), ProjectionType::Perspective);
    assert_eq!(camera.camera.fov(), 0.75);
    assert_eq!(camera.camera.perspective_near(), 0.05);
    assert_eq!(camera.camera.perspective_far(), 250.0);
    assert!(camera.primary);
    assert!(camera.fixed_aspect_ratio);

    let renderer: MeshRenderer = component(&reloaded, root);
    assert_eq!(renderer.mesh_path, "models/cube.glb");
    assert_eq!(renderer.material_path, "materials/brass.mat");
    assert_eq!(renderer.tint, Vec4::new(1.0, 0.5, 0.25, 0.75));
    assert!(renderer.visible);
    assert!(!renderer.cast_shadows);
    // A path survives with no handle: `vault::AssetId` cannot be rebuilt from a
    // file, so the loader leaves it unset rather than failing.
    assert!(renderer.mesh.is_none());
    assert!(renderer.material.is_none());

    let light: Light = component(&reloaded, root);
    assert_eq!(light.kind, LightKind::Spot);
    assert_eq!(light.color, Vec3::new(1.0, 0.5, 0.25));
    assert_eq!(light.intensity, 2.5);
    assert_eq!(light.radius, 12.5);
    assert!((light.inner_cone_angle - 20.0).abs() < 1e-3);
    assert!((light.outer_cone_angle - 40.0).abs() < 1e-3);
    assert!(!light.enabled);

    let body: RigidBody = component(&reloaded, root);
    assert_eq!(body.kind, RigidBodyKind::Dynamic);
    assert_eq!(body.mass, 3.5);
    assert_eq!(body.velocity, Vec3::new(0.5, -0.25, 0.125));
    assert_eq!(body.damping, 0.5);
    assert_eq!(body.gravity_scale, 0.25);

    let collider: Collider = component(&reloaded, root);
    assert_eq!(
        collider.shape,
        ColliderShape::Capsule {
            radius: 0.25,
            half_height: 1.5
        }
    );
    assert_eq!(collider.offset, Vec3::new(0.0, 1.0, 0.0));
    assert_eq!(collider.friction, 0.75);
    assert_eq!(collider.restitution, 0.25);

    let audio: AudioSource = component(&reloaded, root);
    assert_eq!(audio.clip_path, "audio/theme.ogg");
    assert!(audio.playing);
    assert!(audio.looping);
    assert_eq!(audio.gain, 0.5);
    assert_eq!(audio.pitch, 1.25);
    assert!(audio.spatial);
    assert_eq!(audio.range, 30.0);
    assert!(audio.clip.is_none());

    let animator: Animator = component(&reloaded, root);
    assert_eq!(animator.clip_path, "walk");
    assert_eq!(animator.time, 1.5);
    assert_eq!(animator.speed, 2.0);
    assert!(!animator.looping);
    assert!(animator.playing);

    let animation: SpriteAnimation = component(&reloaded, root);
    assert_eq!(animation.sprite_sheet_path, "sprites/sheet.png");
    assert_eq!(animation.frame_size, Vec2::new(32.0, 16.0));
    assert_eq!(animation.frame_count, 8);
    assert_eq!(animation.frame_duration, 0.125);
    assert!(!animation.looping);
    assert_eq!(animation.color, Vec4::new(0.25, 0.5, 0.75, 1.0));
    assert_eq!(animation.time, 0.5);

    let sprite: SpriteRenderer = component(&reloaded, root);
    assert_eq!(sprite.color, Vec4::new(1.0, 0.0, 0.0, 1.0));
    assert_eq!(sprite.texture_path, "sprites/player.png");
    assert_eq!(sprite.tiling_factor, 2.0);
    assert!(sprite.use_sub_texture);
    assert_eq!(sprite.sub_texture_coords, Vec2::new(1.0, 2.0));
    assert_eq!(sprite.sub_texture_cell_size, Vec2::new(8.0, 8.0));
    assert_eq!(sprite.sub_texture_sprite_size, Vec2::new(2.0, 2.0));

    let text: Text = component(&reloaded, root);
    assert_eq!(text.text, "Hello");
    assert_eq!(text.font_path, "fonts/inter.ttf");
    assert_eq!(text.color, Vec4::new(0.5, 0.5, 0.5, 1.0));
    assert_eq!(text.kerning, 0.5);
    assert_eq!(text.line_spacing, 1.5);
    assert!(text.font.is_none());
}

#[test]
fn the_file_uses_the_cpp_component_and_field_names() {
    let yaml = to_yaml(&every_component_scene()).expect("serializes");
    for name in [
        "Scene: Round Trip",
        "Entities:",
        "Entity:",
        "TagComponent:",
        "Tag: Root",
        "TransformComponent:",
        "Translation:",
        "Rotation:",
        "Scale:",
        "CameraComponent:",
        "ProjectionType: 0",
        "PerspectiveFOV:",
        "PerspectiveNear:",
        "PerspectiveFar:",
        "OrthographicSize:",
        "OrthographicNear:",
        "OrthographicFar:",
        "Primary: true",
        "FixedAspectRatio: true",
        "MeshRendererComponent:",
        "MeshPath:",
        "SpriteRendererComponent:",
        "TexturePath:",
        "TilingFactor:",
        "UseSubTexture:",
        "SubTextureCoords:",
        "TextComponent:",
        "TextString: Hello",
        "FontAssetPath:",
        "Kerning:",
        "LineSpacing:",
        "SpotLightComponent:",
        "InnerConeAngle:",
        "OuterConeAngle:",
        "RigidBody3DComponent:",
        "BodyType: 2",
        "LinearDamping:",
        "CapsuleCollider3DComponent:",
        "HalfHeight:",
        "AudioSourceComponent:",
        "ClipPath:",
        "AnimatorComponent:",
        "CurrentAnimationName: walk",
        "PlaybackSpeed:",
        "IsPlaying: true",
        "LoopMode: 0",
        "SpriteAnimationComponent:",
        "SpriteSheetPath:",
        "FrameCount: 8",
        "RelationshipComponent:",
        "Parent:",
        "Children:",
    ] {
        assert!(yaml.contains(name), "`{name}` missing from\n{yaml}");
    }
}

#[test]
fn a_file_that_is_not_a_scene_is_refused() {
    let error = from_yaml("NotAScene: 1\n").unwrap_err();
    assert!(error.to_string().contains("Scene"), "{error}");
}

#[test]
fn a_malformed_file_names_the_line_it_choked_on() {
    // An unterminated flow sequence: a YAML syntax error, so libyaml knows the
    // line.
    let error = from_yaml("Scene: [Untitled\nEntities: []\n").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("line"), "{message}");
}

#[test]
fn an_unknown_component_is_skipped_not_fatal() {
    let yaml = "\
Scene: Untitled
Entities:
  - Entity: 7
    TagComponent:
      Tag: Thing
    TransformComponent:
      Translation: [1, 2, 3]
      Rotation: [0, 0, 0]
      Scale: [1, 1, 1]
    TeleporterComponent:
      Speed: 3
    Model3DComponent:
      ModelPath: models/cube.glb
";
    let scene = from_yaml(yaml).expect("loads despite the unknown components");
    assert_eq!(scene.entities().len(), 1);

    let entity = scene.entities()[0];
    assert_eq!(scene.get(entity).unwrap().tag().as_deref(), Some("Thing"));
    let transform: Transform = component(&scene, entity);
    assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(scene.id_of(entity), Some(7));
}

#[test]
fn duplicate_entity_ids_do_not_refuse_the_file() {
    // Every entity in the shipped `cube.unhuman` carries the same id.
    let yaml = "\
Scene: Untitled
Entities:
  - Entity: 42
    TagComponent:
      Tag: First
  - Entity: 42
    TagComponent:
      Tag: Second
";
    let scene = from_yaml(yaml).expect("loads");
    assert_eq!(scene.entities().len(), 2);
    let first = entity_named(&scene, "First");
    let second = entity_named(&scene, "Second");
    assert_ne!(first, second);
    assert_eq!(scene.id_of(first), Some(42));
    assert_ne!(scene.id_of(second), Some(42));
}

#[test]
fn a_file_with_no_entities_loads_empty() {
    let scene = from_yaml("Scene: Empty\n").expect("loads");
    assert_eq!(scene.name(), "Empty");
    assert!(scene.entities().is_empty());
}

#[test]
fn load_and_save_go_through_the_filesystem() {
    let scene = every_component_scene();
    let path = std::env::temp_dir().join("meats-round-trip.unhuman");
    meats::save(&scene, &path).expect("saves");
    let reloaded = load(&path).expect("loads");
    assert_eq!(reloaded.entities().len(), 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn loads_the_shipped_cpp_cube_scene() {
    // `assets/scenes/cube.unhuman` is `UNHUMAN/UHE_EDITOR/assets/scene/cube.unhuman`
    // verbatim (byte-identical), copied in so the test does not need the C++
    // checkout beside the workspace.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/scenes/cube.unhuman");
    let scene = load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));

    // Seven sprite entities, one of them the primary camera.
    assert_eq!(scene.entities().len(), 7);

    let camera = entity_named(&scene, "Camera");
    let transform: Transform = component(&scene, camera);
    assert!((transform.translation - Vec3::new(-1.44, 0.73, 1.99)).length() < 1e-5);
    let camera_component: Camera = component(&scene, camera);
    assert!(camera_component.primary);
    assert_eq!(
        camera_component.camera.projection_type(),
        ProjectionType::Perspective
    );
    assert!((camera_component.camera.fov() - std::f32::consts::FRAC_PI_4).abs() < 1e-6);

    let bottom = entity_named(&scene, "Bottom");
    let transform: Transform = component(&scene, bottom);
    assert!((transform.translation - Vec3::new(0.0, -0.5, 0.0)).length() < 1e-6);
    let sprite: SpriteRenderer = component(&scene, bottom);
    assert_eq!(sprite.color, Vec4::new(0.0, 1.0, 1.0, 1.0));

    // `cube.unhuman` repeats id 2323232 on every entity, so the loader keeps it
    // on the first and hands out fresh ones for the rest — the file has to stay
    // loadable, and the entities have to stay distinct.
    let ids: Vec<u64> = scene
        .entities()
        .iter()
        .filter_map(|e| scene.id_of(*e))
        .collect();
    assert_eq!(ids.len(), 7);
    assert_eq!(ids.iter().filter(|id| **id == 2323232).count(), 1);
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        7
    );
    assert_eq!(scene.primary_camera(), Some(camera));
}
