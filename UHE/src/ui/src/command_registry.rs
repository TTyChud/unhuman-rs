//! One command registry for every menu, shortcut and palette entry.
//!
//! A command is declared once. The label, the shortcut hint shown in a menu and
//! the binding tested against the keyboard are all read from that declaration,
//! so they cannot drift apart the way `Q` becoming `1` collided with
//! `ImGuizmo::TRANSLATE`.

use eyes::{InputState, Key, Modifiers};

/// A keyboard shortcut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shortcut {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Shortcut {
    /// A shortcut with no modifiers.
    pub fn key(key: Key) -> Self {
        Self {
            modifiers: Modifiers::NONE,
            key,
        }
    }

    /// A shortcut with modifiers.
    pub fn with(modifiers: Modifiers, key: Key) -> Self {
        Self { modifiers, key }
    }

    /// Whether this shortcut fired this frame.
    pub fn matches(&self, input: &InputState) -> bool {
        input.binding_allowed()
            && input.modifiers() == self.modifiers
            && input.key_pressed(self.key)
    }
}

pub type Hotkey = Shortcut;
pub type KeyBinding = Shortcut;

/// One command.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub shortcut: Option<Shortcut>,
    pub menu_path: Option<String>,
    pub enabled: bool,
}

impl Command {
    /// Start a command with an id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            shortcut: None,
            menu_path: None,
            enabled: true,
        }
    }

    /// Set the human-readable label.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Set the shortcut.
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Set the menu path.
    pub fn menu(mut self, path: impl Into<String>) -> Self {
        self.menu_path = Some(path.into());
        self
    }

    /// Enable or disable the command.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Format the shortcut for a menu hint.
    pub fn shortcut_hint(&self) -> Option<String> {
        self.shortcut
            .as_ref()
            .map(|shortcut| format!("{:?}", shortcut.key))
    }
}

pub type Action = Command;

/// A command registry with no drift between its surfaces.
#[derive(Clone, Debug, Default)]
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command. Duplicates are rejected.
    pub fn register(&mut self, command: Command) -> Result<(), String> {
        if self
            .commands
            .iter()
            .any(|existing| existing.id == command.id)
        {
            return Err(format!("duplicate command id: {}", command.id));
        }
        if let Some(shortcut) = command.shortcut {
            if self
                .commands
                .iter()
                .any(|existing| existing.shortcut == Some(shortcut))
            {
                return Err(format!("duplicate shortcut: {:?}", shortcut.key));
            }
        }
        self.commands.push(command);
        Ok(())
    }

    /// Alias for [`Self::register`].
    pub fn add(&mut self, command: Command) -> Result<(), String> {
        self.register(command)
    }

    /// Alias for [`Self::register`].
    pub fn insert(&mut self, command: Command) -> Result<(), String> {
        self.register(command)
    }

    /// Look a command up by id.
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|command| command.id == id)
    }

    /// Alias for [`Self::get`].
    pub fn find(&self, id: &str) -> Option<&Command> {
        self.get(id)
    }

    /// Alias for [`Self::get`].
    pub fn find_by_id(&self, id: &str) -> Option<&Command> {
        self.get(id)
    }

    /// Find the command bound to a shortcut.
    pub fn by_shortcut(&self, shortcut: Shortcut) -> Option<&Command> {
        self.commands
            .iter()
            .find(|command| command.shortcut == Some(shortcut))
    }

    /// Alias for [`Self::by_shortcut`].
    pub fn find_shortcut(&self, shortcut: Shortcut) -> Option<&Command> {
        self.by_shortcut(shortcut)
    }

    /// Alias for [`Self::by_shortcut`].
    pub fn command_for_shortcut(&self, shortcut: Shortcut) -> Option<&Command> {
        self.by_shortcut(shortcut)
    }

    /// Every command.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Alias for [`Self::commands`].
    pub fn all(&self) -> &[Command] {
        self.commands()
    }

    /// Alias for [`Self::commands`].
    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.commands.iter()
    }

    /// Whether a shortcut is free.
    pub fn is_consistent(&self) -> bool {
        self.validate().is_ok()
    }

    /// Alias reporting the opposite of drift.
    pub fn no_drift(&self) -> bool {
        self.is_consistent()
    }

    /// Whether the registry has no drift.
    pub fn has_drift(&self) -> bool {
        !self.is_consistent()
    }

    /// Check every invariant.
    pub fn validate(&self) -> Result<(), String> {
        let mut ids: Vec<&str> = Vec::new();
        let mut shortcuts: Vec<Shortcut> = Vec::new();
        for command in &self.commands {
            if command.id.is_empty() {
                return Err("command with empty id".to_string());
            }
            if command.label.is_empty() {
                return Err(format!("command {} has no label", command.id));
            }
            if ids.contains(&command.id.as_str()) {
                return Err(format!("duplicate command id: {}", command.id));
            }
            ids.push(command.id.as_str());
            if let Some(shortcut) = command.shortcut {
                if shortcuts.contains(&shortcut) {
                    return Err(format!("duplicate shortcut: {:?}", shortcut.key));
                }
                shortcuts.push(shortcut);
            }
        }
        Ok(())
    }

    /// Alias for [`Self::validate`].
    pub fn check(&self) -> Result<(), String> {
        self.validate()
    }
}
