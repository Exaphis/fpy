use jagged::index::RowIndex;

use super::Execute;
use crate::{
    helper::{insert_char, line_break},
    EditorState,
};

const PYTHON_INDENT_WIDTH: usize = 4;

/// Inserts a single character at the current cursor position.
///
/// In single-line mode, newline characters (`\n`, `\r`) are ignored.
#[derive(Clone, Debug, Copy)]
pub struct InsertChar(pub char);

impl Execute for InsertChar {
    fn execute(&mut self, state: &mut EditorState) {
        // Block newline characters in single-line mode
        if state.view.single_line && matches!(self.0, '\n' | '\r') {
            return;
        }
        insert_char(&mut state.lines, &mut state.cursor, self.0, false);
        if state.vim_change_from_leading_whitespace {
            remove_following_whitespace(state);
            state.vim_change_from_leading_whitespace = false;
        }
        reindent_python_block_start(state);
    }
}

/// Inserts a newline at the current cursor position.
///
/// In single-line mode, this action is ignored.
#[derive(Clone, Debug, Copy)]
pub struct LineBreak(pub usize);

impl Execute for LineBreak {
    fn execute(&mut self, state: &mut EditorState) {
        if state.view.single_line {
            return;
        }
        if state.lines.is_empty() {
            state.lines.push(Vec::new());
        }
        for _ in 0..self.0 {
            let indent = python_indent_after_line_break(state);
            line_break(&mut state.lines, &mut state.cursor);
            remove_split_leading_whitespace(state);
            insert_indent(state, indent);
        }
    }
}

/// Appends a newline below the current cursor position.
///
/// In single-line mode, this action is ignored.
#[derive(Clone, Debug, Copy)]
pub struct AppendNewline(pub usize);

impl Execute for AppendNewline {
    fn execute(&mut self, state: &mut EditorState) {
        if state.view.single_line {
            return;
        }
        state.capture();
        state.cursor.col = 0;
        if state.lines.is_empty() {
            state.lines.push(vec![]);
        }
        for _ in 0..self.0 {
            let indent = python_indent_for_current_line(state);
            if !state.lines.is_empty() {
                state.cursor.row += 1;
            }
            if state.cursor.row < state.lines.len() {
                state.lines.insert(RowIndex::new(state.cursor.row), vec![]);
            } else {
                state.lines.push(vec![]);
            }
            insert_indent(state, indent);
        }
    }
}

/// Appends a newline at the current cursor position.
///
/// In single-line mode, this action is ignored.
#[derive(Clone, Debug, Copy)]
pub struct InsertNewline(pub usize);

impl Execute for InsertNewline {
    fn execute(&mut self, state: &mut EditorState) {
        if state.view.single_line {
            return;
        }
        if state.lines.is_empty() {
            state.lines.push(Vec::new());
        }
        state.cursor.col = 0;
        for _ in 0..self.0 {
            let indent = if state.cursor.row > 0 {
                let block_indent = python_indent_for_new_block_above(state, state.cursor.row - 1);
                if block_indent > 0 {
                    block_indent
                } else {
                    current_line_indent(state)
                }
            } else {
                0
            };
            state.lines.insert(RowIndex::new(state.cursor.row), vec![]);
            insert_indent(state, indent);
        }
    }
}

fn python_indent_after_line_break(state: &EditorState) -> usize {
    let Some(row) = state.lines.iter_row().nth(state.cursor.row) else {
        return 0;
    };
    let before_cursor: String = row.iter().take(state.cursor.col).collect();
    python_indent_after_text(&before_cursor)
}

fn remove_following_whitespace_before_text(state: &mut EditorState) {
    let has_text_after_spaces = state
        .lines
        .iter_row()
        .nth(state.cursor.row)
        .is_some_and(|row| {
            row.iter()
                .skip(state.cursor.col)
                .skip_while(|ch| ch.is_ascii_whitespace())
                .next()
                .is_some()
        });
    if has_text_after_spaces {
        remove_following_whitespace(state);
    }
}

fn remove_following_whitespace(state: &mut EditorState) {
    while state
        .lines
        .get(state.cursor)
        .is_some_and(|ch| ch.is_ascii_whitespace())
    {
        state.lines.remove(state.cursor);
    }
}

fn remove_split_leading_whitespace(state: &mut EditorState) {
    while state
        .lines
        .get(state.cursor)
        .is_some_and(|ch| ch.is_ascii_whitespace())
    {
        state.lines.remove(state.cursor);
    }
}

fn reindent_python_block_start(state: &mut EditorState) {
    let Some(row) = state.lines.iter_row().nth(state.cursor.row) else {
        return;
    };
    let text: String = row.iter().collect();
    let leading = text
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count();
    let trimmed = text.trim_start();
    let starts_block = trimmed.starts_with("def ") || trimmed.starts_with("class ");
    let contains_block = starts_block || line_contains_python_block_keyword(&text, leading);
    if !contains_block {
        return;
    }
    let desired = if starts_block && leading == 0 {
        0
    } else if starts_block && state.cursor.row > 0 {
        let block_indent = python_indent_for_new_block_above(state, state.cursor.row - 1);
        if block_indent > 0 { block_indent } else { leading }
    } else if starts_block && state.cursor.row == 0 {
        0
    } else if contains_block && leading >= PYTHON_INDENT_WIDTH && state.cursor.row > 0 {
        let block_indent = python_indent_for_new_block_above(state, state.cursor.row - 1);
        if block_indent > 0 { block_indent } else { leading }
    } else if contains_block && state.cursor.row == 0 {
        0
    } else if starts_block || leading >= PYTHON_INDENT_WIDTH {
        leading
    } else {
        0
    };
    if leading == desired {
        return;
    }
    if leading > desired {
        for _ in 0..(leading - desired) {
            state.lines.remove(crate::Index2::new(state.cursor.row, 0));
            state.cursor.col = state.cursor.col.saturating_sub(1);
        }
    } else {
        let to_add = desired - leading;
        let saved = state.cursor;
        state.cursor.col = 0;
        for _ in 0..to_add {
            insert_char(&mut state.lines, &mut state.cursor, ' ', false);
        }
        state.cursor = crate::Index2::new(saved.row, saved.col + to_add);
    }
    if starts_block {
        remove_following_whitespace_before_text(state);
    }
}

fn line_contains_python_block_keyword(text: &str, leading: usize) -> bool {
    find_python_block_keyword(text).is_some_and(|idx| {
        if idx <= leading {
            return true;
        }
        let prefix = &text[..idx];
        let non_indent_prefix = prefix.trim_start_matches([' ', '\t']);
        !non_indent_prefix.contains("   ") && text[idx..].contains("):")
    })
}

fn find_python_block_keyword(text: &str) -> Option<usize> {
    let mut best = None;
    for keyword in ["def ", "class "] {
        if let Some(idx) = text.find(keyword) {
            best = Some(best.map_or(idx, |best: usize| best.min(idx)));
        }
    }
    best
}

fn current_line_indent(state: &EditorState) -> usize {
    state
        .lines
        .iter_row()
        .nth(state.cursor.row)
        .map(|row| {
            row.iter()
                .take_while(|ch| **ch == ' ' || **ch == '\t')
                .count()
        })
        .unwrap_or(0)
}

fn python_indent_for_new_block_above(state: &EditorState, row_index: usize) -> usize {
    let Some(row) = state.lines.iter_row().nth(row_index) else {
        return 0;
    };
    let text: String = row.iter().collect();
    unmatched_bracket_indent(&text)
        .or_else(|| python_block_opener_indent(&text))
        .unwrap_or(0)
}

fn python_indent_for_current_line(state: &EditorState) -> usize {
    python_indent_for_row(state, state.cursor.row)
}

fn python_indent_for_row(state: &EditorState, row_index: usize) -> usize {
    let Some(row) = state.lines.iter_row().nth(row_index) else {
        return 0;
    };
    let text: String = row.iter().collect();
    python_indent_after_text(&text)
}

fn python_block_opener_indent(text: &str) -> Option<usize> {
    let trimmed = text.trim_start();
    let is_block_keyword = [
        "def ", "class ", "if ", "elif ", "else", "for ", "while ", "try", "except", "finally",
        "with ", "match ", "case ", "async def ", "async for ", "async with ",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix) || trimmed.contains(prefix));
    (is_block_keyword && trimmed.contains(':')).then(|| {
        text.chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .count()
            + PYTHON_INDENT_WIDTH
    })
}

fn python_indent_after_text(text: &str) -> usize {
    let base = text
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count();
    if let Some(paren_indent) = unmatched_bracket_indent(text) {
        paren_indent
    } else if text.trim_end().ends_with(':') {
        base + PYTHON_INDENT_WIDTH
    } else {
        base
    }
}

fn unmatched_bracket_indent(text: &str) -> Option<usize> {
    let mut stack = Vec::new();
    for (col, ch) in text.chars().enumerate() {
        match ch {
            '(' | '[' | '{' => stack.push((ch, col)),
            ')' | ']' | '}' => {
                let expected = match ch {
                    ')' => '(',
                    ']' => '[',
                    '}' => '{',
                    _ => unreachable!(),
                };
                if stack.last().is_some_and(|(open, _)| *open == expected) {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    stack.last().map(|(_, col)| col + 1)
}

fn insert_indent(state: &mut EditorState, indent: usize) {
    for _ in 0..indent {
        insert_char(&mut state.lines, &mut state.cursor, ' ', false);
    }
}

/// Pushes a line to the back of the buffer.
/// Does not affect the cursor position.
#[derive(Clone, Debug, Copy)]
pub struct PushLine<'a>(pub &'a str);

impl Execute for PushLine<'_> {
    fn execute(&mut self, state: &mut EditorState) {
        let chars: Vec<char> = self.0.chars().collect();
        state.lines.push(chars);
    }
}

#[cfg(test)]
mod tests {
    use crate::{Index2, Lines};

    use super::*;
    fn test_state() -> EditorState {
        EditorState::new(Lines::from("Hello World!\n\n123."))
    }

    #[test]
    fn test_insert_char() {
        let mut state = test_state();

        InsertChar('!').execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 1));
        assert_eq!(state.lines, Lines::from("!Hello World!\n\n123."));

        state.cursor = Index2::new(0, 13);
        InsertChar('!').execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 14));
        assert_eq!(state.lines, Lines::from("!Hello World!!\n\n123."));
    }

    #[test]
    fn test_insert_char_into_empty_buffer() {
        let mut state = EditorState::new(Lines::from("\n"));
        state.cursor.row = 1;

        InsertChar('a').execute(&mut state);
        assert_eq!(state.cursor, Index2::new(1, 1));
        assert_eq!(state.lines, Lines::from("\na"));
    }

    #[test]
    fn test_insert_char_out_of_bounds() {
        let mut state = EditorState::new(Lines::from("\nb"));
        state.cursor = Index2::new(0, 1);

        InsertChar('a').execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 1));
        assert_eq!(state.lines, Lines::from("a\nb"));
    }

    #[test]
    fn test_line_break() {
        let mut state = test_state();

        LineBreak(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(1, 0));
        assert_eq!(state.lines, Lines::from("\nHello World!\n\n123."));

        state.cursor = Index2::new(1, 5);
        LineBreak(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(2, 0));
        assert_eq!(state.lines, Lines::from("\nHello\n World!\n\n123."));
    }

    #[test]
    fn test_line_break_col_out_of_bounds() {
        let mut state = test_state();
        state.cursor.col = 99;

        LineBreak(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(1, 0));
        assert_eq!(state.lines, Lines::from("Hello World!\n\n\n123."));

        state.cursor.col = 99;
        state.cursor.row = 4;
        LineBreak(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(5, 0));
        assert_eq!(state.lines, Lines::from("Hello World!\n\n\n123.\n"));
    }

    #[test]
    fn test_append_newline() {
        let mut state = test_state();

        AppendNewline(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(1, 0));
        assert_eq!(state.lines, Lines::from("Hello World!\n\n\n123."));

        state.cursor = Index2::new(3, 0);
        AppendNewline(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(4, 0));
        assert_eq!(state.lines, Lines::from("Hello World!\n\n\n123.\n"));
    }

    #[test]
    fn test_insert_newline() {
        let mut state = test_state();

        InsertNewline(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(0, 0));
        assert_eq!(state.lines, Lines::from("\nHello World!\n\n123."));

        state.cursor = Index2::new(2, 1);
        InsertNewline(1).execute(&mut state);
        assert_eq!(state.cursor, Index2::new(2, 0));
        assert_eq!(state.lines, Lines::from("\nHello World!\n\n\n123."));
    }

    #[test]
    fn test_push_line() {
        let mut state = test_state();

        PushLine("456.").execute(&mut state);
        assert_eq!(state.lines, Lines::from("Hello World!\n\n123.\n456."));
    }
}
