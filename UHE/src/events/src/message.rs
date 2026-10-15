//! Messages: queued, typed, deferred, single consumer.

use std::any::{Any, TypeId};

/// A message is a typed, deferred, single-consumer value.
pub trait Message: Any + Send + Sync + 'static {
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Erased handler stores itself by message type.
pub trait ErasedMessageHandler: Send {
    fn handle(&mut self, message: Box<dyn Any + Send + Sync>);
}

/// Adapter for a typed single-consumer handler.
pub struct FnMessageHandler<M, F> {
    _marker: std::marker::PhantomData<M>,
    handler: F,
}

impl<M, F> FnMessageHandler<M, F>
where
    M: Message,
    F: FnMut(M) + Send + 'static,
{
    pub fn new(handler: F) -> Self {
        Self {
            _marker: std::marker::PhantomData,
            handler,
        }
    }
}

impl<M, F> ErasedMessageHandler for FnMessageHandler<M, F>
where
    M: Message,
    F: FnMut(M) + Send + 'static,
{
    fn handle(&mut self, message: Box<dyn Any + Send + Sync>) {
        if let Ok(typed) = message.downcast::<M>() {
            (self.handler)(*typed);
        }
    }
}

/// An erased queued message.
pub struct QueuedMessage {
    pub type_id: TypeId,
    pub payload: Box<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for QueuedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QueuedMessage({:?})", self.type_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(pub TypeId);

impl MessageId {
    pub fn of<M: Message>() -> Self {
        Self(TypeId::of::<M>())
    }
}
