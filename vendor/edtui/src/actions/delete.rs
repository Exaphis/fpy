use jagged::index::RowIndex;

use super::Execute;
use crate::{
    clipboard::ClipboardTrait,
    helper::{is_out_of_bounds, max_col_insert},
    state::selection::Selection,
    vim::{
        motion as vim_motion,
        operator::{self as vim_operator, Operator},
        range::TextRange,
    },
    EditorState, Index2, Lines,
};

/// Deletes a character at the current cursor position. Does not
/// move the cursor position unless it is at the end of the line
/// Intended to be called in normal mode.
#[derive(Clone, Debug, Copy)]
pub struct RemoveChar(pub usize);

impl Execute for RemoveChar {
    fn execute(&mut self, state: &mut EditorState) {
        state.capture();
        state.clamp_column();
        for _ in 0..self.0 {
            let lines = &mut state.lines;
            let index = &mut state.cursor;

            if is_out_of_bounds(lines, index) {
                return;
            }

            let _ = lines.remove(*index);
            index.col = index.col.min(
                lines
                    .len_col(index.row)
                    .unwrap_or_default()
                    .saturating_sub(1),
            );
        }
    }
}

/// Replaces the character under the cursor with a given character.
/// Intended to be called in normal mode.
#[derive(Clone, Debug, Copy)]
pub struct ReplaceChar(pub char);

impl Execute for ReplaceChar {
    fn execute(&mut self, state: &mut EditorState) {
        let index = state.cursor;
        if is_out_of_bounds(&state.lines, &index) {
            return;
        }
        state.capture();
        if let Some(ch) = state.lines.get_mut(index) {
            *ch = self.0;
        };
    }
}

/// Deletes a character to the left of the current cursor. Deletes
/// the line break if the the cursor is in column zero.
/// Intended to be called in insert mode.
#[derive(Clone, Debug, Copy)]
pub struct DeleteChar(pub usize);

impl Execute for DeleteChar {
    fn execute(&mut self, state: &mut EditorState) {
        state.capture();
        for _ in 0..self.0 {
            delete_char(&mut state.lines, &mut state.cursor);
        }
    }
}

fn delete_char(lines: &mut Lines, index: &mut Index2) {
    fn move_left(lines: &Lines, index: &mut Index2) {
        if index.col > 0 {
            index.col -= 1;
        } else if index.row > 0 {
            index.row -= 1;
            index.col = lines.len_col(index.row).unwrap_or_default();
        }
    }

    let len_col = lines.len_col(index.row).unwrap_or_default();
    if len_col == 0 && index.row == 0 {
        return;
    }

    if index.col > len_col {
        index.col = len_col;
    }

    if index.col == 0 {
        let mut rest = lines.split_off(*index);
        move_left(lines, index);
        lines.merge(&mut rest);
    } else {
        let max_col = max_col_insert(lines, index);
        index.col = index.col.min(max_col);
        move_left(lines, index);
        let _ = lines.remove(*index);
    }
}

/// Deletes the character at the current cursor position.
/// If at the end of a line, deletes the newline character.
/// Intended to be called in insert mode.
#[derive(Clone, Debug, Copy)]
pub struct DeleteCharForward(pub usize);

impl Execute for DeleteCharForward {
    fn execute(&mut self, state: &mut EditorState) {
        state.capture();
        state.clamp_column();
        for _ in 0..self.0 {
            delete_char_forward(&mut state.lines, &mut state.cursor);
        }
    }
}

fn delete_char_forward(lines: &mut Lines, index: &mut Index2) {
    let Some(row) = lines.get(RowIndex::new(index.row)) else {
        return;
    };

    let row_len = row.len();

    // If cursor is at or past the end of the line, delete the newline
    if index.col >= row_len {
        if index.row + 1 >= lines.len() {
            return;
        }

        lines.join_lines(index.row);
        return;
    }

    let _ = lines.remove(*index);
}

/// Deletes from cursor to the end of the current word (Emacs Alt+d / Vim dw).
#[derive(Clone, Debug, Copy)]
pub struct DeleteWordForward(pub usize);

impl Execute for DeleteWordForward {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        state.capture();
        for _ in 0..self.0 {
            delete_word_forward(state);
        }
    }
}

fn delete_word_forward(state: &mut EditorState) {
    let Some(range) = vim_motion::word_forward_range(state) else {
        if state.cursor.row + 1 < state.lines.len() {
            state.lines.join_lines(state.cursor.row);
        }
        return;
    };
    vim_operator::apply_operator_without_capture(state, Operator::Delete, range);
}

macro_rules! range_action {
    ($name:ident, $range_fn:ident, $op:expr, counted) => {
        #[derive(Clone, Debug, Copy)]
        pub struct $name(pub usize);
        impl Execute for $name {
            fn execute(&mut self, state: &mut EditorState) {
                if matches!($op, Operator::Delete | Operator::Change) {
                    state.capture();
                }
                for _ in 0..self.0 {
                    if let Some(range) = vim_motion::$range_fn(state) {
                        vim_operator::apply_operator_without_capture(state, $op, range);
                    }
                }
            }
        }
    };
    ($name:ident, $range_fn:ident, $op:expr) => {
        #[derive(Clone, Debug, Copy)]
        pub struct $name(pub usize);
        impl Execute for $name {
            fn execute(&mut self, state: &mut EditorState) {
                if let Some(range) = vim_motion::$range_fn(state) {
                    vim_operator::apply_operator(state, $op, range);
                }
            }
        }
    };
}

range_action!(
    DeleteBigWordForward,
    big_word_forward_range,
    Operator::Delete,
    counted
);
range_action!(
    ChangeBigWordForward,
    big_word_forward_range,
    Operator::Change,
    counted
);
range_action!(CopyBigWordForward, big_word_forward_range, Operator::Yank);
range_action!(
    DeleteToBigWordEnd,
    big_word_end_range,
    Operator::Delete,
    counted
);
range_action!(
    ChangeToBigWordEnd,
    big_word_end_range,
    Operator::Change,
    counted
);
range_action!(CopyToBigWordEnd, big_word_end_range, Operator::Yank);
range_action!(
    DeleteBigWordBackward,
    big_word_backward_range,
    Operator::Delete,
    counted
);
range_action!(
    ChangeBigWordBackward,
    big_word_backward_range,
    Operator::Change,
    counted
);
range_action!(CopyBigWordBackward, big_word_backward_range, Operator::Yank);

#[derive(Clone, Debug, Copy)]
pub struct DeleteToWordEnd(pub usize);

impl Execute for DeleteToWordEnd {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        state.capture();
        for _ in 0..self.0 {
            delete_to_word_end(state);
        }
    }
}

fn delete_to_word_end(state: &mut EditorState) {
    if let Some(range) = vim_motion::word_end_range(state) {
        vim_operator::apply_operator_without_capture(state, Operator::Delete, range);
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeToWordEnd(pub usize);

impl Execute for ChangeToWordEnd {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        state.capture();
        for _ in 0..self.0 {
            if let Some(range) = vim_motion::word_end_range(state) {
                vim_operator::apply_operator_without_capture(state, Operator::Change, range);
            }
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyToWordEnd(pub usize);

impl Execute for CopyToWordEnd {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::word_end_range(state) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeWordForward(pub usize);

impl Execute for ChangeWordForward {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        state.capture();
        for _ in 0..self.0 {
            if let Some(range) = vim_motion::word_forward_range(state) {
                vim_operator::apply_operator_without_capture(state, Operator::Change, range);
            }
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyWordForward(pub usize);

impl Execute for CopyWordForward {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::word_forward_range(state) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

/// Deletes from cursor backward to start of previous word (Emacs Alt+Backspace / Vim db).
#[derive(Clone, Debug, Copy)]
pub struct DeleteWordBackward(pub usize);

impl Execute for DeleteWordBackward {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        state.capture();
        for _ in 0..self.0 {
            delete_word_backward(state);
        }
    }
}

fn delete_word_backward(state: &mut EditorState) {
    if let Some(range) = vim_motion::word_backward_range(state) {
        vim_operator::apply_operator_without_capture(state, Operator::Delete, range);
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeWordBackward(pub usize);

impl Execute for ChangeWordBackward {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        state.capture();
        for _ in 0..self.0 {
            if let Some(range) = vim_motion::word_backward_range(state) {
                vim_operator::apply_operator_without_capture(state, Operator::Change, range);
            }
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyWordBackward(pub usize);

impl Execute for CopyWordBackward {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::word_backward_range(state) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

/// Deletes the current line.
#[derive(Clone, Debug, Copy)]
pub struct DeleteLine(pub usize);

impl Execute for DeleteLine {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        let end = state.cursor.row.saturating_add(self.0.saturating_sub(1));
        vim_operator::apply_operator(
            state,
            Operator::Delete,
            TextRange::linewise(state.cursor.row, end),
        );
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeLine(pub usize);

impl Execute for ChangeLine {
    fn execute(&mut self, state: &mut EditorState) {
        if state.lines.is_empty() {
            return;
        }
        let start = state.cursor.row;
        let end = start.saturating_add(self.0.saturating_sub(1));
        vim_operator::apply_operator(state, Operator::Change, TextRange::linewise(start, end));
    }
}

#[derive(Clone, Debug, Copy)]
pub struct DeleteToStartOfLine;

impl Execute for DeleteToStartOfLine {
    fn execute(&mut self, state: &mut EditorState) {
        delete_to_start_of_line(state, true, false);
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeToStartOfLine;

impl Execute for ChangeToStartOfLine {
    fn execute(&mut self, state: &mut EditorState) {
        delete_to_start_of_line(state, true, true);
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyToStartOfLine;

impl Execute for CopyToStartOfLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_start_range(state) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

fn delete_to_start_of_line(state: &mut EditorState, capture: bool, insert: bool) {
    if let Some(range) = vim_motion::line_start_range(state) {
        if capture {
            vim_operator::apply_operator(
                state,
                if insert {
                    Operator::Change
                } else {
                    Operator::Delete
                },
                range,
            );
        } else {
            vim_operator::apply_operator_without_capture(
                state,
                if insert {
                    Operator::Change
                } else {
                    Operator::Delete
                },
                range,
            );
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct DeleteLineDown(pub usize);
impl Execute for DeleteLineDown {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_down_range(state, self.0) {
            vim_operator::apply_operator(state, Operator::Delete, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeLineDown(pub usize);
impl Execute for ChangeLineDown {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_down_range(state, self.0) {
            vim_operator::apply_operator(state, Operator::Change, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyLineDown(pub usize);
impl Execute for CopyLineDown {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_down_range(state, self.0) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct DeleteLineUp(pub usize);
impl Execute for DeleteLineUp {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_up_range(state, self.0) {
            vim_operator::apply_operator(state, Operator::Delete, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeLineUp(pub usize);
impl Execute for ChangeLineUp {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_up_range(state, self.0) {
            vim_operator::apply_operator(state, Operator::Change, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyLineUp(pub usize);
impl Execute for CopyLineUp {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_up_range(state, self.0) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct DeleteToLastLine;

impl Execute for DeleteToLastLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::to_last_line_range(state) {
            vim_operator::apply_operator(state, Operator::Delete, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeToLastLine;

impl Execute for ChangeToLastLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::to_last_line_range(state) {
            vim_operator::apply_operator(state, Operator::Change, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyToLastLine;

impl Execute for CopyToLastLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::to_last_line_range(state) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct DeleteToFirstLine;

impl Execute for DeleteToFirstLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::to_first_line_range(state) {
            vim_operator::apply_operator(state, Operator::Delete, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeToFirstLine;

impl Execute for ChangeToFirstLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::to_first_line_range(state) {
            vim_operator::apply_operator(state, Operator::Change, range);
        }
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyToFirstLine;

impl Execute for CopyToFirstLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::to_first_line_range(state) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

/// Deletes from the current cursor position to the first non-whitespace character of the line
#[derive(Clone, Debug, Copy)]
pub struct DeleteToFirstCharOfLine;

impl Execute for DeleteToFirstCharOfLine {
    fn execute(&mut self, state: &mut EditorState) {
        state.capture();

        let row_index = RowIndex::new(state.cursor.row);
        let Some(row) = state.lines.get_mut(row_index) else {
            return;
        };

        let col = state.cursor.col;

        let first_char = row
            .iter()
            .position(|c| !c.is_whitespace())
            .unwrap_or(row.len());

        let anchor = if col <= first_char { 0 } else { first_char };

        if anchor < col && col <= row.len() {
            let deleted = row.drain(anchor..col).collect();
            state.clip.set_text(deleted);
        }

        state.cursor.col = anchor;
    }
}

/// Deletes from the current cursor position to the end of the line
#[derive(Clone, Debug, Copy)]
pub struct DeleteToEndOfLine;

impl Execute for DeleteToEndOfLine {
    fn execute(&mut self, state: &mut EditorState) {
        delete_to_end_of_line(state, true, false);
    }
}

#[derive(Clone, Debug, Copy)]
pub struct ChangeToEndOfLine;

impl Execute for ChangeToEndOfLine {
    fn execute(&mut self, state: &mut EditorState) {
        delete_to_end_of_line(state, true, true);
    }
}

#[derive(Clone, Debug, Copy)]
pub struct CopyToEndOfLine;

impl Execute for CopyToEndOfLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(range) = vim_motion::line_end_range(state) {
            vim_operator::apply_operator(state, Operator::Yank, range);
        }
    }
}

fn delete_to_end_of_line(state: &mut EditorState, capture: bool, insert: bool) {
    if let Some(range) = vim_motion::line_end_range(state) {
        if capture {
            vim_operator::apply_operator(
                state,
                if insert {
                    Operator::Change
                } else {
                    Operator::Delete
                },
                range,
            );
        } else {
            vim_operator::apply_operator_without_capture(
                state,
                if insert {
                    Operator::Change
                } else {
                    Operator::Delete
                },
                range,
            );
        }
    }
}

pub(crate) fn delete_selection(state: &mut EditorState, selection: &Selection) -> Lines {
    state.cursor = selection.start();
    state.clamp_column();
    let extracted = selection.extract_from(&mut state.lines);
    clamp_cursor_to_buffer(state);
    extracted
}

fn clamp_cursor_to_buffer(state: &mut EditorState) {
    state.cursor.row = state.cursor.row.min(state.lines.len().saturating_sub(1));
    state.clamp_column();
}

/// Joins line below to the current line.
#[derive(Clone, Debug, Copy)]
pub struct JoinLineWithLineBelow;

impl Execute for JoinLineWithLineBelow {
    fn execute(&mut self, state: &mut EditorState) {
        if state.cursor.row + 1 >= state.lines.len() {
            return;
        }
        state.capture();
        state.lines.join_lines(state.cursor.row);
    }
}

#[cfg(test)]
mod tests {
    use crate::state::selection::Selection;
    use crate::EditorMode;
    use crate::Index2;
    use crate::Lines;

    use super::*;
    fn test_state() -> EditorState {
        EditorState::new(Lines::from("Hello World!\n\n123."))
    }

    #[test]
    fn test_remove_char() {
        let mut state = test_state();

        state.cursor = Index2::new(0, 4);
        RemoveChar(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 4));
        assert_eq!(state.lines, Lines::from("Hell World!\n\n123."));

        state.cursor = Index2::new(0, 10);
        RemoveChar(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 9));
        assert_eq!(state.lines, Lines::from("Hell World\n\n123."));
    }

    #[test]
    fn test_replace_char() {
        let mut state = test_state();

        state.cursor = Index2::new(0, 4);
        ReplaceChar('x').execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 4));
        assert_eq!(state.lines, Lines::from("Hellx World!\n\n123."));

        // do nothing on empty line
        state.cursor = Index2::new(1, 0);
        ReplaceChar('x').execute(&mut state);
        assert_eq!(state.cursor, Index2::new(1, 0));
        assert_eq!(state.lines, Lines::from("Hellx World!\n\n123."));

        // do nothing if out of bounds
        state.cursor = Index2::new(99, 0);
        ReplaceChar('x').execute(&mut state);
        assert_eq!(state.cursor, Index2::new(99, 0));
        assert_eq!(state.lines, Lines::from("Hellx World!\n\n123."));
    }

    #[test]
    fn test_delete_char() {
        let mut state = test_state();

        state.cursor = Index2::new(0, 5);
        DeleteChar(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 4));
        assert_eq!(state.lines, Lines::from("Hell World!\n\n123."));

        state.cursor = Index2::new(0, 11);
        DeleteChar(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 10));
        assert_eq!(state.lines, Lines::from("Hell World\n\n123."));
    }

    #[test]
    fn test_delete_char_empty_line() {
        let mut state = test_state();
        state.cursor = Index2::new(1, 99);

        DeleteChar(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 12));
        assert_eq!(state.lines, Lines::from("Hello World!\n123."));

        let mut state = EditorState::new(Lines::from("\nb"));
        state.cursor = Index2::new(0, 1);
        DeleteChar(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 1));
        assert_eq!(state.lines, Lines::from("\nb"));
    }

    #[test]
    fn test_delete_line() {
        let mut state = test_state();
        state.cursor = Index2::new(2, 3);

        DeleteLine(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(1, 0));
        assert_eq!(state.lines, Lines::from("Hello World!\n"));

        DeleteLine(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 0));
        assert_eq!(state.lines, Lines::from("Hello World!"));

        DeleteLine(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 0));
        assert_eq!(state.lines.to_string(), "");
        assert_eq!(state.lines.len(), 1);
    }

    #[test]
    fn test_delete_to_first_char_of_line() {
        let mut state = EditorState::new(Lines::from("  Hello World!"));
        state.cursor = Index2::new(0, 4);

        DeleteToFirstCharOfLine.execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 2));
        assert_eq!(state.lines, Lines::from("  llo World!"));

        state.cursor = Index2::new(0, 2);
        DeleteToFirstCharOfLine.execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 0));
        assert_eq!(state.lines, Lines::from("llo World!"));
    }

    #[test]
    fn test_delete_to_end_of_line() {
        let mut state = test_state();
        state.cursor = Index2::new(0, 3);

        DeleteToEndOfLine.execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 2));
        assert_eq!(state.lines, Lines::from("Hel\n\n123."));
    }

    #[test]
    fn test_delete_selection() {
        let mut state = test_state();
        let st = Index2::new(0, 1);
        let en = Index2::new(2, 0);
        let selection = Selection::new(st, en);

        delete_selection(&mut state, &selection);
        assert_eq!(state.cursor, Index2::new(0, 1));
        assert_eq!(state.lines, Lines::from("H23."));
    }

    #[test]
    fn test_delete_selection_out_of_bounds() {
        let mut state = EditorState::new(Lines::from("123.\nHello World!\n456."));
        let st = Index2::new(0, 5);
        let en = Index2::new(2, 10);
        let selection = Selection::new(st, en);

        delete_selection(&mut state, &selection);
        assert_eq!(state.cursor, Index2::new(0, 3));
        assert_eq!(state.lines, Lines::from("123."));
    }

    #[test]
    fn test_delete_line_selection_clamps_cursor_to_buffer_end() {
        let mut state = EditorState::new(Lines::from("one\ntwo\nthree"));
        let selection = Selection::new(Index2::new(1, 0), Index2::new(2, 4)).line_mode();

        delete_selection(&mut state, &selection);
        assert_eq!(state.cursor, Index2::new(0, 0));
        assert_eq!(state.lines, Lines::from("one"));
    }

    #[test]
    fn test_delete_char_forward() {
        let mut state = EditorState::new(Lines::from("Hello World!\nNext line"));
        state.mode = EditorMode::Insert;

        // Delete character 'H'
        state.cursor = Index2::new(0, 0);
        DeleteCharForward(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 0));
        assert_eq!(state.lines, Lines::from("ello World!\nNext line"));

        // Delete character 'e'
        state.cursor = Index2::new(0, 0);
        DeleteCharForward(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 0));
        assert_eq!(state.lines, Lines::from("llo World!\nNext line"));

        // Delete character at end of line (newline)
        state.cursor = Index2::new(0, 10);
        DeleteCharForward(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 10));
        assert_eq!(state.lines, Lines::from("llo World!Next line"));
    }

    #[test]
    fn test_delete_char_forward_at_end() {
        let mut state = EditorState::new(Lines::from("Hello\nWorld"));
        state.mode = EditorMode::Insert;

        // Cursor at end of first line should delete newline
        state.cursor = Index2::new(0, 5);
        DeleteCharForward(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 5));
        assert_eq!(state.lines, Lines::from("HelloWorld"));

        // Cursor at end of last line should do nothing
        state.cursor = Index2::new(0, 10);
        DeleteCharForward(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 10));
        assert_eq!(state.lines, Lines::from("HelloWorld"));
    }

    #[test]
    fn test_delete_word_forward() {
        let mut state = EditorState::new(Lines::from("Hello World Test"));
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 0);

        DeleteWordForward(1).execute(&mut state);
        assert_eq!(state.lines.to_string(), "World Test");
        assert_eq!(state.cursor, Index2::new(0, 0));

        DeleteWordForward(1).execute(&mut state);
        assert_eq!(state.lines.to_string(), "Test");
    }

    #[test]
    fn test_delete_word_forward_mid_word() {
        let mut state = EditorState::new(Lines::from("Hello World"));
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 2);

        DeleteWordForward(1).execute(&mut state);
        assert_eq!(state.lines.to_string(), "HeWorld");
    }

    #[test]
    fn test_delete_word_backward() {
        let mut state = EditorState::new(Lines::from("Hello World Test"));
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 12);

        DeleteWordBackward(1).execute(&mut state);
        assert_eq!(state.lines.to_string(), "Hello Test");
        assert_eq!(state.cursor, Index2::new(0, 6));
    }

    #[test]
    fn test_delete_word_backward_mid_word() {
        // On "o" of World, should only delete "W"
        let mut state = EditorState::new(Lines::from("Hello World"));
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 7);

        DeleteWordBackward(1).execute(&mut state);
        assert_eq!(state.lines.to_string(), "Hello orld");
        assert_eq!(state.cursor, Index2::new(0, 6));
    }

    #[test]
    fn test_delete_word_backward_at_word_start() {
        // On "W" of World, should delete "Hello " (previous word + whitespace)
        let mut state = EditorState::new(Lines::from("Hello World"));
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 6);

        DeleteWordBackward(1).execute(&mut state);
        assert_eq!(state.lines.to_string(), "World");
        assert_eq!(state.cursor, Index2::new(0, 0));
    }
}
