//! This module contains a "brute-force" implementation of undo and redo.
//! It stores the entire editor state at each action.
//!
//! This approach works for basic undo/redo needs, but its not the most efficient.
//! In the long run, this should be replaced with an action-based mechanism.
use crate::{EditorState, Index2, Lines};

#[derive(Debug, Clone)]
pub(crate) struct Stack {
    inner: Vec<UndoState>,
    max_size: usize,
}

impl Stack {
    pub(crate) fn new() -> Self {
        Self {
            inner: Vec::new(),
            max_size: 100,
        }
    }

    pub(crate) fn pop(&mut self) -> Option<UndoState> {
        self.inner.pop()
    }

    pub(crate) fn peek(&self) -> Option<&UndoState> {
        self.inner.last()
    }

    pub(crate) fn push(&mut self, value: UndoState) {
        self.inner.push(value);
        if self.len() > self.max_size {
            self.remove(0);
        }
    }

    fn len(&mut self) -> usize {
        self.inner.len()
    }

    fn remove(&mut self, index: usize) {
        self.inner.remove(index);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UndoState {
    lines: Lines,
    cursor: Index2,
}

impl EditorState {
    pub fn begin_undo_transaction(&mut self) {
        if self.undo_transaction_depth == 0 {
            self.undo_transaction_captured = false;
        }
        self.undo_transaction_depth = self.undo_transaction_depth.saturating_add(1);
    }

    pub fn end_undo_transaction(&mut self) {
        self.undo_transaction_depth = self.undo_transaction_depth.saturating_sub(1);
        if self.undo_transaction_depth == 0 {
            self.undo_transaction_captured = false;
        }
    }

    pub(crate) fn capture(&mut self) {
        if self.undo_transaction_depth > 0 && self.undo_transaction_captured {
            return;
        }
        let editor_state = UndoState {
            lines: self.lines.clone(),
            cursor: self.cursor,
        };
        self.undo.push(editor_state);
        self.redo = Stack::new();
        if self.undo_transaction_depth > 0 {
            self.undo_transaction_captured = true;
        }
    }

    pub(crate) fn discard_redundant_undo_top(&mut self) {
        if self
            .undo
            .peek()
            .is_some_and(|prev| prev.lines.to_string() == self.lines.to_string())
        {
            let _ = self.undo.pop();
        }
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let current = UndoState {
                lines: self.lines.clone(),
                cursor: self.cursor,
            };
            self.lines = prev.lines;
            self.cursor = prev.cursor;
            self.cursor.row = self
                .cursor
                .row
                .min(self.lines.iter_row().count().saturating_sub(1));
            self.cursor.col = self
                .cursor
                .col
                .min(self.lines.len_col(self.cursor.row).unwrap_or_default().saturating_sub(1));
            self.redo.push(current);
        }
    }

    pub fn redo(&mut self) {
        if let Some(prev) = self.redo.pop() {
            let current = UndoState {
                lines: self.lines.clone(),
                cursor: self.cursor,
            };
            self.lines = prev.lines;
            self.cursor = prev.cursor;
            self.undo.push(current);
        }
    }
}
