use crate::actions::motion::CharacterClass;
use crate::{
    helper::{skip_whitespace, skip_whitespace_rev},
    EditorState,
};
use jagged::Index2;

use super::range::TextRange;

pub(crate) fn word_forward_range(state: &EditorState) -> Option<TextRange> {
    let start = state.cursor;
    let start_char = state.lines.get(start)?;
    let mut end = start;
    let start_class = CharacterClass::from(start_char);

    for (ch, idx) in state.lines.iter().from(start) {
        if CharacterClass::from(ch) != start_class {
            break;
        }
        end = idx;
    }
    end.col += 1;
    skip_whitespace(&state.lines, &mut end);
    Some(TextRange::exclusive(start, end))
}

pub(crate) fn word_end_range(state: &EditorState) -> Option<TextRange> {
    let start = state.cursor;
    let start_char = state.lines.get(start)?;
    let start_class = CharacterClass::from(start_char);
    let mut end = start;

    for (ch, idx) in state.lines.iter().from(start) {
        if CharacterClass::from(ch) != start_class {
            break;
        }
        end = idx;
    }
    Some(TextRange::inclusive(start, end))
}

pub(crate) fn big_word_forward_range(state: &EditorState) -> Option<TextRange> {
    let start = state.cursor;
    state.lines.get(start)?;
    let mut end = start;
    for (ch, idx) in state.lines.iter().from(start) {
        if ch.is_some_and(char::is_ascii_whitespace) {
            break;
        }
        end = idx;
    }
    end.col += 1;
    skip_whitespace(&state.lines, &mut end);
    Some(TextRange::exclusive(start, end))
}

pub(crate) fn big_word_end_range(state: &EditorState) -> Option<TextRange> {
    let start = state.cursor;
    state.lines.get(start)?;
    let mut end = start;
    for (ch, idx) in state.lines.iter().from(start) {
        if ch.is_some_and(char::is_ascii_whitespace) {
            break;
        }
        end = idx;
    }
    Some(TextRange::inclusive(start, end))
}

pub(crate) fn big_word_backward_range(state: &EditorState) -> Option<TextRange> {
    let end = state.cursor;
    if end.row == 0 && end.col == 0 {
        return None;
    }
    if end.col == 0 {
        return Some(TextRange::exclusive(
            Index2::new(
                end.row.saturating_sub(1),
                state.lines.len_col(end.row - 1).unwrap_or(0),
            ),
            end,
        ));
    }
    let mut start = Index2::new(end.row, end.col.saturating_sub(1));
    skip_whitespace_rev(&state.lines, &mut start);
    for (ch, idx) in state.lines.iter().from(start).rev() {
        if idx.col == 0 {
            start = idx;
            break;
        }
        if ch.is_some_and(char::is_ascii_whitespace) {
            break;
        }
        start = idx;
    }
    Some(TextRange::exclusive(start, end))
}

pub(crate) fn line_start_range(state: &EditorState) -> Option<TextRange> {
    Some(TextRange::exclusive(
        Index2::new(state.cursor.row, 0),
        state.cursor,
    ))
}

pub(crate) fn line_end_range(state: &EditorState) -> Option<TextRange> {
    let end = Index2::new(state.cursor.row, state.lines.len_col(state.cursor.row)?);
    Some(TextRange::exclusive(state.cursor, end))
}

pub(crate) fn line_down_range(state: &EditorState, count: usize) -> Option<TextRange> {
    if state.lines.is_empty() {
        return None;
    }
    Some(TextRange::linewise(
        state.cursor.row,
        state
            .cursor
            .row
            .saturating_add(count)
            .min(state.lines.len().saturating_sub(1)),
    ))
}

pub(crate) fn line_up_range(state: &EditorState, count: usize) -> Option<TextRange> {
    if state.lines.is_empty() {
        return None;
    }
    Some(TextRange::linewise(
        state.cursor.row.saturating_sub(count),
        state.cursor.row,
    ))
}

pub(crate) fn to_first_line_range(state: &EditorState) -> Option<TextRange> {
    if state.lines.is_empty() {
        return None;
    }
    Some(TextRange::linewise(0, state.cursor.row))
}

pub(crate) fn to_last_line_range(state: &EditorState) -> Option<TextRange> {
    if state.lines.is_empty() {
        return None;
    }
    Some(TextRange::linewise(
        state.cursor.row,
        state.lines.len().saturating_sub(1),
    ))
}

pub(crate) fn char_span_range(state: &EditorState, count: usize) -> Option<TextRange> {
    let line_len = state.lines.len_col(state.cursor.row)?;
    if state.cursor.col >= line_len {
        return None;
    }
    let end_col = state.cursor.col.saturating_add(count).min(line_len);
    Some(TextRange::exclusive(
        state.cursor,
        Index2::new(state.cursor.row, end_col),
    ))
}

pub(crate) fn char_forward_range(
    state: &EditorState,
    target: char,
    count: usize,
    till: bool,
) -> Option<TextRange> {
    let row = state.cursor.row;
    let line_len = state.lines.len_col(row)?;
    if state.cursor.col + 1 >= line_len {
        return None;
    }
    let mut seen = 0;
    for col in state.cursor.col + 1..line_len {
        if state.lines.get(Index2::new(row, col)).copied() == Some(target) {
            seen += 1;
            if seen == count {
                let end_col = if till { col.saturating_sub(1) } else { col };
                if end_col < state.cursor.col {
                    return None;
                }
                return Some(TextRange::inclusive(
                    state.cursor,
                    Index2::new(row, end_col),
                ));
            }
        }
    }
    None
}

pub(crate) fn char_backward_range(
    state: &EditorState,
    target: char,
    count: usize,
    till: bool,
) -> Option<TextRange> {
    let row = state.cursor.row;
    if state.cursor.col == 0 {
        return None;
    }
    let mut seen = 0;
    for col in (0..state.cursor.col).rev() {
        if state.lines.get(Index2::new(row, col)).copied() == Some(target) {
            seen += 1;
            if seen == count {
                let start_col = if till { col.saturating_add(1) } else { col };
                if start_col > state.cursor.col {
                    return None;
                }
                return Some(TextRange::inclusive(
                    Index2::new(row, start_col),
                    state.cursor,
                ));
            }
        }
    }
    None
}

pub(crate) fn word_backward_range(state: &EditorState) -> Option<TextRange> {
    let end = state.cursor;

    if end.row == 0 && end.col == 0 {
        return None;
    }

    if end.col == 0 {
        return Some(TextRange::exclusive(
            Index2::new(
                end.row.saturating_sub(1),
                state.lines.len_col(end.row - 1).unwrap_or(0),
            ),
            end,
        ));
    }

    let mut start = Index2::new(end.row, end.col.saturating_sub(1));
    skip_whitespace_rev(&state.lines, &mut start);
    let start_class = CharacterClass::from(state.lines.get(start));

    for (ch, idx) in state.lines.iter().from(start).rev() {
        if idx.col == 0 {
            start = idx;
            break;
        }
        if CharacterClass::from(ch) != start_class {
            break;
        }
        start = idx;
    }

    Some(TextRange::exclusive(start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorState, Lines};

    #[test]
    fn char_span_range_clamps_to_line_end() {
        let mut state = EditorState::new(Lines::from("abc"));
        assert_eq!(
            char_span_range(&state, 2),
            Some(TextRange::exclusive(Index2::new(0, 0), Index2::new(0, 2)))
        );
        state.cursor.col = 2;
        assert_eq!(
            char_span_range(&state, 3),
            Some(TextRange::exclusive(Index2::new(0, 2), Index2::new(0, 3)))
        );
        state.cursor.col = 3;
        assert_eq!(char_span_range(&state, 1), None);
    }

    #[test]
    fn char_search_ranges_support_to_till_forward_backward_and_counts() {
        let mut state = EditorState::new(Lines::from("a(b)c)d"));
        state.cursor.col = 0;
        assert_eq!(
            char_forward_range(&state, ')', 1, true),
            Some(TextRange::inclusive(Index2::new(0, 0), Index2::new(0, 2)))
        );
        assert_eq!(
            char_forward_range(&state, ')', 1, false),
            Some(TextRange::inclusive(Index2::new(0, 0), Index2::new(0, 3)))
        );
        assert_eq!(
            char_forward_range(&state, ')', 2, true),
            Some(TextRange::inclusive(Index2::new(0, 0), Index2::new(0, 4)))
        );

        state.cursor.col = 6;
        assert_eq!(
            char_backward_range(&state, '(', 1, true),
            Some(TextRange::inclusive(Index2::new(0, 2), Index2::new(0, 6)))
        );
        assert_eq!(
            char_backward_range(&state, '(', 1, false),
            Some(TextRange::inclusive(Index2::new(0, 1), Index2::new(0, 6)))
        );
        assert_eq!(char_forward_range(&state, 'x', 1, false), None);
    }

    #[test]
    fn word_ranges_match_prompt_toolkit_word_classes() {
        let mut state = EditorState::new(Lines::from("one two.three"));
        assert_eq!(
            word_forward_range(&state).unwrap(),
            TextRange::exclusive(Index2::new(0, 0), Index2::new(0, 4))
        );
        assert_eq!(
            word_end_range(&state).unwrap(),
            TextRange::inclusive(Index2::new(0, 0), Index2::new(0, 2))
        );
        state.cursor.col = 7;
        assert_eq!(
            word_backward_range(&state).unwrap(),
            TextRange::exclusive(Index2::new(0, 4), Index2::new(0, 7))
        );
    }
}
