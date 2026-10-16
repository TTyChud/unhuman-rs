//! Scene lifetime, the entity handle, and the hierarchy.

use glam::{Mat4, Vec3};
use meats::{Scene, Transform};

#[test]
fn spawn_get_destroy() {
    let mut scene = Scene::new();
    assert!(scene.entities().is_empty());

    let entity = scene.spawn("Player");
    assert_eq!(scene.entities(), vec![entity]);
    assert!(scene.contains(entity));

    let view = scene.get(entity).expect("just spawned");
    assert_eq!(view.entity(), entity);
    assert_eq!(view.tag().as_deref(), Some("Player"));
    assert!(view.has::<Transform>());
    assert!(view.id().is_some());

    scene.destroy(entity);
    assert!(scene.get(entity).is_none());
    assert!(!scene.contains(entity));
    assert!(scene.entities().is_empty());
}

#[test]
fn an_empty_name_becomes_entity() {
    let mut scene = Scene::new();
    let entity = scene.spawn("");
    assert_eq!(scene.get(entity).unwrap().tag().as_deref(), Some("Entity"));
}

#[test]
fn a_destroyed_id_is_refused_even_after_the_slot_is_reused() {
    let mut scene = Scene::new();
    let first = scene.spawn("First");
    scene.destroy(first);

    let second = scene.spawn("Second");
    assert_ne!(first, second, "the generation must have moved on");
    assert!(scene.get(first).is_none());
    assert_eq!(scene.get(second).unwrap().tag().as_deref(), Some("Second"));
    assert_eq!(scene.entities(), vec![second]);
}

#[test]
fn get_mut_writes_components() {
    let mut scene = Scene::new();
    let entity = scene.spawn("Thing");

    let mut view = scene.get_mut(entity).unwrap();
    view.get_mut::<Transform>().unwrap().translation = Vec3::new(3.0, 4.0, 5.0);

    let view = scene.get(entity).unwrap();
    assert_eq!(
        view.get::<Transform>().unwrap().translation,
        Vec3::new(3.0, 4.0, 5.0)
    );
}

#[test]
fn a_child_inherits_its_parents_transform() {
    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.insert(
        parent,
        Transform {
            translation: Vec3::new(1.0, 0.0, 0.0),
            ..Transform::default()
        },
    );
    scene.insert(
        child,
        Transform {
            translation: Vec3::new(0.0, 2.0, 0.0),
            ..Transform::default()
        },
    );

    scene.set_parent(child, Some(parent));
    assert_eq!(scene.parent(child), Some(parent));
    assert_eq!(scene.children(parent).to_vec(), vec![child]);

    let world = scene.world_transform(child);
    assert!((world.transform_point3(Vec3::ZERO) - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-6);
    // The local transform is untouched by a plain `set_parent`.
    assert_eq!(
        scene
            .get(child)
            .unwrap()
            .get::<Transform>()
            .unwrap()
            .translation,
        Vec3::new(0.0, 2.0, 0.0)
    );
}

#[test]
fn world_transform_composes_rotation_and_scale_down_the_chain() {
    let mut scene = Scene::new();
    let grandparent = scene.spawn("Grandparent");
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.insert(
        grandparent,
        Transform {
            translation: Vec3::new(0.0, 1.0, 0.0),
            rotation: Vec3::new(90.0, 0.0, 0.0),
            scale: Vec3::new(2.0, 2.0, 2.0),
        },
    );
    scene.insert(
        parent,
        Transform {
            translation: Vec3::new(0.0, 0.0, 1.0),
            ..Transform::default()
        },
    );
    scene.insert(
        child,
        Transform {
            translation: Vec3::new(0.0, 1.0, 0.0),
            ..Transform::default()
        },
    );
    scene.set_parent(parent, Some(grandparent));
    scene.set_parent(child, Some(parent));

    // child (0,1,0) -> parent space (0,1,1); grandparent rotates +Y to +Z
    // (so +Z to -Y) and scales by 2: (0,2,2) -> (0,-2,2), plus its own
    // translation (0,1,0) = (0,-1,2).
    let world = scene.world_transform(child).transform_point3(Vec3::ZERO);
    assert!(
        (world - Vec3::new(0.0, -1.0, 2.0)).length() < 1e-5,
        "{world:?}"
    );
}

#[test]
fn a_cycle_is_refused() {
    let mut scene = Scene::new();
    let a = scene.spawn("A");
    let b = scene.spawn("B");
    scene.set_parent(b, Some(a));

    scene.set_parent(a, Some(b));
    assert_eq!(scene.parent(a), None, "a is b's parent, so b cannot be a's");
    assert_eq!(scene.parent(b), Some(a));

    scene.set_parent(a, Some(a));
    assert_eq!(scene.parent(a), None);
}

#[test]
fn reparenting_detaches_from_the_old_parent() {
    let mut scene = Scene::new();
    let first = scene.spawn("First");
    let second = scene.spawn("Second");
    let child = scene.spawn("Child");
    scene.set_parent(child, Some(first));
    scene.set_parent(child, Some(second));

    assert!(scene.children(first).is_empty());
    assert_eq!(scene.children(second).to_vec(), vec![child]);
    assert_eq!(scene.parent(child), Some(second));
}

#[test]
fn set_parent_preserving_world_keeps_the_entity_where_it_is() {
    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.insert(
        parent,
        Transform {
            translation: Vec3::new(10.0, 0.0, 0.0),
            ..Transform::default()
        },
    );
    scene.insert(
        child,
        Transform {
            translation: Vec3::new(2.0, 0.0, 0.0),
            ..Transform::default()
        },
    );

    scene.set_parent_preserving_world(child, Some(parent));
    let world = scene.world_transform(child).transform_point3(Vec3::ZERO);
    assert!(
        (world - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5,
        "{world:?}"
    );
    assert_eq!(
        scene
            .get(child)
            .unwrap()
            .get::<Transform>()
            .unwrap()
            .translation,
        Vec3::new(-8.0, 0.0, 0.0)
    );
}

#[test]
fn destroying_a_parent_destroys_its_subtree() {
    let mut scene = Scene::new();
    let root = scene.spawn("Root");
    let child = scene.spawn("Child");
    let grandchild = scene.spawn("Grandchild");
    scene.set_parent(child, Some(root));
    scene.set_parent(grandchild, Some(child));

    scene.destroy(root);
    assert!(scene.entities().is_empty());
    assert!(scene.get(child).is_none());
    assert!(scene.get(grandchild).is_none());
}

#[test]
fn destroying_a_child_leaves_the_parent_alone() {
    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.set_parent(child, Some(parent));

    scene.destroy(child);
    assert_eq!(scene.entities(), vec![parent]);
    assert!(scene.children(parent).is_empty());
}

#[test]
fn update_transforms_is_stable_when_called_twice() {
    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.set_parent(child, Some(parent));
    scene.insert(
        parent,
        Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Vec3::new(0.0, 45.0, 0.0),
            scale: Vec3::new(2.0, 2.0, 2.0),
        },
    );

    let entities = scene.entities();

    // No cache exists yet, so this is the ancestor walk.
    let walked: Vec<Mat4> = entities
        .iter()
        .map(|entity| scene.world_transform(*entity))
        .collect();

    scene.update_transforms();
    let first: Vec<Mat4> = entities
        .iter()
        .map(|entity| scene.world_transform(*entity))
        .collect();
    assert_eq!(
        walked, first,
        "the cache must agree with the walk it replaced"
    );

    scene.update_transforms();
    let second: Vec<Mat4> = entities
        .iter()
        .map(|entity| scene.world_transform(*entity))
        .collect();
    assert_eq!(first, second);
}

#[test]
fn the_transform_cache_is_dropped_when_the_hierarchy_changes() {
    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.insert(
        parent,
        Transform {
            translation: Vec3::new(5.0, 0.0, 0.0),
            ..Transform::default()
        },
    );
    scene.update_transforms();
    assert_eq!(
        scene.world_transform(child).transform_point3(Vec3::ZERO),
        Vec3::ZERO
    );

    scene.set_parent(child, Some(parent));
    assert!(
        (scene.world_transform(child).transform_point3(Vec3::ZERO) - Vec3::new(5.0, 0.0, 0.0))
            .length()
            < 1e-6
    );
}

#[test]
fn primary_camera_finds_the_flagged_camera() {
    use meats::{Camera, SceneCamera};

    let mut scene = Scene::new();
    let plain = scene.spawn("Plain");
    scene.insert(
        plain,
        Camera {
            camera: SceneCamera::new_perspective(1.0, 0.1, 100.0),
            primary: false,
            fixed_aspect_ratio: false,
        },
    );
    assert_eq!(scene.primary_camera(), None);

    let main = scene.spawn("Main");
    scene.insert(main, Camera::default());
    assert_eq!(scene.primary_camera(), Some(main));
}

#[test]
fn the_viewport_reaches_cameras_that_do_not_pin_their_aspect() {
    use meats::Camera;

    let mut scene = Scene::new();
    let free = scene.spawn("Free");
    let pinned = scene.spawn("Pinned");
    scene.insert(
        free,
        Camera {
            fixed_aspect_ratio: false,
            ..Camera::default()
        },
    );
    scene.insert(
        pinned,
        Camera {
            fixed_aspect_ratio: true,
            ..Camera::default()
        },
    );

    scene.set_viewport_size(1920, 1080);
    assert_eq!(scene.viewport_size(), (1920, 1080));
    let aspect = scene
        .get(free)
        .unwrap()
        .get::<Camera>()
        .unwrap()
        .camera
        .aspect_ratio();
    assert!((aspect - 1920.0 / 1080.0).abs() < 1e-6);
    assert_eq!(
        scene
            .get(pinned)
            .unwrap()
            .get::<Camera>()
            .unwrap()
            .camera
            .aspect_ratio(),
        0.0
    );
}

#[test]
fn a_clone_carries_the_hierarchy_and_the_components() {
    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.set_parent(child, Some(parent));
    scene.insert(
        child,
        Transform {
            translation: Vec3::new(1.0, 1.0, 1.0),
            ..Transform::default()
        },
    );

    let copy = scene.clone();
    assert_eq!(copy.entities().len(), 2);
    assert_eq!(copy.name(), scene.name());

    let copy_parent = copy
        .entities()
        .into_iter()
        .find(|entity| copy.get(*entity).unwrap().tag().as_deref() == Some("Parent"))
        .unwrap();
    let copy_child = copy
        .entities()
        .into_iter()
        .find(|entity| copy.get(*entity).unwrap().tag().as_deref() == Some("Child"))
        .unwrap();
    assert_eq!(copy.parent(copy_child), Some(copy_parent));
    assert_eq!(
        copy.get(copy_child)
            .unwrap()
            .get::<Transform>()
            .unwrap()
            .translation,
        Vec3::new(1.0, 1.0, 1.0)
    );
    assert_eq!(
        copy.world_transform(copy_child)
            .transform_point3(Vec3::ZERO),
        Vec3::new(1.0, 1.0, 1.0)
    );
}

#[test]
fn clear_empties_the_scene() {
    let mut scene = Scene::new();
    let parent = scene.spawn("Parent");
    let child = scene.spawn("Child");
    scene.set_parent(child, Some(parent));

    scene.clear();
    assert!(scene.entities().is_empty());
    assert!(scene.get(parent).is_none());
    assert_eq!(scene.children(parent).len(), 0);
}
