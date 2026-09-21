use crate::document::Note;

pub struct UndoStack {
    past: Vec<Note>,
    future: Vec<Note>,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
        }
    }
}

impl UndoStack {
    pub fn push(&mut self, before: &Note) {
        self.past.push(before.clone());
        if self.past.len() > 60 {
            self.past.remove(0);
        }
        self.future.clear();
    }

    pub fn undo(&mut self, current: &mut Note) -> bool {
        let Some(prev) = self.past.pop() else {
            return false;
        };
        self.future.push(current.clone());
        *current = prev;
        true
    }

    pub fn redo(&mut self, current: &mut Note) -> bool {
        let Some(next) = self.future.pop() else {
            return false;
        };
        self.past.push(current.clone());
        *current = next;
        true
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }
}
