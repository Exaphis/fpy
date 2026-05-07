use jagged::index::{Index2, RowIndex};

use crate::{clipboard::ClipboardTrait, EditorMode, EditorState, Lines};

use super::range::{RangeKind, TextRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
}

pub(crate) fn apply_operator(state: &mut EditorState, operator: Operator, range: TextRange) {
    apply_operator_with_capture(state, operator, range, true);
}

pub(crate) fn apply_operator_without_capture(
    state: &mut EditorState,
    operator: Operator,
    range: TextRange,
) {
    apply_operator_with_capture(state, operator, range, false);
}

pub(crate) fn delete_char(state: &mut EditorState, count: usize) {
    let Some(range) = super::motion::char_span_range(state, count) else {
        if state.lines.iter_row().any(|row| !row.is_empty()) || !state.vim_last_yank_linewise {
            state.capture();
        }
        return;
    };
    if let Some(anchor) = state.vim_undo_cursor_anchor.filter(|anchor| {
        anchor.row == state.cursor.row
            && anchor.col <= state.cursor.col
            && only_whitespace_between(state, *anchor, state.cursor)
    }) {
        state.capture_with_cursor(anchor);
        apply_operator_without_capture(state, Operator::Delete, range);
        state.vim_undo_cursor_anchor = None;
    } else {
        apply_operator(state, Operator::Delete, range);
    }
}

fn only_whitespace_between(state: &EditorState, start: Index2, end: Index2) -> bool {
    if start == end {
        return true;
    }
    (start.col..end.col).all(|col| {
        state
            .lines
            .get(Index2::new(start.row, col))
            .is_none_or(|ch| ch.is_ascii_whitespace())
    })
}

pub(crate) fn delete_to_end_of_line(state: &mut EditorState) {
    let Some(range) = super::motion::line_end_range(state) else {
        return;
    };
    if range.start == range.end {
        return;
    }
    apply_operator(state, Operator::Delete, range);
}

pub(crate) fn change_to_end_of_line(state: &mut EditorState) {
    let Some(range) = super::motion::line_end_range(state) else {
        state.mode = EditorMode::Insert;
        return;
    };
    apply_operator(state, Operator::Change, range);
}

pub(crate) fn paste_after(state: &mut EditorState) {
    let mut text = state.clip.get_text();
    if text.is_empty() && !state.lines.iter_row().any(|row| !row.is_empty()) {
        text = "\n".to_string();
        state.vim_last_yank_linewise = true;
    }
    if text.is_empty() {
        return;
    }
    state.preferred_col = None;
    state.capture();
    if state.vim_last_yank_linewise {
        let mut rows: Vec<String> = state
            .lines
            .iter_row()
            .map(|row| row.iter().collect::<String>())
            .collect();
        if rows.is_empty() {
            rows.push(String::new());
        }
        let insert_at = (state.cursor.row + 1).min(rows.len());
        let empty_buffer = !rows.iter().any(|row| !row.is_empty());
        let pasted_text = text.trim_start_matches('\n');
        let mut pasted_rows: Vec<String> = if pasted_text.is_empty() {
            vec![String::new()]
        } else {
            pasted_text.split('\n').map(str::to_string).collect()
        };
        if !empty_buffer
            && pasted_text.len() > 1
            && pasted_rows.last().is_some_and(|row| row.is_empty())
        {
            pasted_rows.pop();
        }
        rows.splice(insert_at..insert_at, pasted_rows);
        state.lines = Lines::default();
        for row in rows {
            state.lines.push(row.chars().collect::<Vec<_>>());
        }
        state.cursor.row = insert_at.min(state.lines.iter_row().count().saturating_sub(1));
        state.cursor.col = 0;
        return;
    }

    if !state.lines.iter_row().any(|row| !row.is_empty()) && text.contains('\n') {
        state.lines = Lines::from(format!("\n{text}"));
        state.cursor.row = 1.min(state.lines.iter_row().count().saturating_sub(1));
        state.cursor.col = 0;
        return;
    }

    let row = state.cursor.row;
    let col = (state.cursor.col + 1).min(state.lines.len_col(row).unwrap_or_default());
    let mut rows: Vec<String> = state
        .lines
        .iter_row()
        .map(|row| row.iter().collect::<String>())
        .collect();
    if rows.is_empty() {
        rows.push(String::new());
    }
    if let Some(line) = rows.get_mut(row) {
        line.insert_str(col, &text);
        state.lines = Lines::from(rows.join("\n"));
        state.cursor.row = row;
        state.cursor.col = col + text.chars().count().saturating_sub(1);
        clamp_cursor(state);
    }
}

pub(crate) fn join_line_with_line_below(state: &mut EditorState) {
    let mut rows: Vec<String> = state
        .lines
        .iter_row()
        .map(|row| row.iter().collect::<String>())
        .collect();
    if state.cursor.row + 1 >= rows.len() {
        return;
    }

    state.preferred_col = None;
    state.capture();

    let row = state.cursor.row;
    let left_had_trailing_whitespace = rows[row]
        .chars()
        .last()
        .is_some_and(|ch| ch.is_ascii_whitespace());
    let left = rows[row].trim_end().to_string();
    let right = rows.remove(row + 1).trim_start().to_string();
    let join_col = left.chars().count();
    let joined_with_space = !left.is_empty() && !right.is_empty();
    rows[row] = if joined_with_space {
        format!("{left} {right}")
    } else {
        format!("{left}{right}")
    };
    state.lines = Lines::default();
    for row in rows {
        state.lines.push(row.chars().collect::<Vec<_>>());
    }
    let cursor_col = join_col + usize::from(joined_with_space && left_had_trailing_whitespace);
    state.cursor.col = cursor_col.min(
        state
            .lines
            .len_col(row)
            .unwrap_or_default()
            .saturating_sub(1),
    );
}

fn apply_operator_with_capture(
    state: &mut EditorState,
    operator: Operator,
    range: TextRange,
    capture: bool,
) {
    match operator {
        Operator::Yank => yank_range(state, range),
        Operator::Indent => shift_lines(state, range, ShiftDirection::Right, capture),
        Operator::Outdent => shift_lines(state, range, ShiftDirection::Left, capture),
        Operator::Delete | Operator::Change if range.kind == RangeKind::Linewise => {
            apply_linewise_edit(state, operator, range, capture);
        }
        Operator::Delete | Operator::Change => {
            if capture {
                let undo_cursor = if operator == Operator::Delete {
                    range.start
                } else {
                    state.cursor
                };
                state.capture_with_cursor(undo_cursor);
            }
            let yanked = extract_range(state, range);
            state.clip.set_text(lines_to_text(&yanked));
            state.vim_last_yank_linewise = false;
            if operator == Operator::Change {
                state.cursor = Index2::new(
                    range.start.row,
                    range
                        .start
                        .col
                        .min(state.lines.len_col(range.start.row).unwrap_or_default()),
                );
                state.vim_undo_cursor_anchor = Some(state.cursor);
                state.mode = EditorMode::Insert;
            } else {
                clamp_cursor(state);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShiftDirection {
    Left,
    Right,
}

fn shift_lines(state: &mut EditorState, range: TextRange, direction: ShiftDirection, capture: bool) {
    let row_count = state.lines.iter_row().count();
    if row_count == 0 {
        return;
    }
    let start_row = range.start.row.min(row_count.saturating_sub(1));
    let end_row = range.end.row.min(row_count.saturating_sub(1));
    if capture && shift_would_change(state, start_row, end_row, direction) {
        state.capture_with_cursor(state.cursor);
    }
    for row_index in start_row..=end_row {
        let Some(row) = state.lines.get_mut(RowIndex::new(row_index)) else {
            continue;
        };
        match direction {
            ShiftDirection::Right => {
                if row.is_empty() {
                    continue;
                }
                row.splice(0..0, [' ', ' ', ' ', ' ']);
            }
            ShiftDirection::Left => {
                if row.first() == Some(&'\t') {
                    row.remove(0);
                } else {
                    let remove_count = row.iter().take(4).take_while(|ch| **ch == ' ').count();
                    for _ in 0..remove_count {
                        row.remove(0);
                    }
                }
            }
        }
    }
    state.cursor.row = start_row;
    clamp_cursor(state);
    state.preferred_col = None;
}

fn shift_would_change(
    state: &EditorState,
    start_row: usize,
    end_row: usize,
    direction: ShiftDirection,
) -> bool {
    (start_row..=end_row).any(|row_index| {
        let Some(row) = state.lines.get(RowIndex::new(row_index)) else {
            return false;
        };
        match direction {
            ShiftDirection::Right => !row.is_empty(),
            ShiftDirection::Left => row.first() == Some(&'\t') || row.first() == Some(&' '),
        }
    })
}

fn yank_range(state: &mut EditorState, range: TextRange) {
    let text = copy_range(&state.lines, range);
    if !text.is_empty() {
        state.clip.set_text(text);
        state.vim_last_yank_linewise = range.kind == RangeKind::Linewise;
    }
}

fn apply_linewise_edit(
    state: &mut EditorState,
    operator: Operator,
    range: TextRange,
    capture: bool,
) {
    if state.lines.iter_row().count() == 1
        && state.lines.len_col(0).unwrap_or_default() == 0
    {
        if !state.vim_last_yank_linewise || state.clip.get_text().is_empty() {
            state.clip.set_text("\n".to_string());
            state.vim_last_yank_linewise = true;
        }
        return;
    }
    let yanked_text = copy_linewise(&state.lines, range.start.row, range.end.row);
    if capture {
        capture_linewise_undo_state(state, range);
    }
    let cursor_col_before_delete = if state.mode == EditorMode::Visual {
        0
    } else {
        state.cursor.col
    };
    let _ = extract_linewise(state, range.start.row, range.end.row);
    state.clip.set_text(yanked_text);
    state.vim_last_yank_linewise = true;
    place_cursor_after_linewise_edit(state, range.start.row, cursor_col_before_delete);
    if operator == Operator::Change {
        state.mode = EditorMode::Insert;
    }
}

fn capture_linewise_undo_state(state: &mut EditorState, _range: TextRange) {
    state.capture_with_cursor(state.cursor);
}

fn place_cursor_after_linewise_edit(
    state: &mut EditorState,
    deleted_start_row: usize,
    cursor_col_before_delete: usize,
) {
    state.cursor.row = deleted_start_row.min(state.lines.len().saturating_sub(1));
    state.cursor.col = cursor_col_before_delete;
    clamp_cursor(state);
}

fn copy_range(lines: &Lines, range: TextRange) -> String {
    match range.kind {
        RangeKind::Linewise => copy_linewise(lines, range.start.row, range.end.row),
        RangeKind::Exclusive | RangeKind::Inclusive => {
            let mut end = range.end;
            if range.kind == RangeKind::Inclusive {
                end.col = end.col.saturating_add(1);
            }
            lines.copy_range(range.start..end).to_string()
        }
    }
}

fn extract_range(state: &mut EditorState, range: TextRange) -> Lines {
    match range.kind {
        RangeKind::Linewise => extract_linewise(state, range.start.row, range.end.row),
        RangeKind::Exclusive | RangeKind::Inclusive => {
            let mut end = range.end;
            if range.kind == RangeKind::Inclusive {
                end.col = end.col.saturating_add(1);
            }
            if range.start.row == end.row
                && range.start.col == 0
                && end.col >= state.lines.len_col(range.start.row).unwrap_or_default()
                && state.lines.iter_row().count() > 1
            {
                let text = state
                    .lines
                    .iter_row()
                    .nth(range.start.row)
                    .map(|row| row.iter().collect::<String>())
                    .unwrap_or_default();
                if let Some(row) = state.lines.get_mut(jagged::index::RowIndex::new(range.start.row)) {
                    row.clear();
                }
                state.cursor = range.start;
                return Lines::from(text);
            }
            let text = state.lines.extract(range.start..end);
            state.cursor = range.start;
            text
        }
    }
}

fn lines_to_text(lines: &Lines) -> String {
    lines
        .iter_row()
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn copy_linewise(lines: &Lines, start_row: usize, end_row: usize) -> String {
    if lines.is_empty() || start_row >= lines.len() {
        return String::new();
    }
    let end_row = end_row.min(lines.len().saturating_sub(1));
    let mut text = String::new();
    for row in start_row..=end_row {
        if let Some(line) = lines.get(RowIndex::new(row)) {
            text.push('\n');
            text.extend(line.iter());
        }
    }
    text
}

fn extract_linewise(state: &mut EditorState, start_row: usize, end_row: usize) -> Lines {
    let text = copy_linewise(&state.lines, start_row, end_row);
    if state.lines.is_empty() || start_row >= state.lines.len() {
        return Lines::from(text);
    }
    let end_row = end_row.min(state.lines.len().saturating_sub(1));
    for _ in start_row..=end_row {
        if start_row >= state.lines.len() {
            break;
        }
        state.lines.remove(RowIndex::new(start_row));
    }
    if state.lines.is_empty() {
        state.lines.push(Vec::<char>::new());
    }
    Lines::from(text)
}

fn clamp_cursor(state: &mut EditorState) {
    state.cursor.row = state.cursor.row.min(state.lines.len().saturating_sub(1));
    state.clamp_column();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        clipboard::{ClipboardTrait, InternalClipboard},
        Index2,
    };

    #[test]
    fn applies_characterwise_delete_yank_and_change() {
        let mut state = EditorState::new(Lines::from("one two"));
        state.set_clipboard(InternalClipboard::default());
        let range = TextRange::exclusive(Index2::new(0, 0), Index2::new(0, 4));

        apply_operator(&mut state, Operator::Yank, range);
        assert_eq!(state.lines.to_string(), "one two");
        assert_eq!(state.clip.get_text(), "one ");

        apply_operator(&mut state, Operator::Delete, range);
        assert_eq!(state.lines.to_string(), "two");
        assert_eq!(state.clip.get_text(), "one ");

        state.undo();
        apply_operator(&mut state, Operator::Change, range);
        assert_eq!(state.lines.to_string(), "two");
        assert_eq!(state.mode, EditorMode::Insert);
    }
}
