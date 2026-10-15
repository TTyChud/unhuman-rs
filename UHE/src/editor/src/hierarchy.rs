//! The outliner's model: the tree under a `meats::Scene`, which rows are open,
//! what the filter hides, and which drops are legal.
//!
//! The C++ panel keyed expansion by entity UUID and validated a parent/child
//! pair by asking the child for its parent. Here the tree is walked downwards
//! from the roots, so a stale child entry cannot draw a second row, and a drop
//! is refused when the target is inside the dragged subtree — the case the C++
//! panel allowed once, producing a cycle.

use std::collections::HashSet;

use meats::{Entity, Scene};

use crate::commands::Command;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    pub entity: Entity,
    pub depth: usize,
    pub name: String,
    pub has_children: bool,
    pub expanded: bool,
}

#[derive(Default)]
pub struct Hierarchy {
    /// Case-insensitive substring filter; empty shows everything.
    pub filter: String,
    pub renaming: Option<Entity>,
    pub rename_buffer: String,
    expanded: HashSet<Entity>,
    drag: Option<Entity>,
}

impl Hierarchy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every visible row: the roots, and the children of the expanded ones. A
    /// filter forces the ancestors of a match open instead of hiding them.
    pub fn rows(&self, scene: &Scene) -> Vec<Row> {
        let mut rows = Vec::new();
        let filter = self.filter.trim().to_ascii_lowercase();
        let mut roots = self.roots(scene);
        roots.sort_by_key(|entity| tag_name(scene, *entity));
        for root in roots {
            if filter.is_empty() {
                self.walk(scene, root, 0, &mut rows);
            } else {
                self.walk_filtered(scene, root, 0, &filter, &mut rows);
            }
        }
        rows
    }

    /// Entities that are nobody's child. `Scene` has one owner per entity, so
    /// the roots are exactly the complement of every parent's child list.
    fn roots(&self, scene: &Scene) -> Vec<Entity> {
        let entities = scene.entities();
        let mut linked: HashSet<Entity> = HashSet::new();
        for entity in &entities {
            for child in scene.children(*entity) {
                linked.insert(*child);
            }
        }
        entities
            .into_iter()
            .filter(|entity| !linked.contains(entity))
            .collect()
    }

    fn walk(&self, scene: &Scene, entity: Entity, depth: usize, rows: &mut Vec<Row>) {
        let children = scene.children(entity);
        let has_children = !children.is_empty();
        let expanded = !has_children || self.expanded.contains(&entity);
        rows.push(Row {
            entity,
            depth,
            name: tag_name(scene, entity),
            has_children,
            expanded,
        });
        if expanded {
            for child in children {
                self.walk(scene, *child, depth + 1, rows);
            }
        }
    }

    /// Appends the matching entities and their ancestors, and reports whether
    /// this subtree contributed anything.
    fn walk_filtered(
        &self,
        scene: &Scene,
        entity: Entity,
        depth: usize,
        filter: &str,
        rows: &mut Vec<Row>,
    ) -> bool {
        let start = rows.len();
        let children = scene.children(entity);
        let has_children = !children.is_empty();
        let mut matched_below = false;
        for child in children {
            if self.walk_filtered(scene, *child, depth + 1, filter, rows) {
                matched_below = true;
            }
        }
        let name = tag_name(scene, entity);
        let matched = matched_below || name.to_ascii_lowercase().contains(filter);
        if matched {
            rows.insert(
                start,
                Row {
                    entity,
                    depth,
                    name,
                    has_children,
                    expanded: true,
                },
            );
        }
        matched
    }

    pub fn is_expanded(&self, entity: Entity) -> bool {
        self.expanded.contains(&entity)
    }

    pub fn set_expanded(&mut self, entity: Entity, expanded: bool) {
        if expanded {
            self.expanded.insert(entity);
        } else {
            self.expanded.remove(&entity);
        }
    }

    pub fn toggle_expanded(&mut self, entity: Entity) {
        let expanded = self.is_expanded(entity);
        self.set_expanded(entity, !expanded);
    }

    /// Drops expansion state and any rename in flight; the C++ panel did this
    /// when the panel was pointed at a different scene.
    pub fn reset(&mut self) {
        self.expanded.clear();
        self.renaming = None;
        self.rename_buffer.clear();
        self.drag = None;
    }

    pub fn begin_rename(&mut self, scene: &Scene, entity: Entity) {
        self.rename_buffer = tag_name(scene, entity);
        self.renaming = Some(entity);
    }

    pub fn cancel_rename(&mut self) {
        self.renaming = None;
        self.rename_buffer.clear();
    }

    /// Closes the rename editor and yields the undoable edit, or `None` when
    /// the name did not change. The scene is not touched: undo has to be able
    /// to reach the old name, so the edit goes through the command history.
    pub fn commit_rename(&mut self, scene: &Scene) -> Option<Command> {
        let entity = self.renaming.take()?;
        let before = tag_name(scene, entity);
        let after = std::mem::take(&mut self.rename_buffer);
        if after.is_empty() || after == before {
            return None;
        }
        Some(Command::Rename {
            entity,
            before,
            after,
        })
    }

    pub fn begin_drag(&mut self, entity: Entity) {
        self.drag = Some(entity);
    }

    pub fn dragging(&self) -> Option<Entity> {
        self.drag
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// Whether `dragged` may become a child of `onto`. An entity cannot be
    /// parented to itself or into its own subtree.
    pub fn can_drop(&self, scene: &Scene, dragged: Entity, onto: Entity) -> bool {
        !is_within_subtree(scene, dragged, onto)
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
    }
}

pub(crate) fn tag_name(scene: &Scene, entity: Entity) -> String {
    scene
        .get(entity)
        .and_then(|entry| entry.tag())
        .unwrap_or_default()
        .to_string()
}

/// Whether `entity` is `root` or below it. The drag-drop rule and the delete
/// rule are the same walk.
pub fn is_within_subtree(scene: &Scene, root: Entity, entity: Entity) -> bool {
    let mut frontier = vec![root];
    let mut seen: HashSet<Entity> = HashSet::from([root]);
    while let Some(current) = frontier.pop() {
        if current == entity {
            return true;
        }
        for child in scene.children(current) {
            if seen.insert(*child) {
                frontier.push(*child);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_are_found_downwards_only() {
        let mut scene = Scene::new();
        let grandparent = scene.spawn("A");
        let parent = scene.spawn("B");
        let child = scene.spawn("C");
        scene.set_parent(parent, Some(grandparent));
        scene.set_parent(child, Some(parent));

        assert!(is_within_subtree(&scene, grandparent, child));
        assert!(is_within_subtree(&scene, grandparent, grandparent));
        assert!(is_within_subtree(&scene, parent, child));
        assert!(!is_within_subtree(&scene, child, parent));
        assert!(!is_within_subtree(&scene, parent, grandparent));
    }
}
