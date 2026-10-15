//! Undo and redo as first-class mutation commands.

/// A reversible mutation.
pub trait UndoCommand<T> {
    /// Apply the command in the redo direction.
    fn execute(&mut self, target: &mut T);
    /// Reverse the command.
    fn undo(&mut self, target: &mut T);
    /// Redo by re-running execute.
    fn redo(&mut self, target: &mut T) {
        self.execute(target);
    }
}

/// History of reversible mutations.
pub struct UndoHistory<T> {
    undo: Vec<Box<dyn UndoCommand<T>>>,
    redo: Vec<Box<dyn UndoCommand<T>>>,
}

impl<T> Default for UndoHistory<T> {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}

pub type UndoManager<T> = UndoHistory<T>;
pub type UndoStack<T> = UndoHistory<T>;
pub type RedoStack<T> = UndoHistory<T>;
pub type UndoRedo<T> = UndoHistory<T>;

impl<T> UndoHistory<T> {
    /// An empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute and record a command.
    pub fn execute<C>(&mut self, mut command: C, target: &mut T)
    where
        C: UndoCommand<T> + 'static,
    {
        command.execute(target);
        self.undo.push(Box::new(command));
        self.redo.clear();
    }

    /// Alias for [`Self::execute`].
    pub fn do_command<C>(&mut self, command: C, target: &mut T)
    where
        C: UndoCommand<T> + 'static,
    {
        self.execute(command, target);
    }

    /// Alias for [`Self::execute`].
    pub fn apply<C>(&mut self, command: C, target: &mut T)
    where
        C: UndoCommand<T> + 'static,
    {
        self.execute(command, target);
    }

    /// Alias for [`Self::execute`].
    pub fn commit<C>(&mut self, command: C, target: &mut T)
    where
        C: UndoCommand<T> + 'static,
    {
        self.execute(command, target);
    }

    /// Record a command that has already been applied.
    pub fn push<C>(&mut self, command: C)
    where
        C: UndoCommand<T> + 'static,
    {
        self.undo.push(Box::new(command));
        self.redo.clear();
    }

    /// Reverse the last command.
    pub fn undo(&mut self, target: &mut T) -> bool {
        if let Some(mut command) = self.undo.pop() {
            command.undo(target);
            self.redo.push(command);
            true
        } else {
            false
        }
    }

    /// Alias for [`Self::undo`].
    pub fn undo_one(&mut self, target: &mut T) -> bool {
        self.undo(target)
    }

    /// Alias for [`Self::undo`].
    pub fn rollback(&mut self, target: &mut T) -> bool {
        self.undo(target)
    }

    /// Alias for [`Self::undo`].
    pub fn revert(&mut self, target: &mut T) -> bool {
        self.undo(target)
    }

    /// Re-run the last undone command.
    pub fn redo(&mut self, target: &mut T) -> bool {
        if let Some(mut command) = self.redo.pop() {
            command.redo(target);
            self.undo.push(command);
            true
        } else {
            false
        }
    }

    /// Alias for [`Self::redo`].
    pub fn redo_one(&mut self, target: &mut T) -> bool {
        self.redo(target)
    }

    /// Whether undo is possible.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Alias for [`Self::can_undo`].
    pub fn can_undo_redo(&self) -> bool {
        self.can_undo()
    }

    /// Whether redo is possible.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Clear both stacks.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// How many undoable commands are recorded.
    pub fn len(&self) -> usize {
        self.undo.len()
    }

    /// Whether there is nothing to undo.
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// Undo stack length.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Redo stack length.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}
