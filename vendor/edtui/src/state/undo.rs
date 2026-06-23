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
    entries: Vec<UndoEntry>,
    redo_cursor_override: Option<Index2>,
}

#[derive(Debug, Clone)]
struct UndoEntry {
    top: usize,
    old_size: usize,
    new_size: usize,
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
            self.refresh_current_undo_entries();
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
        self.capture_with_cursor_and_span(cursor, cursor.row.saturating_sub(1), cursor.row + 1);
    }

    pub(crate) fn capture_with_cursor_and_span(&mut self, cursor: Index2, top: usize, bot: usize) {
        if self.undo_transaction_depth > 0 && self.undo_transaction_captured {
            return;
        }
        let pending_cursor = self.vim_pending_undo_cursor.take();
        let cursor = pending_cursor.unwrap_or(cursor);
        let lines = self.lines.clone();
        let line_count = lines.iter_row().count();
        let top = pending_cursor
            .map_or(top, |pending| top.min(pending.row))
            .min(line_count);
        let bot = bot.min(line_count + 1).max(top + 1);
        let editor_state = UndoState {
            lines: lines.clone(),
            cursor,
            entries: vec![UndoEntry {
                top,
                old_size: bot.saturating_sub(top + 1),
                new_size: bot.saturating_sub(top + 1),
            }],
            redo_cursor_override: None,
        };
        self.undo.push(editor_state);
        self.redo = Stack::new();
        if self.undo_transaction_depth > 0 {
            self.undo_transaction_captured = true;
        }
    }

    fn refresh_current_undo_entries(&mut self) {
        if let Some(prev) = self.undo.inner.last_mut() {
            let mut entries = undo_entries_for_snapshots(&prev.lines, &self.lines);
            if let (Some(existing), Some(updated)) = (prev.entries.first(), entries.first_mut()) {
                if existing.top < updated.top {
                    let delta = updated.top - existing.top;
                    updated.top = existing.top;
                    updated.old_size += delta;
                    updated.new_size += delta;
                }
            }
            prev.entries = entries;
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

    pub(crate) fn discard_undo_top(&mut self) {
        let _ = self.undo.pop();
    }

    pub(crate) fn replace_undo_top_with_current(&mut self) {
        if let Some(prev) = self.undo.inner.last_mut() {
            prev.entries = undo_entries_for_snapshots(&prev.lines, &self.lines);
            prev.lines = self.lines.clone();
            prev.cursor = self.cursor;
            prev.redo_cursor_override = None;
        }
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let current_lines = self.lines.clone();
            let mut current_cursor = prev
                .redo_cursor_override
                .or_else(|| redo_cursor_for_state(&prev.lines, &current_lines))
                .unwrap_or(self.cursor);
            let restore_insert_start = prev.cursor.col == 0
                && is_insert_at_cursor(&prev.lines, &current_lines, prev.cursor);
            let restore_insert_block_start = prev.entries.first().is_some_and(|entry| {
                current_lines.iter_row().count() > prev.lines.iter_row().count()
                    && prev.cursor.row <= entry.top + entry.new_size
            });
            if restore_insert_start || restore_insert_block_start {
                current_cursor = prev.cursor;
            }
            let changed_cursor =
                undo_redo_changed_cursor(&current_lines, &prev.lines, prev.cursor, &prev.entries);
            let current = UndoState {
                entries: undo_entries_for_snapshots(&prev.lines, &current_lines),
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
            let current_lines = self.lines.clone();
            let current_cursor =
                first_changed_position(&current_lines, &prev.lines).unwrap_or(self.cursor);
            let current = UndoState {
                entries: undo_entries_for_snapshots(&prev.lines, &current_lines),
                lines: current_lines,
                cursor: current_cursor,
                redo_cursor_override: None,
            };
            let restore_insert_start =
                prev.cursor.col == 0 && is_insert_at_cursor(&self.lines, &prev.lines, prev.cursor);
            let changed_cursor =
                undo_redo_changed_cursor(&self.lines, &prev.lines, prev.cursor, &prev.entries);
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

fn first_changed_position(before: &Lines, after: &Lines) -> Option<Index2> {
    for row in 0..before.iter_row().count().min(after.iter_row().count()) {
        let before_row = before.iter_row().nth(row)?;
        let after_row = after.iter_row().nth(row)?;
        let max = before_row.len().min(after_row.len());
        for col in 0..max {
            if before_row[col] != after_row[col] {
                return Some(Index2::new(row, col));
            }
        }
        if before_row.len() != after_row.len() {
            return Some(Index2::new(row, max.saturating_sub(1)));
        }
    }
    None
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

fn undo_entries_for_snapshots(before: &Lines, after: &Lines) -> Vec<UndoEntry> {
    let before_rows: Vec<String> = before.iter_row().map(|row| row.iter().collect()).collect();
    let after_rows: Vec<String> = after.iter_row().map(|row| row.iter().collect()).collect();
    let mut top = 0;
    while top < before_rows.len().min(after_rows.len()) && before_rows[top] == after_rows[top] {
        top += 1;
    }
    if top == before_rows.len() && top == after_rows.len() {
        return Vec::new();
    }
    let mut before_tail = before_rows.len();
    let mut after_tail = after_rows.len();
    while before_tail > top
        && after_tail > top
        && before_rows[before_tail - 1] == after_rows[after_tail - 1]
    {
        before_tail -= 1;
        after_tail -= 1;
    }
    vec![UndoEntry {
        top,
        old_size: before_tail.saturating_sub(top),
        new_size: after_tail.saturating_sub(top),
    }]
}

fn undo_redo_changed_cursor(
    before: &Lines,
    after: &Lines,
    saved_cursor: Index2,
    entries: &[UndoEntry],
) -> Index2 {
    let inferred;
    let entries = if entries.is_empty() {
        inferred = undo_entries_for_snapshots(before, after);
        inferred.as_slice()
    } else {
        entries
    };
    let mut new_curpos = saved_cursor;
    let mut newlnum = usize::MAX;
    for entry in entries {
        let top = entry.top;
        let newsize = entry.new_size;
        let oldsize = entry.old_size;
        if saved_cursor.row >= top && saved_cursor.row <= top + newsize {
            new_curpos = saved_cursor;
            newlnum = usize::MAX - 1;
        } else if top < newlnum {
            if newsize == 0 && newlnum == usize::MAX {
                newlnum = top;
                new_curpos = Index2::new(newlnum, 0);
            } else if oldsize != newsize || newsize > 0 {
                newlnum = top;
                new_curpos = Index2::new(newlnum, 0);
            }
        }
    }
    new_curpos
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
    cursor.col = cursor.col.min(
        lines
            .len_col(cursor.row)
            .unwrap_or_default()
            .saturating_sub(1),
    );
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
