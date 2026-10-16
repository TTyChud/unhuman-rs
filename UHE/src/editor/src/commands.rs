//! The edit history: a small command enum, a bounded undo stack, and the
//! inverses. The C++ editor had none of this, so every mistake in it was
//! permanent.

use std::collections::VecDeque;

use meats::{Entity, Scene, Tag};

use crate::inspector::{Inspector, PropertyValue};

/// How many edits are kept. Bounded because a scene clone-sized leak is worse
/// than a forgotten edit, and 128 is more than anyone reaches for by hand.
pub const DEFAULT_CAPACITY: usize = 128;

#[derive(Clone, PartialEq, Debug)]
pub enum Command {
    CreateEntity {
        entity: Entity,
        parent: Option<Entity>,
        name: String,
    },
    DestroyEntity {
        entity: Entity,
        parent: Option<Entity>,
        name: String,
    },
    SetProperty {
        entity: Entity,
        component: &'static str,
        property: &'static str,
        before: PropertyValue,
        after: PropertyValue,
    },
    Rename {
        entity: Entity,
        before: String,
        after: String,
    },
    Reparent {
        entity: Entity,
        before: Option<Entity>,
        after: Option<Entity>,
    },
}

impl Command {
    /// What a menu item says.
    pub fn label(&self) -> String {
        match self {
            Command::CreateEntity { name, .. } => format!("Create {name}"),
            Command::DestroyEntity { name, .. } => format!("Delete {name}"),
            Command::SetProperty {
                component,
                property,
                ..
            } => format!("Set {component}.{property}"),
            Command::Rename { before, after, .. } => format!("Rename {before} to {after}"),
            Command::Reparent { .. } => "Reparent".to_string(),
        }
    }

    /// Does the edit. `&mut self` because undoing a create hands the entity
    /// back to the scene on redo, and that is a new handle.
    pub fn apply(&mut self, scene: &mut Scene, inspector: &mut Inspector) {
        match self {
            Command::CreateEntity {
                entity,
                parent,
                name,
            } => {
                let created = scene.spawn(name.as_str());
                set_parent_if_present(scene, created, *parent);
                *entity = created;
            }
            Command::DestroyEntity { entity, .. } => scene.destroy(*entity),
            Command::SetProperty {
                entity,
                component,
                property,
                after,
                ..
            } => {
                inspector.set_named(scene, *entity, component, property, after);
            }
            Command::Rename { entity, after, .. } => set_tag(scene, *entity, after.as_str()),
            Command::Reparent { entity, after, .. } => scene.set_parent(*entity, *after),
        }
    }

    /// Puts the scene back. Undoing a destroy rebuilds the entity, its name and
    /// its parent: the components it held were dropped with it.
    ///
    /// shortcut: a destroyed entity's components do not come back, because
    /// `meats` has no per-entity snapshot to rebuild them from; add one and
    /// this inverse becomes exact.
    pub fn revert(&mut self, scene: &mut Scene, inspector: &mut Inspector) {
        match self {
            Command::CreateEntity { entity, .. } => scene.destroy(*entity),
            Command::DestroyEntity {
                entity,
                parent,
                name,
            } => {
                let restored = scene.spawn(name.as_str());
                set_parent_if_present(scene, restored, *parent);
                *entity = restored;
            }
            Command::SetProperty {
                entity,
                component,
                property,
                before,
                ..
            } => {
                inspector.set_named(scene, *entity, component, property, before);
            }
            Command::Rename { entity, before, .. } => set_tag(scene, *entity, before.as_str()),
            Command::Reparent { entity, before, .. } => scene.set_parent(*entity, *before),
        }
    }
}

pub(crate) fn set_tag(scene: &mut Scene, entity: Entity, name: &str) {
    if let Some(mut entry) = scene.get_mut(entity) {
        if let Some(tag) = entry.get_mut::<Tag>() {
            tag.tag = name.to_string();
        }
    }
}

/// A parent that has since been destroyed is dropped rather than restored, so
/// an undo cannot attach an entity to a handle the scene no longer knows.
fn set_parent_if_present(scene: &mut Scene, child: Entity, parent: Option<Entity>) {
    if let Some(parent) = parent {
        if scene.get(parent).is_some() {
            scene.set_parent(child, Some(parent));
        }
    }
}

pub struct History {
    capacity: usize,
    done: VecDeque<Command>,
    undone: Vec<Command>,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// A capacity of zero keeps nothing, which is how a headless run turns undo
    /// off without a second code path.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            done: VecDeque::new(),
            undone: Vec::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Records an edit the caller has already made. Anything that was undone
    /// is gone: the timeline just branched.
    pub fn push(&mut self, command: Command) {
        self.undone.clear();
        self.done.push_back(command);
        while self.done.len() > self.capacity {
            self.done.pop_front();
        }
    }

    pub fn undo(&mut self, scene: &mut Scene, inspector: &mut Inspector) -> bool {
        let Some(mut command) = self.done.pop_back() else {
            return false;
        };
        command.revert(scene, inspector);
        self.undone.push(command);
        true
    }

    pub fn redo(&mut self, scene: &mut Scene, inspector: &mut Inspector) -> bool {
        let Some(mut command) = self.undone.pop() else {
            return false;
        };
        command.apply(scene, inspector);
        self.done.push_back(command);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Edits on the undo stack.
    pub fn len(&self) -> usize {
        self.done.len()
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }

    /// The label of the edit undo would revert.
    pub fn undo_label(&self) -> Option<String> {
        self.done.back().map(Command::label)
    }

    pub fn redo_label(&self) -> Option<String> {
        self.undone.last().map(Command::label)
    }

    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }
}
