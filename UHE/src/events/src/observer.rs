//! ECS observer: component add/remove/change observation.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// A simple entity key. ECS integration supplies richer handles.
pub type Entity = u64;
pub type ObserverCallback<E> = Box<dyn FnMut(E, &mut dyn Any) + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObserverId(pub u64);

pub struct EcsObserver<E = Entity> {
    added: HashMap<TypeId, Vec<ObserverCallback<E>>>,
    removed: HashMap<TypeId, Vec<ObserverCallback<E>>>,
    changed: HashMap<TypeId, Vec<ObserverCallback<E>>>,
    next_id: u64,
}

impl<E> EcsObserver<E>
where
    E: Copy + Send + 'static,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_added<C, F>(&mut self, handler: F)
    where
        C: 'static,
        F: FnMut(E, &mut C) + Send + 'static,
    {
        let mut handler = handler;
        let boxed: ObserverCallback<E> = Box::new(move |entity, any| {
            if let Some(comp) = any.downcast_mut::<C>() {
                (handler)(entity, comp);
            }
        });
        self.added.entry(TypeId::of::<C>()).or_default().push(boxed);
    }

    pub fn on_removed<C, F>(&mut self, handler: F)
    where
        C: 'static,
        F: FnMut(E, &mut C) + Send + 'static,
    {
        let mut handler = handler;
        let boxed: ObserverCallback<E> = Box::new(move |entity, any| {
            if let Some(comp) = any.downcast_mut::<C>() {
                (handler)(entity, comp);
            }
        });
        self.removed
            .entry(TypeId::of::<C>())
            .or_default()
            .push(boxed);
    }

    pub fn on_changed<C, F>(&mut self, handler: F)
    where
        C: 'static,
        F: FnMut(E, &mut C) + Send + 'static,
    {
        let mut handler = handler;
        let boxed: ObserverCallback<E> = Box::new(move |entity, any| {
            if let Some(comp) = any.downcast_mut::<C>() {
                (handler)(entity, comp);
            }
        });
        self.changed
            .entry(TypeId::of::<C>())
            .or_default()
            .push(boxed);
    }

    pub fn notify_added<C>(&mut self, entity: E, component: &mut C)
    where
        C: 'static,
    {
        if let Some(ls) = self.added.get_mut(&TypeId::of::<C>()) {
            for cb in ls.iter_mut() {
                cb(entity, component as &mut dyn Any);
            }
        }
    }

    pub fn notify_removed<C>(&mut self, entity: E, component: &mut C)
    where
        C: 'static,
    {
        if let Some(ls) = self.removed.get_mut(&TypeId::of::<C>()) {
            for cb in ls.iter_mut() {
                cb(entity, component as &mut dyn Any);
            }
        }
    }

    pub fn notify_changed<C>(&mut self, entity: E, component: &mut C)
    where
        C: 'static,
    {
        if let Some(ls) = self.changed.get_mut(&TypeId::of::<C>()) {
            for cb in ls.iter_mut() {
                cb(entity, component as &mut dyn Any);
            }
        }
    }

    pub fn clear(&mut self) {
        self.added.clear();
        self.removed.clear();
        self.changed.clear();
    }

    pub fn alloc_id(&mut self) -> ObserverId {
        self.next_id += 1;
        ObserverId(self.next_id)
    }
}

impl<E> Default for EcsObserver<E> {
    fn default() -> Self {
        Self {
            added: HashMap::new(),
            removed: HashMap::new(),
            changed: HashMap::new(),
            next_id: 0,
        }
    }
}
