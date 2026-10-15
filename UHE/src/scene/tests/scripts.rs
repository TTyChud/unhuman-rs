//! The `NativeScript` lifecycle: `on_create` once, then `on_update`, and
//! `on_destroy` exactly once however the entity goes away.

use std::sync::{Arc, Mutex, PoisonError};

use meats::{Entity, NativeScript, Scene, Script, Transform};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn push(&self, entry: impl Into<String>) {
        // A panicking hook poisons the mutex; the entries recorded before it are
        // still what the test wants to see.
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry.into());
    }

    fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Logs each hook, and reaches back into the scene through the `&mut Scene` it
/// is handed — which is the whole point of the shape.
struct TestScript {
    recorder: Recorder,
    ticks: u32,
}

impl Script for TestScript {
    fn on_create(&mut self, entity: Entity, scene: &mut Scene) {
        let tag = scene
            .get(entity)
            .and_then(|view| view.tag())
            .unwrap_or_default();
        self.recorder.push(format!("create:{tag}"));
    }

    fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: f32) {
        self.ticks += 1;
        let x = scene
            .get(entity)
            .and_then(|view| view.get::<Transform>().map(|transform| transform.translation.x))
            .unwrap_or_default();
        self.recorder
            .push(format!("update:{}:{dt}:{x}", self.ticks));
    }

    fn on_destroy(&mut self, _entity: Entity, _scene: &mut Scene) {
        self.recorder.push("destroy");
    }
}

#[test]
fn the_three_hooks_fire_once_each_in_order() {
    let recorder = Recorder::default();
    let mut scene = Scene::new();
    let entity = scene.spawn("Scripted");
    scene.insert(
        entity,
        NativeScript::new(TestScript {
            recorder: recorder.clone(),
            ticks: 0,
        }),
    );

    scene.update_scripts(0.5);
    scene.destroy(entity);

    assert_eq!(
        recorder.entries(),
        vec![
            "create:Scripted".to_string(),
            "update:1:0.5:0".to_string(),
            "destroy".to_string(),
        ]
    );
}

#[test]
fn on_create_fires_once_across_many_updates() {
    let recorder = Recorder::default();
    let mut scene = Scene::new();
    let entity = scene.spawn("Scripted");
    scene.insert(
        entity,
        NativeScript::new(TestScript {
            recorder: recorder.clone(),
            ticks: 0,
        }),
    );

    scene.update_scripts(0.1);
    scene.update_scripts(0.1);
    scene.update_scripts(0.1);

    let entries = recorder.entries();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0], "create:Scripted");
    assert_eq!(entries[1], "update:1:0.1:0");
    assert_eq!(entries[2], "update:2:0.1:0");
    assert_eq!(entries[3], "update:3:0.1:0");
}

#[test]
fn on_destroy_fires_when_the_component_is_removed() {
    let recorder = Recorder::default();
    let mut scene = Scene::new();
    let entity = scene.spawn("Scripted");
    scene.insert(
        entity,
        NativeScript::new(TestScript {
            recorder: recorder.clone(),
            ticks: 0,
        }),
    );
    scene.update_scripts(0.1);

    let _ = scene.remove::<NativeScript>(entity);
    assert_eq!(recorder.entries().last().unwrap(), "destroy");
    assert!(scene.contains(entity), "only the component went");
    scene.update_scripts(0.1);
    assert_eq!(recorder.entries().len(), 2);
}

#[test]
fn on_destroy_fires_when_the_scene_is_cleared() {
    let recorder = Recorder::default();
    let mut scene = Scene::new();
    let entity = scene.spawn("Scripted");
    scene.insert(
        entity,
        NativeScript::new(TestScript {
            recorder: recorder.clone(),
            ticks: 0,
        }),
    );
    scene.update_scripts(0.1);

    scene.clear();
    assert!(scene.entities().is_empty());
    assert!(recorder.entries().contains(&"destroy".to_string()));
}

#[test]
fn on_destroy_fires_when_the_scene_is_dropped() {
    let recorder = Recorder::default();
    {
        let mut scene = Scene::new();
        let entity = scene.spawn("Scripted");
        scene.insert(
            entity,
            NativeScript::new(TestScript {
                recorder: recorder.clone(),
                ticks: 0,
            }),
        );
    }

    // `on_create` never ran — the scene was dropped before an update.
    assert_eq!(recorder.entries(), vec!["destroy".to_string()]);
}

#[test]
fn a_script_can_add_entities_while_it_runs() {
    struct Spawner {
        recorder: Recorder,
    }

    impl Script for Spawner {
        fn on_create(&mut self, _entity: Entity, _scene: &mut Scene) {}

        fn on_update(&mut self, entity: Entity, scene: &mut Scene, _dt: f32) {
            if scene.children(entity).is_empty() {
                let child = scene.spawn("Child");
                scene.set_parent(child, Some(entity));
                self.recorder.push("spawned");
            }
        }

        fn on_destroy(&mut self, _entity: Entity, _scene: &mut Scene) {}
    }

    let recorder = Recorder::default();
    let mut scene = Scene::new();
    let entity = scene.spawn("Spawner");
    scene.insert(
        entity,
        NativeScript::new(Spawner {
            recorder: recorder.clone(),
        }),
    );

    scene.update_scripts(0.1);
    scene.update_scripts(0.1);

    assert_eq!(recorder.entries(), vec!["spawned".to_string()]);
    assert_eq!(scene.children(entity).len(), 1);
    assert_eq!(scene.entities().len(), 2);
}

#[test]
fn a_script_that_destroys_its_own_entity_still_gets_on_destroy() {
    struct Suicide {
        recorder: Recorder,
    }

    impl Script for Suicide {
        fn on_create(&mut self, _entity: Entity, _scene: &mut Scene) {}

        fn on_update(&mut self, entity: Entity, scene: &mut Scene, _dt: f32) {
            scene.destroy(entity);
            self.recorder.push("destroyed itself");
        }

        fn on_destroy(&mut self, entity: Entity, scene: &mut Scene) {
            self.recorder
                .push(format!("destroy:{}", scene.contains(entity)));
        }
    }

    let recorder = Recorder::default();
    let mut scene = Scene::new();
    let entity = scene.spawn("Suicide");
    scene.insert(
        entity,
        NativeScript::new(Suicide {
            recorder: recorder.clone(),
        }),
    );

    scene.update_scripts(0.1);

    assert_eq!(
        recorder.entries(),
        vec!["destroyed itself".to_string(), "destroy:false".to_string()]
    );
    assert!(scene.entities().is_empty());
}
