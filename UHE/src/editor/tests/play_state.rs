//! Play, pause, step, and the snapshot that makes stopping safe.

use glam::Vec3;
use meats::{Entity, Scene, Transform};
use mirror::{Editor, PlayStatus};

fn translation(scene: &Scene, entity: Entity) -> Vec3 {
    scene
        .get(entity)
        .and_then(|entry| entry.get::<Transform>().map(|transform| transform.translation))
        .unwrap_or(Vec3::ZERO)
}

fn move_to(scene: &mut Scene, entity: Entity, translation: Vec3) {
    if let Some(mut entry) = scene.get_mut(entity) {
        if let Some(transform) = entry.get_mut::<Transform>() {
            transform.translation = translation;
        }
    }
}

#[test]
fn play_takes_a_snapshot_and_stop_puts_the_authored_scene_back() {
    let mut scene = Scene::new();
    let player = scene.spawn("Player");
    let mut editor = Editor::new();

    assert_eq!(editor.play.status, PlayStatus::Stopped);
    assert!(!editor.play.has_snapshot());

    editor.play(&mut scene);
    assert!(editor.play.is_playing());
    assert!(editor.play.has_snapshot());

    // What the running scene does is not authored content.
    move_to(&mut scene, player, Vec3::new(0.0, 9.0, 0.0));
    // Playing again while playing is not a second snapshot.
    editor.play(&mut scene);

    editor.stop(&mut scene);
    assert_eq!(editor.play.status, PlayStatus::Stopped);
    assert!(!editor.play.has_snapshot());
    assert_eq!(translation(&scene, player), Vec3::ZERO);
}

#[test]
fn pause_keeps_the_scene_and_stop_still_discards_it() {
    let mut scene = Scene::new();
    let player = scene.spawn("Player");
    let mut editor = Editor::new();

    editor.play(&mut scene);
    move_to(&mut scene, player, Vec3::new(1.0, 0.0, 0.0));
    editor.pause();

    assert!(editor.play.is_paused());
    assert_eq!(editor.tick(&mut scene, 1.0), 0.0);
    assert_eq!(translation(&scene, player), Vec3::new(1.0, 0.0, 0.0));

    editor.stop(&mut scene);
    assert_eq!(translation(&scene, player), Vec3::ZERO);
}

#[test]
fn an_edit_made_while_playing_is_discarded_on_stop() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    let player = scene.spawn("Player");
    editor.select(Some(player));

    let translation_property = editor
        .properties(&scene)
        .into_iter()
        .find(|property| property.component == "Transform" && property.name == "Translation")
        .expect("a Translation property");

    editor.play(&mut scene);
    editor.set_property(
        &mut scene,
        &translation_property,
        mirror::PropertyValue::Vec3([4.0, 0.0, 0.0]),
    );
    assert_eq!(editor.tick(&mut scene, 1.0 / 60.0), 1.0 / 60.0);
    assert_eq!(translation(&scene, player), Vec3::new(4.0, 0.0, 0.0));

    editor.stop(&mut scene);
    assert_eq!(translation(&scene, player), Vec3::ZERO);
}

#[test]
fn step_advances_exactly_one_fixed_step() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();

    editor.play(&mut scene);
    editor.pause();

    // Nothing happens until a step is asked for.
    assert_eq!(editor.tick(&mut scene, 1.0), 0.0);
    editor.step();
    assert_eq!(editor.tick(&mut scene, 0.0), editor.play.fixed_dt);
    assert_eq!(editor.tick(&mut scene, 0.0), 0.0);

    // Stepping while stopped is not a thing either.
    editor.stop(&mut scene);
    editor.step();
    assert_eq!(editor.tick(&mut scene, 1.0), 0.0);
    assert!(editor.play.is_stopped());
}

#[test]
fn playing_accumulates_whole_fixed_steps() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    editor.play(&mut scene);
    let step = editor.play.fixed_dt;

    assert_eq!(editor.tick(&mut scene, step), step);
    // Half a step is not a step, and is not thrown away.
    assert_eq!(editor.tick(&mut scene, step * 0.5), 0.0);
    assert_eq!(editor.tick(&mut scene, step), step);
    // A long frame runs as many steps as it is worth.
    assert_eq!(editor.tick(&mut scene, step * 3.0), step * 3.0);
}

#[test]
fn stopping_without_playing_does_nothing() {
    let mut scene = Scene::new();
    let player = scene.spawn("Player");
    let mut editor = Editor::new();

    editor.stop(&mut scene);
    assert!(editor.play.is_stopped());
    assert_eq!(translation(&scene, player), Vec3::ZERO);

    // Opening a scene during play drops the snapshot rather than restoring it
    // over the scene that was just loaded.
    editor.play(&mut scene);
    assert!(editor.play.has_snapshot());
    editor.new_scene(&mut scene);
    assert!(editor.play.is_stopped());
    assert!(!editor.play.has_snapshot());
    assert!(scene.entities().is_empty());
}
