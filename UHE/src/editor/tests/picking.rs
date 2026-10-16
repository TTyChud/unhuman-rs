//! Picking: a cursor becomes a ray, and a ray picks the nearest mesh.

use std::path::{Path, PathBuf};

use glam::{Mat4, Vec2, Vec3};
use meats::{Entity, MeshRenderer, Scene, Transform};
use mirror::gizmo::Camera;
use mirror::{Editor, Ray};
use vault::{AssetId, Assets};

const VIEWPORT: (u32, u32) = (800, 600);

/// The bounded triangle the fixture loads: positions 0..1 in x and y at z = 0.
const TRIANGLE_GLTF: &str = r#"{
  "asset": { "version": "2.0" },
  "buffers": [ { "byteLength": 36, "uri": "tri.bin" } ],
  "bufferViews": [ { "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 } ],
  "accessors": [ { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                   "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] } ],
  "meshes": [ { "name": "Tri", "primitives": [ { "attributes": { "POSITION": 0 }, "mode": 4 } ] } ],
  "nodes": [ { "mesh": 0 } ],
  "scenes": [ { "nodes": [0] } ],
  "scene": 0
}"#;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mirror_picking_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("scratch directory");
    root
}

/// A three-vertex mesh on disk, loaded through the real loader.
fn triangle_assets(name: &str) -> (Assets, PathBuf) {
    let root = scratch(name);
    let mut bin = Vec::new();
    for vertex in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for channel in vertex {
            bin.extend_from_slice(&channel.to_le_bytes());
        }
    }
    std::fs::write(root.join("tri.bin"), &bin).expect("vertex buffer");
    std::fs::write(root.join("tri.gltf"), TRIANGLE_GLTF).expect("gltf");
    (Assets::new(&root), root)
}

/// A second copy on disk, so it gets its own registry slot.
fn second_mesh(assets: &mut Assets, root: &Path) -> AssetId {
    std::fs::write(root.join("other.gltf"), TRIANGLE_GLTF).expect("gltf");
    assets
        .load("other.gltf")
        .expect("the second triangle loads")
}

fn view() -> Mat4 {
    glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y)
}

fn projection() -> Mat4 {
    glam::camera::rh::proj::directx::perspective(60f32.to_radians(), 800.0 / 600.0, 0.1, 100.0)
}

fn view_projection() -> Mat4 {
    projection() * view()
}

/// Where a world point lands on screen, through the same matrices the pick
/// unprojects with.
fn cursor_at(world: Vec3) -> (f32, f32) {
    let camera = Camera {
        view: view(),
        projection: projection(),
        viewport_size: Vec2::new(VIEWPORT.0 as f32, VIEWPORT.1 as f32),
    };
    let screen = camera.project(world).expect("the point is in front");
    (screen.x, screen.y)
}

fn add_mesh(scene: &mut Scene, entity: Entity, mesh: AssetId) {
    if let Some(mut entry) = scene.get_mut(entity) {
        entry.insert(MeshRenderer {
            mesh: Some(mesh),
            visible: true,
            ..Default::default()
        });
    }
}

fn move_to(scene: &mut Scene, entity: Entity, translation: Vec3) {
    if let Some(mut entry) = scene.get_mut(entity) {
        if let Some(transform) = entry.get_mut::<Transform>() {
            transform.translation = translation;
        }
    }
}

fn ray_at(world: Vec3) -> Ray {
    Ray::through_cursor(cursor_at(world), VIEWPORT, view_projection())
        .expect("a ray through the cursor")
}

/// The triangle spans x and y in 0..1 at z = 0, so this is a point inside it.
const INSIDE: Vec3 = Vec3::new(0.4, 0.4, 0.0);

#[test]
fn the_centre_ray_points_down_the_camera_forward_axis() {
    let ray = Ray::through_cursor((400.0, 300.0), VIEWPORT, view_projection()).unwrap();
    assert!((ray.direction.normalize() - Vec3::NEG_Z).length() < 1e-4);
    assert!(ray.direction.is_finite());
    // Depth 0 is the near plane, a tenth of a unit in front of the eye.
    assert!((ray.origin - Vec3::new(0.0, 0.0, 4.9)).length() < 1e-3);
}

#[test]
fn a_degenerate_view_projection_gives_no_ray() {
    assert!(Ray::through_cursor((400.0, 300.0), VIEWPORT, Mat4::ZERO).is_none());
    let collapsed = Mat4::from_scale(Vec3::new(0.0, 1.0, 1.0)) * view_projection();
    assert!(Ray::through_cursor((400.0, 300.0), VIEWPORT, collapsed).is_none());
    // And a cursor outside the viewport never becomes a ray at all.
    assert!(Ray::through_cursor((-4.0, 300.0), VIEWPORT, view_projection()).is_none());
}

#[test]
fn a_ray_at_a_known_entity_hits_it() {
    let (mut assets, _) = triangle_assets("hit");
    let mesh = assets.load("tri.gltf").expect("the triangle loads");
    assert!(
        assets
            .mesh(mesh)
            .is_some_and(|data| !data.positions.is_empty()),
        "the loader produced no geometry"
    );

    let mut scene = Scene::new();
    let entity = scene.spawn("Tri");
    add_mesh(&mut scene, entity, mesh);
    scene.update_transforms();

    let mut editor = Editor::new();
    assert_eq!(editor.pick(&scene, &assets, ray_at(INSIDE)), Some(entity));
    assert_eq!(editor.hovered(), Some(entity));
    assert_eq!(editor.selected_entity(), Some(entity));
}

#[test]
fn a_ray_beside_the_entity_misses_and_clears_the_selection() {
    let (mut assets, _) = triangle_assets("miss");
    let mesh = assets.load("tri.gltf").expect("the triangle loads");

    let mut scene = Scene::new();
    let entity = scene.spawn("Tri");
    add_mesh(&mut scene, entity, mesh);
    scene.update_transforms();

    let mut editor = Editor::new();
    editor.select(Some(entity));

    // Two metres to the left of the mesh, still inside the viewport.
    let ray = ray_at(Vec3::new(-2.0, 0.5, 0.0));
    assert_eq!(editor.pick(&scene, &assets, ray), None);
    assert_eq!(editor.hovered(), None);
    assert_eq!(editor.selected_entity(), None);
}

#[test]
fn the_nearer_of_two_overlapping_entities_wins() {
    let (mut assets, _) = triangle_assets("nearer");
    let mesh = assets.load("tri.gltf").expect("the triangle loads");

    let mut scene = Scene::new();
    let far = scene.spawn("Far");
    let near = scene.spawn("Near");
    add_mesh(&mut scene, far, mesh);
    add_mesh(&mut scene, near, mesh);
    // Both cover the same pixels; one is two metres closer to the camera.
    move_to(&mut scene, far, Vec3::new(0.0, 0.0, -2.0));
    move_to(&mut scene, near, Vec3::ZERO);
    scene.update_transforms();

    let ray = ray_at(INSIDE);
    let mut editor = Editor::new();
    assert_eq!(editor.pick(&scene, &assets, ray), Some(near));

    // Swapped over, the same ray picks the other one: nearest by parameter,
    // never by the order the scene happens to hand entities out.
    move_to(&mut scene, near, Vec3::new(0.0, 0.0, -2.0));
    move_to(&mut scene, far, Vec3::ZERO);
    scene.update_transforms();
    assert_eq!(editor.pick(&scene, &assets, ray), Some(far));
}

#[test]
fn an_unresolved_handle_is_skipped_rather_than_refused() {
    let (mut assets, root) = triangle_assets("skipped");
    let solid_mesh = assets.load("tri.gltf").expect("the triangle loads");
    let broken_mesh = second_mesh(&mut assets, &root);
    assert!(assets.release(broken_mesh), "the slot frees");
    assert!(assets.mesh(broken_mesh).is_none());

    let mut scene = Scene::new();
    let broken = scene.spawn("Broken");
    let solid = scene.spawn("Solid");
    add_mesh(&mut scene, broken, broken_mesh);
    add_mesh(&mut scene, solid, solid_mesh);
    // The broken entity sits in front of the solid one and must not win.
    move_to(&mut scene, broken, Vec3::new(0.0, 0.0, 1.0));
    scene.update_transforms();

    let ray = ray_at(INSIDE);
    let mut editor = Editor::new();
    assert_eq!(editor.pick(&scene, &assets, ray), Some(solid));

    // A mesh handle that was never set is skipped too.
    let unset = scene.spawn("Unset");
    {
        let mut entry = scene.get_mut(unset).unwrap();
        entry.insert(MeshRenderer {
            mesh: None,
            visible: true,
            ..Default::default()
        });
    }
    move_to(&mut scene, unset, Vec3::new(0.0, 0.0, 2.0));
    scene.update_transforms();
    assert_eq!(editor.pick(&scene, &assets, ray), Some(solid));
}
