use pings::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
struct Ping {
    value: i32,
}

impl Event for Ping {}

#[derive(Clone, Debug, PartialEq)]
struct Note {
    text: String,
}

impl Message for Note {}

#[test]
fn immediate_event_broadcasts() {
    let mut d = Dispatcher::new();
    let hits = Arc::new(Mutex::new(0i32));
    let h = Arc::clone(&hits);
    d.listen::<Ping, _>(move |_e| {
        *h.lock().unwrap() += 1;
    });
    let h2 = Arc::clone(&hits);
    d.listen::<Ping, _>(move |_e| {
        *h2.lock().unwrap() += 10;
    });
    d.dispatch(Ping { value: 1 });
    assert_eq!(*hits.lock().unwrap(), 11);
}

#[test]
fn strongly_typed_event() {
    #[derive(Clone)]
    struct Other;
    impl Event for Other {}
    let mut d = Dispatcher::new();
    let seen = Arc::new(Mutex::new(false));
    let s = Arc::clone(&seen);
    d.listen::<Ping, _>(move |_| *s.lock().unwrap() = true);
    d.dispatch(Other);
    assert!(!*seen.lock().unwrap());
    d.dispatch(Ping { value: 5 });
    assert!(*seen.lock().unwrap());
}

#[test]
fn deferred_dispatch() {
    let mut d = Dispatcher::new();
    let seen = Arc::new(Mutex::new(0i32));
    let s = Arc::clone(&seen);
    d.listen::<Ping, _>(move |e| *s.lock().unwrap() = e.value);
    d.defer(Ping { value: 7 });
    assert_eq!(*seen.lock().unwrap(), 0);
    d.process();
    assert_eq!(*seen.lock().unwrap(), 7);
}

#[test]
fn message_single_consumer_deferred() {
    let mut d = Dispatcher::new();
    let seen = Arc::new(Mutex::new(String::new()));
    let s = Arc::clone(&seen);
    d.register_message::<Note, _>(move |n| *s.lock().unwrap() = n.text);
    d.send(Note { text: "hi".into() });
    d.update();
    assert_eq!(*seen.lock().unwrap(), "hi");
}

#[test]
fn message_queue() {
    let mut q = MessageQueue::new();
    q.push_message(Note { text: "x".into() });
    assert_eq!(q.len(), 1);
    let m = q.pop().unwrap();
    assert_eq!(m.type_id, TypeId::of::<Note>());
}

#[test]
fn generic_queue() {
    let mut q = Queue::new();
    q.push(1);
    q.push(2);
    assert_eq!(q.pop(), Some(1));
    assert_eq!(q.drain(), vec![2]);
}

#[test]
fn observer() {
    let mut obs = EcsObserver::<u64>::new();
    let seen = Arc::new(Mutex::new(0u64));
    let s = Arc::clone(&seen);
    #[derive(Clone)]
    struct Comp {
        v: u64,
    }
    obs.on_changed::<Comp, _>(move |entity, c| {
        *s.lock().unwrap() = entity + c.v;
    });
    let mut c = Comp { v: 3 };
    obs.notify_changed(10, &mut c);
    assert_eq!(*seen.lock().unwrap(), 13);
}

#[test]
fn aliases() {
    let mut d = Dispatcher::default();
    d.emit(Ping { value: 2 });
    d.flush();
    // immediate handler none, okay
}

#[test]
fn subscribe_bool_handler() {
    let mut d = Dispatcher::new();
    let seen = Arc::new(Mutex::new(false));
    let s = Arc::clone(&seen);
    d.subscribe::<Ping, _>(move |e| {
        *s.lock().unwrap() = e.value == 9;
        true
    });
    d.dispatch(Ping { value: 9 });
    assert!(*seen.lock().unwrap());
}
