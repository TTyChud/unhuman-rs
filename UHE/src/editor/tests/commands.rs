//! Undo and redo: every command, and the bound on the history.

use meats::{Entity, Scene};
use mirror::inspector::{Inspector, PropertyValue};
use mirror::{Command, Editor, History};

fn tag_of(scene: &Scene, entity: Entity) -> String {
    scene
        .get(entity)
        .and_then(|entry| entry.tag())
        .unwrap_or_default()
        .to_string()
}

fn translation_property(editor: &Editor, scene: &Scene) -> mirror::Property {
    editor
        .properties(scene)
        .into_iter()
        .find(|property| property.component == "Transform" && property.name == "Translation")
        .expect("a Transform entity has a Translation")
}

#[test]
fn a_create_is_undone_and_redone() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    let before = scene.entities().len();

    let created = editor.create_entity(&mut scene, None);
    assert_eq!(scene.entities().len(), before + 1);
    assert_eq!(tag_of(&scene, created), "Empty Entity");
    assert_eq!(editor.selected_entity(), Some(created));

    assert!(editor.undo(&mut scene));
    assert_eq!(scene.entities().len(), before);
    assert!(scene.get(created).is_none());

    assert!(editor.redo(&mut scene));
    assert_eq!(scene.entities().len(), before + 1);
}

#[test]
fn a_destroy_is_undone_and_redone() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    let entity = scene.spawn("Doomed");

    editor.destroy_entity(&mut scene, entity);
    assert!(scene.get(entity).is_none());

    assert!(editor.undo(&mut scene));
    assert_eq!(scene.entities().len(), 1);
    let restored = scene.entities()[0];
    assert_eq!(tag_of(&scene, restored), "Doomed");

    assert!(editor.redo(&mut scene));
    assert!(scene.entities().is_empty());
}

#[test]
fn a_rename_is_undone_and_redone() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    let entity = scene.spawn("Before");

    editor.rename(&mut scene, entity, "After".into());
    assert_eq!(tag_of(&scene, entity), "After");

    assert!(editor.undo(&mut scene));
    assert_eq!(tag_of(&scene, entity), "Before");

    assert!(editor.redo(&mut scene));
    assert_eq!(tag_of(&scene, entity), "After");
}

#[test]
fn a_reparent_is_undone_and_redone() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    let root = scene.spawn("Root");
    let child = scene.spawn("Child");

    assert!(editor.reparent(&mut scene, child, Some(root)));
    assert_eq!(scene.parent(child), Some(root));

    assert!(editor.undo(&mut scene));
    assert_eq!(scene.parent(child), None);

    assert!(editor.redo(&mut scene));
    assert_eq!(scene.parent(child), Some(root));

    // And back up to the root.
    let grandchild = scene.spawn("Grandchild");
    assert!(editor.reparent(&mut scene, grandchild, Some(child)));
    assert!(editor.reparent(&mut scene, grandchild, None));
    assert!(editor.undo(&mut scene));
    assert_eq!(scene.parent(grandchild), Some(child));
}

#[test]
fn a_property_edit_is_undone_and_redone() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    let entity = scene.spawn("Prop");
    editor.select(Some(entity));

    let translation = translation_property(&editor, &scene);
    editor.set_property(&mut scene, &translation, PropertyValue::Vec3([1.0, 2.0, 3.0]));
    assert_eq!(
        translation.value(&scene, entity),
        PropertyValue::Vec3([1.0, 2.0, 3.0])
    );

    assert!(editor.undo(&mut scene));
    assert_eq!(
        translation.value(&scene, entity),
        PropertyValue::Vec3([0.0, 0.0, 0.0])
    );

    assert!(editor.redo(&mut scene));
    assert_eq!(
        translation.value(&scene, entity),
        PropertyValue::Vec3([1.0, 2.0, 3.0])
    );

    // Setting the value it already has is not an edit.
    let length = editor.history.len();
    editor.set_property(&mut scene, &translation, PropertyValue::Vec3([1.0, 2.0, 3.0]));
    assert_eq!(editor.history.len(), length);
}

#[test]
fn the_history_is_bounded_and_the_oldest_edit_falls_off() {
    let mut scene = Scene::new();
    let entity = scene.spawn("A");
    let mut inspector = Inspector::new();
    let mut history = History::with_capacity(2);

    for name in ["B", "C", "D"] {
        let mut command = Command::Rename {
            entity,
            before: tag_of(&scene, entity),
            after: name.to_string(),
        };
        command.apply(&mut scene, &mut inspector);
        history.push(command);
    }

    assert_eq!(history.capacity(), 2);
    assert_eq!(history.len(), 2);
    assert_eq!(tag_of(&scene, entity), "D");

    assert!(history.undo(&mut scene, &mut inspector));
    assert_eq!(tag_of(&scene, entity), "C");
    assert!(history.undo(&mut scene, &mut inspector));
    assert_eq!(tag_of(&scene, entity), "B");

    // "A to B" fell off the end when "C to D" arrived.
    assert!(!history.undo(&mut scene, &mut inspector));
    assert!(!history.can_undo());
    assert_eq!(tag_of(&scene, entity), "B");
}

#[test]
fn a_zero_capacity_history_keeps_nothing() {
    let mut scene = Scene::new();
    let entity = scene.spawn("A");
    let mut inspector = Inspector::new();
    let mut history = History::with_capacity(0);

    let mut command = Command::Rename {
        entity,
        before: "A".into(),
        after: "B".into(),
    };
    command.apply(&mut scene, &mut inspector);
    history.push(command);

    assert!(history.is_empty());
    assert!(!history.can_undo());
    assert_eq!(tag_of(&scene, entity), "B");
}

#[test]
fn a_new_edit_throws_the_redo_stack_away() {
    let mut scene = Scene::new();
    let mut editor = Editor::new();
    let entity = scene.spawn("Start");

    editor.rename(&mut scene, entity, "Middle".into());
    assert!(editor.undo(&mut scene));
    assert!(editor.history.can_redo());

    editor.rename(&mut scene, entity, "Other".into());
    assert!(!editor.history.can_redo());
    assert!(!editor.redo(&mut scene));
    assert_eq!(tag_of(&scene, entity), "Other");
}

#[test]
fn commands_name_themselves_for_a_menu() {
    let mut scene = Scene::new();
    let entity = scene.spawn("Hero");
    assert_eq!(
        Command::CreateEntity {
            entity,
            parent: None,
            name: "Hero".into()
        }
        .label(),
        "Create Hero"
    );
    assert_eq!(
        Command::Rename {
            entity,
            before: "Hero".into(),
            after: "Villain".into()
        }
        .label(),
        "Rename Hero to Villain"
    );
    assert_eq!(
        Command::SetProperty {
            entity,
            component: "Transform",
            property: "Scale",
            before: PropertyValue::Vec3([1.0, 1.0, 1.0]),
            after: PropertyValue::Vec3([2.0, 2.0, 2.0]),
        }
        .label(),
        "Set Transform.Scale"
    );
    assert_eq!(
        Command::Reparent {
            entity,
            before: None,
            after: None
        }
        .label(),
        "Reparent"
    );

    let mut editor = Editor::new();
    editor.create_entity(&mut scene, None);
    assert_eq!(editor.history.undo_label().as_deref(), Some("Create Empty Entity"));
    assert_eq!(editor.history.redo_label(), None);
    assert!(editor.undo(&mut scene));
    assert_eq!(editor.history.redo_label().as_deref(), Some("Create Empty Entity"));
}
