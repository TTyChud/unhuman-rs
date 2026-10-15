use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::Event;

type Handler = Box<dyn FnMut(&Event) + Send + 'static>;

struct Subscriber {
    alive: Arc<AtomicBool>,
    handler: Handler,
}

/// Events waiting for the frame, and the handlers watching them as they arrive.
///
/// The C++ `Application::OnEvent` (`Core/Application.cpp:83`) walks the layer
/// stack until a layer sets `Handled`; a subscription decides for itself whether
/// it cares, so the flag and the category bitmask it was tested with have no
/// counterpart here.
#[derive(Default)]
pub struct EventBus {
    events: VecDeque<Event>,
    subscribers: Vec<Subscriber>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues an event and hands it to every live subscriber, in the order they
    /// subscribed.
    pub fn push(&mut self, event: Event) {
        self.subscribers.retain(|s| s.alive.load(Ordering::Acquire));
        for subscriber in &mut self.subscribers {
            (subscriber.handler)(&event);
        }
        self.events.push_back(event);
    }

    /// Hands the frame's events over, oldest first, and empties the queue.
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }

    pub fn subscribe(&mut self, f: impl FnMut(&Event) + Send + 'static) -> Subscription {
        let alive = Arc::new(AtomicBool::new(true));
        self.subscribers.push(Subscriber {
            alive: Arc::clone(&alive),
            handler: Box::new(f),
        });
        Subscription { alive }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Stops its handler seeing events once dropped. The flag is set here and the
/// handler is dropped at the next push, so a subscription can be dropped from a
/// place that has no access to the bus.
pub struct Subscription {
    alive: Arc<AtomicBool>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}
