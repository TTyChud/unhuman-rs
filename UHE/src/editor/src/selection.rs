//! What the editor has selected and what the pointer is over. Kept in one
//! place because the outliner, the viewport and the inspector all read it, and
//! the C++ scattered it across `m_SelectionContext`, `m_HoverdEntity` and a
//! per-viewport pick.

use meats::Entity;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    pub selected: Option<Entity>,
    pub hovered: Option<Entity>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn select(&mut self, entity: Option<Entity>) {
        self.selected = entity;
    }

    pub fn hover(&mut self, entity: Option<Entity>) {
        self.hovered = entity;
    }

    /// Dropping a selection that is no longer in the scene, which the C++
    /// pointer-hover path did by rebuilding the handle every frame.
    pub fn clear(&mut self) {
        self.selected = None;
        self.hovered = None;
    }

    pub fn is_selected(&self, entity: Entity) -> bool {
        self.selected == Some(entity)
    }

    pub fn is_hovered(&self, entity: Entity) -> bool {
        self.hovered == Some(entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meats::Scene;

    #[test]
    fn selection_follows_the_last_call() {
        let mut scene = Scene::new();
        let a = scene.spawn("A");
        let b = scene.spawn("B");

        let mut selection = Selection::new();
        assert!(!selection.is_selected(a));
        selection.select(Some(a));
        assert!(selection.is_selected(a));
        selection.hover(Some(b));
        assert!(selection.is_hovered(b));
        assert!(selection.is_selected(a));

        selection.select(Some(b));
        assert!(!selection.is_selected(a));

        selection.clear();
        assert_eq!(selection, Selection::new());
    }
}
