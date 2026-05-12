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
    redo_cursor_override: Option<Index2>,
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
        let cursor = self
            .vim_insert_capture_cursor_override
            .take()
            .unwrap_or(self.cursor);
        self.capture_with_cursor(cursor);
    }

    pub(crate) fn capture_with_cursor(&mut self, cursor: Index2) {
        if self.undo_transaction_depth > 0 && self.undo_transaction_captured {
            return;
        }
        let editor_state = UndoState {
            lines: self.lines.clone(),
            cursor,
            redo_cursor_override: None,
        };
        self.undo.push(editor_state);
        self.redo = Stack::new();
        if self.undo_transaction_depth > 0 {
            self.undo_transaction_captured = true;
        }
    }

    pub(crate) fn set_redo_cursor_override(&mut self, cursor: Index2) {
        if let Some(prev) = self.undo.inner.last_mut() {
            prev.redo_cursor_override = Some(cursor);
        }
    }

    pub(crate) fn discard_redundant_undo_top(&mut self) {
        if self.undo.peek().is_some_and(|prev| {
            prev.lines.to_string() == self.lines.to_string() && prev.cursor == self.cursor
        }) {
            let _ = self.undo.pop();
        }
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let current_lines = self.lines.clone();
            let mut current_cursor = prev
                .redo_cursor_override
                .or_else(|| redo_cursor_for_state(&prev.lines, &current_lines))
                .unwrap_or(self.cursor);
            let restore_insert_start =
                prev.cursor.col == 0 && is_insert_at_cursor(&prev.lines, &current_lines, prev.cursor);
            if restore_insert_start {
                current_cursor = prev.cursor;
            }
            let changed_cursor = redo_cursor_for_state(&current_lines, &prev.lines).unwrap_or(prev.cursor);
            let current = UndoState {
                lines: current_lines,
                cursor: current_cursor,
                redo_cursor_override: None,
            };
            self.lines = prev.lines;
            self.cursor = if restore_insert_start {
                prev.cursor
            } else {
                vim_undoredo_cursor(prev.cursor, changed_cursor, &self.lines)
            };
            self.redo.push(current);
        }
    }

    pub fn redo(&mut self) {
        let Some(prev) = self.redo.pop() else {
            self.preferred_col = Some(self.cursor.col);
            return;
        };
        {
            let current = UndoState {
                lines: self.lines.clone(),
                cursor: self.cursor,
                redo_cursor_override: None,
            };
            let restore_insert_start =
                prev.cursor.col == 0 && is_insert_at_cursor(&self.lines, &prev.lines, prev.cursor);
            let changed_cursor = redo_cursor_for_state(&self.lines, &prev.lines).unwrap_or(prev.cursor);
            self.lines = prev.lines;
            self.cursor = if restore_insert_start {
                prev.cursor
            } else {
                vim_undoredo_cursor(prev.cursor, changed_cursor, &self.lines)
            };
            self.undo.push(current);
        }
    }
}

fn is_insert_at_cursor(before: &Lines, after: &Lines, cursor: Index2) -> bool {
    let Some(before_row) = before.iter_row().nth(cursor.row) else {
        return false;
    };
    let Some(after_row) = after.iter_row().nth(cursor.row) else {
        return false;
    };
    after_row.len() == before_row.len() + 1
        && after_row[..cursor.col] == before_row[..cursor.col]
        && after_row[cursor.col + 1..] == before_row[cursor.col..]
}

fn vim_undoredo_cursor(saved_cursor: Index2, changed_cursor: Index2, lines: &Lines) -> Index2 {
    let mut cursor = changed_cursor;
    if saved_cursor.row + 1 == cursor.row && cursor.row > 0 {
        cursor.row -= 1;
    }
    let row_count = lines.iter_row().count();
    if row_count == 0 {
        return Index2::new(0, 0);
    }
    cursor.row = cursor.row.min(row_count.saturating_sub(1));
    if saved_cursor.row == cursor.row {
        cursor.col = saved_cursor.col;
    } else {
        cursor.col = lines
            .iter_row()
            .nth(cursor.row)
            .and_then(|row| row.iter().position(|ch| !ch.is_ascii_whitespace()))
            .unwrap_or(0);
    }
    cursor.col = cursor
        .col
        .min(lines.len_col(cursor.row).unwrap_or_default().saturating_sub(1));
    cursor
}

fn redo_cursor_for_state(before: &Lines, after: &Lines) -> Option<Index2> {
    let before_rows: Vec<Vec<char>> = before.iter_row().map(|row| row.to_vec()).collect();
    for (row_index, after_row) in after.iter_row().enumerate() {
        let Some(before_row) = before_rows.get(row_index) else {
            return Some(Index2::new(row_index, 0));
        };
        for (col_index, after_char) in after_row.iter().enumerate() {
            if before_row.get(col_index) != Some(after_char) {
                if col_index > 0 && after_row[col_index..] == before_row[col_index - 1..] {
                    return Some(Index2::new(row_index, col_index - 1));
                }
                return Some(Index2::new(row_index, col_index));
            }
        }
        if after_row.len() != before_row.len() {
            let col = if after_row.len() < before_row.len() {
                after_row.len().saturating_sub(1)
            } else {
                before_row.len()
            };
            return Some(Index2::new(row_index, col));
        }
    }
    None
}
