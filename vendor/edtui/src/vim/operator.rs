use jagged::index::{Index2, RowIndex};

use crate::{clipboard::ClipboardTrait, EditorMode, EditorState, Lines};

use super::range::{RangeKind, TextRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Operator {
    Delete,
    Change,
    Yank,
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
        state.capture();
        return;
    };
    apply_operator(state, Operator::Delete, range);
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
        Operator::Delete | Operator::Change if range.kind == RangeKind::Linewise => {
            apply_linewise_edit(state, operator, range, capture);
        }
        Operator::Delete | Operator::Change => {
            if capture {
                state.capture();
            }
            let yanked = extract_range(state, range);
            state.clip.set_text(yanked.to_string());
            if operator == Operator::Change {
                state.cursor = Index2::new(
                    range.start.row,
                    range
                        .start
                        .col
                        .min(state.lines.len_col(range.start.row).unwrap_or_default()),
                );
                state.mode = EditorMode::Insert;
            } else {
                clamp_cursor(state);
            }
        }
    }
}

fn yank_range(state: &mut EditorState, range: TextRange) {
    let text = copy_range(&state.lines, range);
    if !text.is_empty() {
        state.clip.set_text(text);
    }
}

fn apply_linewise_edit(
    state: &mut EditorState,
    operator: Operator,
    range: TextRange,
    capture: bool,
) {
    if capture {
        capture_linewise_undo_state(state, range);
    }
    let yanked = extract_linewise(state, range.start.row, range.end.row);
    state.clip.set_text(yanked.to_string());
    place_cursor_after_linewise_edit(state, range.start.row);
    if operator == Operator::Change {
        state.mode = EditorMode::Insert;
    }
}

fn capture_linewise_undo_state(state: &mut EditorState, range: TextRange) {
    if should_restore_linewise_delete_to_column_zero(state, range) {
        let cursor = state.cursor;
        state.cursor.col = 0;
        state.capture();
        state.cursor = cursor;
    } else {
        state.capture();
    }
}

fn should_restore_linewise_delete_to_column_zero(state: &EditorState, range: TextRange) -> bool {
    range.start.row == range.end.row
        || (range.start.row == 0
            && range.end.row >= state.lines.iter_row().count().saturating_sub(1))
}

fn place_cursor_after_linewise_edit(state: &mut EditorState, deleted_start_row: usize) {
    state.cursor.row = deleted_start_row.min(state.lines.len().saturating_sub(1));
    state.cursor.col = state
        .lines
        .iter_row()
        .nth(state.cursor.row)
        .and_then(|row| row.iter().position(|ch| !ch.is_ascii_whitespace()))
        .unwrap_or(0);
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
