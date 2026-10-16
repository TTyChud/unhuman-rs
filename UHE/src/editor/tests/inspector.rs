//! The property panel: a list per component kind, and edits that reach the
//! component and the dirty set.

use glam::{Vec2, Vec3, Vec4};
use meats::{
    Animator, AudioSource, Camera, Collider, ColliderShape, Entity, Light, LightKind, MeshRenderer,
    RigidBody, RigidBodyKind, Scene, SceneCamera, SpriteRenderer, Text,
};
use mirror::inspector::{Inspector, PropertyKind, PropertyValue};
use mirror::Property;

/// `EntityRefMut::insert` without naming the borrow.
fn add<T: Send + Sync + 'static>(scene: &mut Scene, entity: Entity, component: T) {
    if let Some(mut entry) = scene.get_mut(entity) {
        entry.insert(component);
    }
}

/// One entity wearing every component the panel knows about.
fn everything() -> (Scene, Entity) {
    let mut scene = Scene::new();
    let entity = scene.spawn("Everything");
    add(
        &mut scene,
        entity,
        Camera {
            camera: SceneCamera::default(),
            primary: true,
            fixed_aspect_ratio: false,
        },
    );
    add(
        &mut scene,
        entity,
        MeshRenderer {
            mesh: None,
            mesh_path: String::new(),
            material: None,
            material_path: String::new(),
            tint: Vec4::ONE,
            visible: true,
            cast_shadows: false,
        },
    );
    add(
        &mut scene,
        entity,
        SpriteRenderer {
            color: Vec4::ONE,
            texture: None,
            texture_path: String::new(),
            tiling_factor: 1.0,
            use_sub_texture: false,
            sub_texture_coords: Vec2::ZERO,
            sub_texture_cell_size: Vec2::ONE,
            sub_texture_sprite_size: Vec2::ONE,
        },
    );
    add(
        &mut scene,
        entity,
        Text {
            text: "Text Node".into(),
            font: None,
            font_path: String::new(),
            color: Vec4::ONE,
            kerning: 0.0,
            line_spacing: 0.0,
        },
    );
    add(
        &mut scene,
        entity,
        Light {
            kind: LightKind::Directional,
            color: Vec3::ONE,
            intensity: 1.0,
            radius: 10.0,
            inner_cone_angle: 15.0,
            outer_cone_angle: 30.0,
            enabled: true,
        },
    );
    add(
        &mut scene,
        entity,
        RigidBody {
            kind: RigidBodyKind::Static,
            mass: 1.0,
            velocity: Vec3::ZERO,
            damping: 0.05,
            gravity_scale: 1.0,
        },
    );
    add(
        &mut scene,
        entity,
        Collider {
            shape: ColliderShape::Box {
                half_extent: Vec3::splat(0.5),
            },
            offset: Vec3::ZERO,
            friction: 0.2,
            restitution: 0.0,
        },
    );
    add(
        &mut scene,
        entity,
        AudioSource {
            clip: None,
            clip_path: String::new(),
            playing: false,
            looping: false,
            spatial: false,
            gain: 1.0,
            pitch: 1.0,
            range: 10.0,
        },
    );
    add(
        &mut scene,
        entity,
        Animator {
            clip: None,
            clip_path: String::new(),
            time: 0.0,
            speed: 1.0,
            looping: false,
            playing: false,
        },
    );
    (scene, entity)
}

fn find(
    inspector: &Inspector,
    scene: &Scene,
    entity: Entity,
    component: &str,
    name: &str,
) -> Property {
    inspector
        .properties(scene, entity)
        .into_iter()
        .find(|property| property.component == component && property.name == name)
        .unwrap_or_else(|| panic!("no {component}.{name}"))
}

fn float_of(value: &PropertyValue) -> f32 {
    match value {
        PropertyValue::Float(value) => *value,
        other => panic!("expected a float, got {other:?}"),
    }
}

/// Angles go through radians and back, so an exact comparison is the wrong
/// question to ask of them.
fn same(a: &PropertyValue, b: &PropertyValue) -> bool {
    match (a, b) {
        (PropertyValue::Float(a), PropertyValue::Float(b)) => {
            (a - b).abs() <= 1e-4 * a.abs().max(1.0)
        }
        _ => a == b,
    }
}

#[test]
fn every_component_kind_contributes_a_property_list() {
    let (scene, entity) = everything();
    let inspector = Inspector::new();
    let properties = inspector.properties(&scene, entity);

    for component in [
        "Tag",
        "Transform",
        "Camera",
        "MeshRenderer",
        "SpriteRenderer",
        "Text",
        "Light",
        "RigidBody",
        "Collider",
        "AudioSource",
        "Animator",
    ] {
        let found = properties
            .iter()
            .filter(|property| property.component == component)
            .count();
        assert!(found > 0, "no {component} properties");
    }

    let names: Vec<&str> = properties.iter().map(|property| property.name).collect();
    for expected in [
        "Tag",
        "Translation",
        "Rotation",
        "Scale",
        "FOV",
        "Projection",
        "Body Type",
        "Shape",
        "Half Height",
        "Clip Path",
        "Mesh Path",
        "SubTexture Coords",
    ] {
        assert!(names.contains(&expected), "no {expected} property");
    }

    let kinds: Vec<PropertyKind> = properties.iter().map(|property| property.kind).collect();
    for expected in [
        PropertyKind::Bool,
        PropertyKind::Float,
        PropertyKind::Vec2,
        PropertyKind::Vec3,
        PropertyKind::Color,
        PropertyKind::Text,
    ] {
        assert!(kinds.contains(&expected), "no {expected:?} property");
    }
    assert!(kinds
        .iter()
        .any(|kind| matches!(kind, PropertyKind::Enum(_))));
}

#[test]
fn a_bare_entity_still_has_its_tag_and_transform() {
    let mut scene = Scene::new();
    let lonely = scene.spawn("Lonely");
    let properties = Inspector::new().properties(&scene, lonely);
    let components: Vec<&str> = properties
        .iter()
        .map(|property| property.component)
        .collect();
    assert_eq!(
        components,
        vec!["Tag", "Transform", "Transform", "Transform"]
    );
}

#[test]
fn setting_a_property_changes_the_component_and_marks_it_dirty() {
    let (mut scene, entity) = everything();
    let mut inspector = Inspector::new();

    let translation = find(&inspector, &scene, entity, "Transform", "Translation");
    assert_eq!(
        translation.value(&scene, entity),
        PropertyValue::Vec3([0.0, 0.0, 0.0])
    );

    inspector.set(
        &mut scene,
        entity,
        &translation,
        PropertyValue::Vec3([1.0, 2.0, 3.0]),
    );

    assert_eq!(
        translation.value(&scene, entity),
        PropertyValue::Vec3([1.0, 2.0, 3.0])
    );
    assert!(inspector.is_dirty(entity));
    assert_eq!(inspector.take_dirty(), vec![entity]);
    assert!(!inspector.is_dirty(entity));
}

#[test]
fn every_property_reads_back_the_value_it_was_given() {
    let (mut scene, entity) = everything();
    let mut inspector = Inspector::new();

    for property in inspector.properties(&scene, entity) {
        let before = property.value(&scene, entity);
        inspector.set(&mut scene, entity, &property, before.clone());
        let after = find(
            &inspector,
            &scene,
            entity,
            property.component,
            property.name,
        )
        .value(&scene, entity);
        assert!(
            same(&before, &after),
            "{}.{}: {before:?} became {after:?}",
            property.component,
            property.name
        );
    }
}

#[test]
fn an_edit_named_by_component_and_field_is_what_undo_replays() {
    let (mut scene, entity) = everything();
    let mut inspector = Inspector::new();

    let tag = find(&inspector, &scene, entity, "Tag", "Tag");
    inspector.set(&mut scene, entity, &tag, PropertyValue::Text("Hero".into()));
    assert_eq!(scene.get(entity).unwrap().tag().as_deref(), Some("Hero"));

    assert!(inspector.set_named(
        &mut scene,
        entity,
        "Tag",
        "Tag",
        &PropertyValue::Text("Villain".into())
    ));
    assert_eq!(scene.get(entity).unwrap().tag().as_deref(), Some("Villain"));

    // A name no component has is refused rather than silently swallowed.
    assert!(!inspector.set_named(
        &mut scene,
        entity,
        "Nope",
        "Nothing",
        &PropertyValue::Float(1.0)
    ));
    assert!(!inspector.set_named(
        &mut scene,
        entity,
        "Light",
        "Nope",
        &PropertyValue::Float(1.0)
    ));
}

#[test]
fn a_handle_the_scene_does_not_know_is_a_no_op() {
    let (mut scene, entity) = everything();
    let mut inspector = Inspector::new();
    let tag = find(&inspector, &scene, entity, "Tag", "Tag");

    let ghost = scene.spawn("Gone");
    scene.destroy(ghost);

    inspector.set(
        &mut scene,
        ghost,
        &tag,
        PropertyValue::Text("Nobody".into()),
    );
    assert!(inspector.properties(&scene, ghost).is_empty());
    assert_eq!(
        scene.get(entity).unwrap().tag().as_deref(),
        Some("Everything")
    );
}

#[test]
fn component_values_survive_the_round_trip() {
    let (mut scene, entity) = everything();
    let mut inspector = Inspector::new();

    let scale = find(&inspector, &scene, entity, "Transform", "Scale");
    inspector.set(
        &mut scene,
        entity,
        &scale,
        PropertyValue::Vec3([2.0, 2.0, 2.0]),
    );
    assert_eq!(
        scale.value(&scene, entity),
        PropertyValue::Vec3([2.0, 2.0, 2.0])
    );

    let fov = find(&inspector, &scene, entity, "Camera", "FOV");
    inspector.set(&mut scene, entity, &fov, PropertyValue::Float(60.0));
    assert!((float_of(&fov.value(&scene, entity)) - 60.0).abs() < 1e-3);

    let projection = find(&inspector, &scene, entity, "Camera", "Projection");
    inspector.set(&mut scene, entity, &projection, PropertyValue::Int(1));
    assert_eq!(projection.value(&scene, entity), PropertyValue::Int(1));

    let cone = find(&inspector, &scene, entity, "Light", "Outer Cone");
    inspector.set(&mut scene, entity, &cone, PropertyValue::Float(40.0));
    assert_eq!(cone.value(&scene, entity), PropertyValue::Float(40.0));

    let shape = find(&inspector, &scene, entity, "Collider", "Shape");
    inspector.set(&mut scene, entity, &shape, PropertyValue::Int(2));
    let radius = find(&inspector, &scene, entity, "Collider", "Radius");
    assert_eq!(radius.value(&scene, entity), PropertyValue::Float(0.5));
    let half_height = find(&inspector, &scene, entity, "Collider", "Half Height");
    assert_eq!(half_height.value(&scene, entity), PropertyValue::Float(1.0));

    let tint = find(&inspector, &scene, entity, "MeshRenderer", "Tint");
    inspector.set(
        &mut scene,
        entity,
        &tint,
        PropertyValue::Color([1.0, 0.0, 0.0, 1.0]),
    );
    assert_eq!(
        tint.value(&scene, entity),
        PropertyValue::Color([1.0, 0.0, 0.0, 1.0])
    );

    let coords = find(
        &inspector,
        &scene,
        entity,
        "SpriteRenderer",
        "SubTexture Coords",
    );
    inspector.set(&mut scene, entity, &coords, PropertyValue::Vec2([1.0, 2.0]));
    assert_eq!(
        coords.value(&scene, entity),
        PropertyValue::Vec2([1.0, 2.0])
    );

    // Values the panel never wrote stay where the component's default put them.
    let colour = find(&inspector, &scene, entity, "Text", "Color");
    assert_eq!(
        colour.value(&scene, entity),
        PropertyValue::Color([1.0, 1.0, 1.0, 1.0])
    );
    let offset = find(&inspector, &scene, entity, "Collider", "Offset");
    assert_eq!(
        offset.value(&scene, entity),
        PropertyValue::Vec3([0.0, 0.0, 0.0])
    );
    let velocity = find(&inspector, &scene, entity, "RigidBody", "Velocity");
    assert_eq!(
        velocity.value(&scene, entity),
        PropertyValue::Vec3([0.0, 0.0, 0.0])
    );
    let clip = find(&inspector, &scene, entity, "Animator", "Clip Path");
    assert_eq!(
        clip.value(&scene, entity),
        PropertyValue::Text(String::new())
    );
    let gain = find(&inspector, &scene, entity, "AudioSource", "Gain");
    assert!(matches!(
        gain.value(&scene, entity),
        PropertyValue::Float(_)
    ));
}
