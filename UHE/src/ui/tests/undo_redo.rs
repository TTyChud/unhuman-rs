use mirror::undo::{UndoCommand, UndoHistory};

struct Counter(i32);
struct Increment(i32);

impl UndoCommand<Counter> for Increment {
    fn execute(&mut self, target: &mut Counter) {
        target.0 += self.0;
    }
    fn undo(&mut self, target: &mut Counter) {
        target.0 -= self.0;
    }
}

#[test]
fn undo_redo_roundtrip() {
    let mut history = UndoHistory::new();
    let mut counter = Counter(0);
    history.execute(Increment(5), &mut counter);
    assert_eq!(counter.0, 5);
    assert!(history.undo(&mut counter));
    assert_eq!(counter.0, 0);
    assert!(history.redo(&mut counter));
    assert_eq!(counter.0, 5);
}
