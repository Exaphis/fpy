use jagged::index::RowIndex;

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

fn apply_operator_with_capture(
    state: &mut EditorState,
    operator: Operator,
    range: TextRange,
    capture: bool,
) {
    match operator {
        Operator::Yank => yank_range(state, range),
        Operator::Delete | Operator::Change => {
            if capture {
                state.capture();
            }
            let yanked = extract_range(state, range);
            state.clip.set_text(yanked.to_string());
            clamp_cursor(state);
            if operator == Operator::Change {
                state.mode = EditorMode::Insert;
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
    state.cursor.row = start_row.min(state.lines.len().saturating_sub(1));
    state.cursor.col = 0;
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
