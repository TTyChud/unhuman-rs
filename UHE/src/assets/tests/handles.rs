//! Generational ids: a stale id is refused, and a reused slot is a new id.

use std::fs;
use std::path::PathBuf;

use vault::{AssetKind, Assets, Handle, Image};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("vault-handles-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        Self { root }
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, contents).expect("scratch file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_released_slot_is_never_reused_without_bumping_its_generation() {
    let scratch = Scratch::new("generation");
    scratch.write("a.txt", "hello");

    let mut assets = Assets::new(&scratch.root);
    let first = assets.load("a.txt").expect("a.txt loads");
    assert!(assets.contains(first));
    assert_eq!(assets.text(first), Some("hello"));

    assert!(assets.release(first));
    assert!(!assets.contains(first));
    assert_eq!(assets.text(first), None);
    assert_eq!(assets.bytes(first), None);
    assert!(assets.handle::<str>(first).is_none());
    // A stale release is refused rather than freeing the slot twice.
    assert!(!assets.release(first));
    assert!(assets.is_empty());

    // The very next allocation reuses the freed index, which is exactly the
    // case the generation exists for.
    let second = assets.load("a.txt").expect("a.txt loads again");
    assert_eq!(second.index(), first.index());
    assert_eq!(second.generation(), first.generation() + 1);
    assert_ne!(second, first);

    // The stale id must not resolve to the asset that took its slot.
    assert!(!assets.contains(first));
    assert_eq!(assets.bytes(first), None);
    assert!(assets.mesh(first).is_none());
    assert_eq!(assets.text(second), Some("hello"));
    assert_eq!(assets.len(), 1);
}

#[test]
fn the_null_id_names_nothing() {
    let scratch = Scratch::new("null");
    let mut assets = Assets::new(&scratch.root);

    assert!(vault::AssetId::NULL.is_null());
    assert_eq!(assets.bytes(vault::AssetId::NULL), None);
    assert!(!assets.release(vault::AssetId::NULL));
    assert!(assets.handle::<str>(vault::AssetId::NULL).is_none());
    assert_eq!(vault::AssetId::default(), vault::AssetId::NULL);
}

#[test]
fn a_second_load_of_the_same_path_shares_the_first_id() {
    let scratch = Scratch::new("dedup");
    scratch.write("shared.txt", "one copy");
    scratch.write("other.txt", "another");

    let mut assets = Assets::new(&scratch.root);
    let first = assets.load("shared.txt").expect("shared.txt loads");
    let again = assets.load("shared.txt").expect("shared.txt loads again");
    let other = assets.load("other.txt").expect("other.txt loads");

    assert_eq!(first, again);
    assert_ne!(first, other);
    assert_eq!(assets.len(), 2);
    assert_eq!(assets.find("shared.txt"), Some(first));
    assert_eq!(assets.path(first), Some("shared.txt"));
    assert_eq!(assets.kind(first), Some(AssetKind::Text));
    assert_eq!(assets.find("never-loaded.txt"), None);
}

#[test]
fn a_typed_handle_only_resolves_its_own_kind() {
    let scratch = Scratch::new("kinds");
    scratch.write("notes.txt", "text");

    let mut assets = Assets::new(&scratch.root);
    let id = assets.load("notes.txt").expect("notes.txt loads");

    let text: Handle<str> = assets.handle(id).expect("a text asset has a text handle");
    assert_eq!(text.get(&assets), Some("text"));
    assert_eq!(text.id(), id);

    let bytes: Handle<[u8]> = assets.handle(id).expect("every asset has bytes");
    assert_eq!(bytes.get(&assets), Some(b"text".as_slice()));

    // The same id cannot be a handle to a payload it does not hold.
    assert!(assets.handle::<Image>(id).is_none());
    assert!(assets.handle::<vault::MeshData>(id).is_none());
}

#[test]
fn a_fabricated_id_resolves_to_nothing() {
    let scratch = Scratch::new("fabricated");
    scratch.write("a.txt", "a");

    let mut assets = Assets::new(&scratch.root);
    let id = assets.load("a.txt").expect("a.txt loads");

    // Same index, wrong generation - the shape a stale handle has.
    let stale = vault::AssetId::from_parts(id.index(), id.generation() + 1);
    assert!(!assets.contains(stale));
    assert_eq!(assets.bytes(stale), None);

    // An index past the end of the table is refused, not an out-of-bounds read.
    let out_of_range = vault::AssetId::from_parts(u32::MAX - 1, 1);
    assert!(!assets.contains(out_of_range));
    assert!(!assets.release(out_of_range));
}
