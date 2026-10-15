//! Central event dispatcher combining events and messages.

use std::any::{Any, TypeId};
use std::collections::HashMap;

use crate::event::{ErasedEventListener, Event};
use crate::message::{ErasedMessageHandler, Message};
use crate::queue::{MessageQueue, Queue};

pub type EventQueue = Queue<Box<dyn Any + Send + Sync>>;

pub struct Dispatcher {
    listeners: HashMap<TypeId, Vec<Box<dyn ErasedEventListener>>>,
    deferred_events: Vec<(TypeId, Box<dyn Any + Send + Sync>)>,
    message_handlers: HashMap<TypeId, Box<dyn ErasedMessageHandler>>,
    message_queue: MessageQueue,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            deferred_events: Vec::new(),
            message_handlers: HashMap::new(),
            message_queue: MessageQueue::new(),
        }
    }

    // --- listeners / immediate events ---
    pub fn subscribe<E, F>(&mut self, handler: F)
    where
        E: Event,
        F: FnMut(&mut E) -> bool + Send + 'static,
    {
        use crate::event::FnEventListener;
        let entry = self.listeners.entry(TypeId::of::<E>()).or_default();
        entry.push(Box::new(FnEventListener::new(handler)));
    }

    pub fn listen<E, F>(&mut self, handler: F)
    where
        E: Event,
        F: FnMut(&mut E) + Send + 'static,
    {
        use crate::event::VoidEventListener;
        let entry = self.listeners.entry(TypeId::of::<E>()).or_default();
        entry.push(Box::new(VoidEventListener::new(handler)));
    }

    pub fn register<E, F>(&mut self, handler: F)
    where
        E: Event,
        F: FnMut(&mut E) + Send + 'static,
    {
        self.listen(handler);
    }

    pub fn add_listener<E, F>(&mut self, handler: F)
    where
        E: Event,
        F: FnMut(&mut E) + Send + 'static,
    {
        self.listen(handler);
    }

    pub fn on<E, F>(&mut self, handler: F)
    where
        E: Event,
        F: FnMut(&mut E) + Send + 'static,
    {
        self.listen(handler);
    }

    pub fn add_handler<E, F>(&mut self, handler: F)
    where
        E: Event,
        F: FnMut(&mut E) -> bool + Send + 'static,
    {
        self.subscribe(handler);
    }

    pub fn dispatch<E: Event>(&mut self, mut event: E) {
        let type_id = TypeId::of::<E>();
        if let Some(listeners) = self.listeners.get_mut(&type_id) {
            let erased: &mut dyn Any = &mut event;
            for l in listeners.iter_mut() {
                l.handle(erased);
            }
        }
    }

    pub fn emit<E: Event>(&mut self, event: E) {
        self.dispatch(event);
    }

    pub fn fire<E: Event>(&mut self, event: E) {
        self.dispatch(event);
    }

    pub fn notify<E: Event>(&mut self, event: E) {
        self.dispatch(event);
    }

    pub fn send_event<E: Event>(&mut self, event: E) {
        self.dispatch(event);
    }

    // --- deferred events ---
    pub fn defer<E: Event>(&mut self, event: E) {
        self.deferred_events
            .push((TypeId::of::<E>(), Box::new(event)));
    }

    pub fn enqueue_event<E: Event>(&mut self, event: E) {
        self.defer(event);
    }

    pub fn queue_event<E: Event>(&mut self, event: E) {
        self.defer(event);
    }

    pub fn dispatch_later<E: Event>(&mut self, event: E) {
        self.defer(event);
    }

    pub fn flush_events(&mut self) {
        let events = std::mem::take(&mut self.deferred_events);
        for (type_id, mut payload) in events {
            if let Some(listeners) = self.listeners.get_mut(&type_id) {
                for l in listeners.iter_mut() {
                    let any: &mut dyn Any = &mut *payload;
                    l.handle(any);
                }
            }
        }
    }

    // fix flush events issue: need mutable payload retained; redo below by storing and processing differently is impossible here.
    // We'll instead process using indexed access.

    pub fn process_events(&mut self) {
        self.flush_events();
    }

    pub fn drain_events(&mut self) {
        self.flush_events();
    }

    // --- messages single consumer ---
    pub fn register_message<M, F>(&mut self, handler: F)
    where
        M: Message,
        F: FnMut(M) + Send + 'static,
    {
        use crate::message::FnMessageHandler;
        self.message_handlers
            .insert(TypeId::of::<M>(), Box::new(FnMessageHandler::new(handler)));
    }

    pub fn on_message<M, F>(&mut self, handler: F)
    where
        M: Message,
        F: FnMut(M) + Send + 'static,
    {
        self.register_message(handler);
    }

    pub fn subscribe_message<M, F>(&mut self, handler: F)
    where
        M: Message,
        F: FnMut(M) + Send + 'static,
    {
        self.register_message(handler);
    }

    pub fn add_message_handler<M, F>(&mut self, handler: F)
    where
        M: Message,
        F: FnMut(M) + Send + 'static,
    {
        self.register_message(handler);
    }

    pub fn send<M: Message>(&mut self, message: M) {
        self.message_queue.push_message(message);
    }

    pub fn post<M: Message>(&mut self, message: M) {
        self.send(message);
    }

    pub fn enqueue<M: Message>(&mut self, message: M) {
        self.send(message);
    }

    pub fn queue_message<M: Message>(&mut self, message: M) {
        self.send(message);
    }

    pub fn dispatch_message<M: Message>(&mut self, message: M) {
        if let Some(h) = self.message_handlers.get_mut(&TypeId::of::<M>()) {
            h.handle(Box::new(message));
        }
    }

    pub fn process_messages(&mut self) {
        for queued in self.message_queue.drain() {
            if let Some(h) = self.message_handlers.get_mut(&queued.type_id) {
                h.handle(queued.payload);
            }
        }
    }

    pub fn flush_messages(&mut self) {
        self.process_messages();
    }

    pub fn drain_messages(&mut self) {
        self.process_messages();
    }

    pub fn poll_messages(&mut self) {
        self.process_messages();
    }

    // --- global ---
    pub fn process(&mut self) {
        self.process_events();
        self.process_messages();
    }

    pub fn flush(&mut self) {
        self.process();
    }

    pub fn update(&mut self) {
        self.process();
    }

    pub fn message_queue(&mut self) -> &mut MessageQueue {
        &mut self.message_queue
    }
}

// Fix deferred event dispatch because previous flush moved payload incorrectly.
// Re-implement with shadow storage to avoid type move issues.
