//! The outliner: tree shape, expansion, filtering, rename, and the drop rule
//! the C++ panel got wrong once.

use meats::{Entity, Scene};
use mirror::hierarchy::Hierarchy;
use mirror::Editor;

/// Root -> Child -> Grandchild.
fn tree() -> (Scene, Entity, Entity, Entity) {
    let mut scene = Scene::new();
    let root = scene.spawn("Root");
    let child = scene.spawn("Child");
    let grandchild = scene.spawn("Grandchild");
    scene.set_parent(child, Some(root));
    scene.set_parent(grandchild, Some(child));
    (scene, root, child, grandchild)
}

fn names(rows: &[mirror::Row]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}

fn depths(rows: &[mirror::Row]) -> Vec<usize> {
    rows.iter().map(|row| row.depth).collect()
}

#[test]
fn a_collapsed_root_hides_its_subtree() {
    let (scene, root, _, _) = tree();
    let hierarchy = Hierarchy::new();
    let rows = hierarchy.rows(&scene);
    assert_eq!(names(&rows), vec!["Root"]);
    assert!(rows[0].has_children);
    assert!(!rows[0].expanded);
    assert_eq!(rows[0].entity, root);
}

#[test]
fn expanding_shows_the_grandchild_at_its_depth() {
    let (scene, root, child, grandchild) = tree();
    let mut hierarchy = Hierarchy::new();
    hierarchy.set_expanded(root, true);
    hierarchy.set_expanded(child, true);

    let rows = hierarchy.rows(&scene);
    assert_eq!(names(&rows), vec!["Root", "Child", "Grandchild"]);
    assert_eq!(depths(&rows), vec![0, 1, 2]);
    assert_eq!(rows[2].entity, grandchild);
    assert!(!rows[2].has_children);
    assert!(rows[2].expanded, "a leaf is always open");

    // Collapsing the child hides only what is below it.
    hierarchy.set_expanded(child, false);
    assert_eq!(names(&hierarchy.rows(&scene)), vec!["Root", "Child"]);
}

#[test]
fn the_filter_narrows_the_list_to_matches_and_their_ancestors() {
    let (scene, _, _, _) = tree();
    let mut hierarchy = Hierarchy::new();

    hierarchy.filter = "grand".into();
    let rows = hierarchy.rows(&scene);
    assert_eq!(names(&rows), vec!["Root", "Child", "Grandchild"]);
    assert!(
        rows.iter().all(|row| row.expanded),
        "matches force their path open"
    );

    hierarchy.filter = "root".into();
    assert_eq!(names(&hierarchy.rows(&scene)), vec!["Root"]);

    hierarchy.filter = "NOTHING".into();
    assert!(hierarchy.rows(&scene).is_empty());

    hierarchy.clear_filter();
    assert_eq!(names(&hierarchy.rows(&scene)), vec!["Root"]);
}

#[test]
fn an_entity_may_not_be_dropped_into_its_own_subtree() {
    let (scene, root, child, grandchild) = tree();
    let hierarchy = Hierarchy::new();

    assert!(!hierarchy.can_drop(&scene, root, root));
    assert!(!hierarchy.can_drop(&scene, root, child));
    assert!(!hierarchy.can_drop(&scene, root, grandchild));
    assert!(!hierarchy.can_drop(&scene, child, grandchild));

    assert!(hierarchy.can_drop(&scene, grandchild, root));
    assert!(hierarchy.can_drop(&scene, child, root));
}

#[test]
fn dropping_a_parent_onto_its_own_child_is_refused_by_the_model() {
    let (mut scene, root, child, grandchild) = tree();
    let mut editor = Editor::new();

    assert!(!editor.reparent(&mut scene, root, Some(grandchild)));
    assert_eq!(scene.parent(root), None);
    assert_eq!(scene.parent(child), Some(root));

    assert!(editor.reparent(&mut scene, grandchild, Some(root)));
    assert_eq!(scene.parent(grandchild), Some(root));
    assert!(editor.undo(&mut scene));
    assert_eq!(scene.parent(grandchild), Some(child));
}

#[test]
fn a_rename_reaches_the_tag_and_the_row() {
    let (mut scene, root, child, _) = tree();
    let mut editor = Editor::new();
    editor.set_expanded(root, true);

    editor.rename(&mut scene, child, "Player".into());
    assert_eq!(scene.get(child).unwrap().tag().as_deref(), Some("Player"));
    let rows = editor.rows(&scene);
    assert!(rows.iter().any(|row| row.name == "Player"));

    assert!(editor.undo(&mut scene));
    assert_eq!(scene.get(child).unwrap().tag().as_deref(), Some("Child"));
}

#[test]
fn the_inline_rename_editor_commits_through_the_history() {
    let (mut scene, _, child, _) = tree();
    let mut editor = Editor::new();

    editor.begin_rename(&scene, child);
    assert_eq!(editor.hierarchy.renaming, Some(child));
    assert_eq!(editor.hierarchy.rename_buffer, "Child");

    editor.hierarchy.rename_buffer = "Hero".into();
    assert!(editor.commit_rename(&mut scene));
    assert_eq!(scene.get(child).unwrap().tag().as_deref(), Some("Hero"));
    assert!(editor.hierarchy.renaming.is_none());

    // An unchanged name is not an edit.
    let before = editor.history.len();
    editor.begin_rename(&scene, child);
    assert!(!editor.commit_rename(&mut scene));
    assert_eq!(editor.history.len(), before);
}

#[test]
fn deleting_the_selection_clears_it() {
    let (mut scene, _, child, grandchild) = tree();
    let mut editor = Editor::new();

    editor.select(Some(grandchild));
    editor.destroy_entity(&mut scene, child);
    assert_eq!(editor.selected_entity(), None);
    assert!(scene.get(grandchild).is_none());
}
