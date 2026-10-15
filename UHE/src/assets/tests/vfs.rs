//! The virtual file system: search order, escape refusal, missing paths.

use std::fs;
use std::path::{Path, PathBuf};

use vault::{Error, Vfs};

/// A scratch directory that only this test uses, so a parallel run cannot
/// collide with another suite's files.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("vault-vfs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        Self { root }
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).expect("scratch subdirectory");
        path
    }

    fn write(&self, dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).expect("scratch file");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_first_root_that_holds_the_path_wins() {
    let scratch = Scratch::new("order");
    let first = scratch.dir("first");
    let second = scratch.dir("second");
    scratch.write(&first, "shared.txt", "first");
    scratch.write(&second, "shared.txt", "second");
    scratch.write(&second, "only-second.txt", "second only");

    let mut vfs = Vfs::new();
    vfs.mount(&first);
    vfs.mount(&second);

    let resolved = vfs.resolve("shared.txt").expect("shared.txt resolves");
    assert_eq!(fs::read_to_string(resolved).expect("readable"), "first");

    // A path only the later root holds still resolves: the list is a search
    // order, not a single root.
    let resolved = vfs
        .resolve("only-second.txt")
        .expect("only-second.txt resolves");
    assert_eq!(fs::read_to_string(resolved).expect("readable"), "second only");

    // Mount order is the override order, and `resolve_all` reports both.
    let all = vfs.resolve_all("shared.txt").expect("path is well formed");
    assert_eq!(all.len(), 2);
}

#[test]
fn mounting_the_same_root_twice_does_not_change_resolution() {
    let scratch = Scratch::new("duplicate");
    let root = scratch.dir("root");
    scratch.write(&root, "a.txt", "a");

    let mut vfs = Vfs::new();
    vfs.mount(&root);
    vfs.mount(&root);

    assert_eq!(vfs.roots().len(), 1);
    assert!(vfs.resolve("a.txt").is_ok());
}

#[test]
fn subdirectories_resolve_with_either_separator() {
    let scratch = Scratch::new("subdirs");
    let root = scratch.dir("root");
    let nested = scratch.dir("root/textures");
    scratch.write(&nested, "wall.png", "not really a png");

    let mut vfs = Vfs::new();
    vfs.mount(&root);

    assert!(vfs.resolve("textures/wall.png").is_ok());
    assert!(vfs.resolve("/textures/wall.png").is_ok());
    assert!(vfs.resolve("./textures//wall.png").is_ok());
    assert!(vfs.resolve("textures\\wall.png").is_ok());
}

#[test]
fn a_path_that_escapes_its_root_is_refused() {
    let scratch = Scratch::new("escape");
    let root = scratch.dir("root");
    // A file one level above the root, which a traversal would reach.
    scratch.write(&scratch.root.clone(), "secret.txt", "secret");

    let mut vfs = Vfs::new();
    vfs.mount(&root);

    assert!(matches!(
        vfs.resolve("../secret.txt"),
        Err(Error::EscapesRoot(_))
    ));
    assert!(matches!(
        vfs.resolve("textures/../../secret.txt"),
        Err(Error::EscapesRoot(_))
    ));
    // A Windows drive-qualified path would become absolute under `join`.
    assert!(matches!(
        vfs.resolve("C:/secret.txt"),
        Err(Error::EscapesRoot(_))
    ));
}

#[test]
fn a_missing_path_is_an_error_and_not_a_panic() {
    let scratch = Scratch::new("missing");
    let root = scratch.dir("root");
    scratch.write(&root, "present.txt", "here");

    let mut vfs = Vfs::new();
    vfs.mount(&root);

    let error = vfs.resolve("absent.txt").expect_err("absent.txt is absent");
    assert!(matches!(error, Error::NotFound(path) if path == "absent.txt"));
    assert!(matches!(vfs.resolve("."), Err(Error::NotFound(_))));
    assert!(matches!(vfs.resolve(""), Err(Error::NotFound(_))));
}

#[test]
fn an_empty_search_path_resolves_nothing() {
    let vfs = Vfs::new();
    assert!(vfs.is_empty());
    assert!(matches!(vfs.resolve("anything.txt"), Err(Error::NotFound(_))));
}
