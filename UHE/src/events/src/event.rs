//! Strongly typed events: immediate broadcast to many subscribers.

use std::any::{Any, TypeId};
use std::marker::PhantomData;

/// Category bitflags for an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventCategory(pub u32);

impl EventCategory {
    pub const NONE: EventCategory = EventCategory(0);
    pub const APPLICATION: EventCategory = EventCategory(1);
    pub const INPUT: EventCategory = EventCategory(2);
    pub const KEYBOARD: EventCategory = EventCategory(4);
    pub const MOUSE: EventCategory = EventCategory(8);
    pub const MOUSE_BUTTON: EventCategory = EventCategory(16);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub fn contains(self, other: EventCategory) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for EventCategory {
    type Output = EventCategory;
    fn bitor(self, rhs: Self) -> Self {
        EventCategory(self.0 | rhs.0)
    }
}

impl std::ops::BitAnd for EventCategory {
    type Output = EventCategory;
    fn bitand(self, rhs: Self) -> Self {
        EventCategory(self.0 & rhs.0)
    }
}

/// An event is a strongly typed, broadcast, immediate value.
pub trait Event: Any + Send + Sync + 'static {
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn category(&self) -> EventCategory {
        EventCategory::NONE
    }
}

pub type EventId = TypeId;

/// Type-erased listener handle (trait object).
pub trait ErasedEventListener: Send {
    fn handle(&mut self, event: &mut dyn Any) -> bool;
}

/// Adapter storing a typed closure.
pub struct FnEventListener<E, F> {
    _marker: PhantomData<E>,
    handler: F,
}

impl<E, F> FnEventListener<E, F>
where
    E: Event,
    F: FnMut(&mut E) -> bool + Send + 'static,
{
    pub fn new(handler: F) -> Self {
        Self {
            _marker: PhantomData,
            handler,
        }
    }
}

impl<E, F> ErasedEventListener for FnEventListener<E, F>
where
    E: Event,
    F: FnMut(&mut E) -> bool + Send + 'static,
{
    fn handle(&mut self, event: &mut dyn Any) -> bool {
        if let Some(e) = event.downcast_mut::<E>() {
            (self.handler)(e)
        } else {
            false
        }
    }
}

/// Adapter for handlers that return no value.
pub struct VoidEventListener<E, F> {
    _marker: PhantomData<E>,
    handler: F,
}

impl<E, F> VoidEventListener<E, F>
where
    E: Event,
    F: FnMut(&mut E) + Send + 'static,
{
    pub fn new(handler: F) -> Self {
        Self {
            _marker: PhantomData,
            handler,
        }
    }
}

impl<E, F> ErasedEventListener for VoidEventListener<E, F>
where
    E: Event,
    F: FnMut(&mut E) + Send + 'static,
{
    fn handle(&mut self, event: &mut dyn Any) -> bool {
        if let Some(e) = event.downcast_mut::<E>() {
            (self.handler)(e);
            false
        } else {
            false
        }
    }
}
