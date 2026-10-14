use chassis::{Buffer, Handle, Pipeline, Registry};
use std::collections::HashSet;

#[test]
fn create_and_get_roundtrips() {
    let mut registry: Registry<String> = Registry::new();
    let a = registry.create("a".into());
    assert_eq!(registry.get(a), Some(&String::from("a")));
    assert_eq!(registry.len(), 1);
}

#[test]
fn remove_invalidates_the_old_handle() {
    let mut registry: Registry<String> = Registry::new();
    let a = registry.create("a".into());
    assert_eq!(registry.remove(a), Some(String::from("a")));
    assert_eq!(registry.get(a), None, "freed handle still resolved");
    assert!(!registry.contains(a));
}

#[test]
fn slot_reuse_does_not_resurrect_the_old_handle() {
    let mut registry: Registry<u32> = Registry::new();

    let old = registry.create(1);
    registry.remove(old);
    let new = registry.create(2);

    assert_eq!(old.index(), new.index(), "slot should have been reused");
    assert_ne!(old.generation(), new.generation(), "generation must bump");
    assert_eq!(
        registry.get(old),
        None,
        "stale handle aliased the new value"
    );
    assert_eq!(registry.get(new), Some(&2));
    assert_eq!(registry.len(), 1);
}

#[test]
fn generations_survive_many_reuse_cycles() {
    let mut registry: Registry<u32> = Registry::new();
    let mut stale = Vec::new();

    for value in 0..1_000u32 {
        let handle = registry.create(value);
        stale.push(handle);
        assert_eq!(registry.get(handle), Some(&value));
        registry.remove(handle);
    }

    assert_eq!(registry.len(), 0);
    for handle in stale {
        assert_eq!(registry.get(handle), None, "a freed handle resolved");
    }
    assert_eq!(registry.capacity(), 1, "free list should reuse one slot");
}

#[test]
fn free_list_reuses_slots() {
    let mut registry: Registry<u32> = Registry::new();
    let handles: Vec<Handle<u32>> = (0..100).map(|i| registry.create(i)).collect();
    assert_eq!(registry.capacity(), 100);

    for handle in &handles {
        registry.remove(*handle);
    }
    assert_eq!(registry.len(), 0);

    for i in 0..100u32 {
        registry.create(i);
    }
    assert_eq!(registry.capacity(), 100, "freed slots were not reused");
}

#[test]
fn handles_are_distinct_per_resource_type() {
    let mut buffers: Registry<u32, chassis::BufferTag> = Registry::new();
    let mut pipelines: Registry<u32, chassis::PipelineTag> = Registry::new();

    // Both registries hand out index 0, but the types cannot be confused: a
    // `Buffer` will not typecheck where a `Pipeline` is expected. This is the
    // class of bug the C++ `void*` handles allowed silently
    // (`RHITypes.h:37-41`).
    let buffer: Buffer = buffers.create(7);
    let pipeline: Pipeline = pipelines.create(9);

    assert_eq!(buffers.get(buffer), Some(&7));
    assert_eq!(pipelines.get(pipeline), Some(&9));
}

#[test]
fn iter_yields_only_live_entries() {
    let mut registry: Registry<u32> = Registry::new();
    let a = registry.create(1);
    let b = registry.create(2);
    let c = registry.create(3);
    registry.remove(b);

    let live: HashSet<u32> = registry.iter().map(|(h, v)| (h, *v).1).collect();
    assert_eq!(live, HashSet::from([1, 3]));
    assert!(registry.contains(a));
    assert!(!registry.contains(b));
    assert!(registry.contains(c));
}

#[test]
fn handles_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Buffer>();
    assert_send_sync::<Handle<u32>>();
}

#[test]
fn generations_start_at_one_so_a_default_handle_is_never_valid() {
    let mut registry: Registry<u32> = Registry::new();
    let handle = registry.create(1);
    assert_eq!(handle.index(), 0);
    assert_eq!(handle.generation(), 1);
}
