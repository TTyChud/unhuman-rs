//! Deferred queues.

use std::any::{Any, TypeId};
use std::collections::VecDeque;

use crate::message::QueuedMessage;

#[derive(Default)]
pub struct Queue<T> {
    inner: VecDeque<T>,
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Self {
            inner: VecDeque::new(),
        }
    }

    pub fn push(&mut self, value: T) {
        self.inner.push_back(value);
    }

    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop_front()
    }

    pub fn pop_front(&mut self) -> Option<T> {
        self.inner.pop_front()
    }

    pub fn peek(&self) -> Option<&T> {
        self.inner.front()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn drain(&mut self) -> Vec<T> {
        self.inner.drain(..).collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.inner.iter()
    }
}

impl<T> FromIterator<T> for Queue<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut q = Self::new();
        for v in iter {
            q.push(v);
        }
        q
    }
}

#[derive(Default)]
pub struct MessageQueue {
    inner: VecDeque<QueuedMessage>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            inner: VecDeque::new(),
        }
    }

    pub fn push(&mut self, type_id: TypeId, payload: Box<dyn Any + Send + Sync>) {
        self.inner.push_back(QueuedMessage { type_id, payload });
    }

    pub fn push_message<M>(&mut self, message: M)
    where
        M: crate::message::Message,
    {
        self.push(TypeId::of::<M>(), Box::new(message));
    }

    pub fn pop(&mut self) -> Option<QueuedMessage> {
        self.inner.pop_front()
    }

    pub fn drain(&mut self) -> Vec<QueuedMessage> {
        self.inner.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }
}
