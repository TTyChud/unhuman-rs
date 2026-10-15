use mirror::command_registry::{Command, CommandRegistry, Shortcut};
use mirror::eyes::Key;

#[test]
fn registry_rejects_drift() {
    let mut registry = CommandRegistry::new();
    registry
        .register(
            Command::new("translate")
                .label("Translate")
                .shortcut(Shortcut::key(Key::KeyW)),
        )
        .unwrap();
    let duplicate = registry.register(
        Command::new("translate2")
            .label("Duplicate")
            .shortcut(Shortcut::key(Key::KeyW)),
    );
    assert!(duplicate.is_err());
    assert!(!registry.has_drift() || registry.validate().is_ok());
    let mut bad = CommandRegistry::new();
    bad.register(Command::new("a").label("A")).unwrap();
    bad.register(Command::new("a").label("A2")).unwrap_err();
}

#[test]
fn lookup_is_consistent() {
    let mut registry = CommandRegistry::new();
    let cmd = Command::new("rotate")
        .label("Rotate")
        .shortcut(Shortcut::key(Key::KeyE));
    registry.register(cmd).unwrap();
    assert!(registry.get("rotate").is_some());
    assert!(registry.by_shortcut(Shortcut::key(Key::KeyE)).is_some());
    assert!(registry.no_drift());
}
