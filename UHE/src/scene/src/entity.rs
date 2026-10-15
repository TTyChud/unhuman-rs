//! Entity handles and the per-entity component views.

use hecs::Ref;

use crate::components::{Id, Tag};
use crate::scene::Scene;

/// A generational handle into a [`Scene`]. Deliberately *not* `hecs::Entity`:
/// the scene owns the mapping, so a handle taken before a destroy is refused by
/// [`Scene::get`] instead of addressing whatever now occupies the slot.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Entity {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl Entity {
    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }
}

/// Read-only view of one entity. Every accessor is shared, so several views may
/// be live at once.
pub struct EntityRef<'a> {
    pub(crate) scene: &'a Scene,
    pub(crate) entity: Entity,
}

impl<'a> EntityRef<'a> {
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// The `Id` component — the identity that survives a save.
    pub fn id(&self) -> Option<u64> {
        self.get::<Id>().map(|id| id.0)
    }

    pub fn tag(&self) -> Option<String> {
        self.get::<Tag>().map(|tag| tag.tag.clone())
    }

    pub fn has<T: hecs::Component>(&self) -> bool {
        let scene: &'a Scene = self.scene;
        match scene.hecs_of(self.entity) {
            Some(hecs) => scene.world.satisfies::<&T>(hecs),
            None => false,
        }
    }

    /// Borrow a component. The result is `hecs::Ref` (re-exported as
    /// [`crate::Ref`]), which derefs to `&T`.
    pub fn get<T: hecs::Component>(&self) -> Option<Ref<'a, T>> {
        let scene: &'a Scene = self.scene;
        let hecs = scene.hecs_of(self.entity)?;
        scene.world.get::<&T>(hecs).ok()
    }
}

/// Read-write view of one entity. Holding it borrows the scene, so there is
/// exactly one at a time.
pub struct EntityRefMut<'a> {
    pub(crate) scene: &'a mut Scene,
    pub(crate) entity: Entity,
}

impl<'a> EntityRefMut<'a> {
    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn id(&self) -> Option<u64> {
        self.get::<Id>().map(|id| id.0)
    }

    pub fn tag(&self) -> Option<String> {
        self.get::<Tag>().map(|tag| tag.tag.clone())
    }

    pub fn has<T: hecs::Component>(&self) -> bool {
        self.scene.has::<T>(self.entity)
    }

    pub fn get<T: hecs::Component>(&self) -> Option<Ref<'_, T>> {
        let hecs = self.scene.hecs_of(self.entity)?;
        self.scene.world.get::<&T>(hecs).ok()
    }

    pub fn get_mut<T: hecs::Component>(&mut self) -> Option<&mut T> {
        let hecs = self.scene.hecs_of(self.entity)?;
        self.scene.world.query_one_mut::<&mut T>(hecs).ok()
    }

    pub fn insert<T: hecs::Component>(&mut self, component: T) {
        self.scene.insert(self.entity, component);
    }

    pub fn remove<T: hecs::Component>(&mut self) -> Option<T> {
        self.scene.remove::<T>(self.entity)
    }
}
