use jagged::index::RowIndex;

use crate::{actions::motion::CharacterClass, EditorState, Index2, Lines};

use super::range::TextRange;

pub(crate) fn inner_word_range(state: &EditorState) -> Option<TextRange> {
    word_range_by(state, |ch, class| CharacterClass::from(&ch) == class)
}

pub(crate) fn around_word_range(state: &EditorState) -> Option<TextRange> {
    around_word_range_by(state, |ch, class| CharacterClass::from(&ch) == class)
}

pub(crate) fn inner_big_word_range(state: &EditorState) -> Option<TextRange> {
    let ch = state.lines.get(state.cursor).copied()?;
    if ch.is_ascii_whitespace() {
        return None;
    }
    word_range_by(state, |ch, _| !ch.is_ascii_whitespace())
}

pub(crate) fn around_big_word_range(state: &EditorState) -> Option<TextRange> {
    around_word_range_by(state, |ch, _| !ch.is_ascii_whitespace())
}

pub(crate) fn inner_between_range(
    state: &EditorState,
    opening: char,
    closing: char,
) -> Option<TextRange> {
    between_range(state, opening, closing, false)
}

pub(crate) fn around_between_range(
    state: &EditorState,
    opening: char,
    closing: char,
) -> Option<TextRange> {
    between_range(state, opening, closing, true)
}

fn word_range_by(
    state: &EditorState,
    same_unit: impl Fn(char, CharacterClass) -> bool,
) -> Option<TextRange> {
    let row_index = state.cursor.row;
    let line = state.lines.get(RowIndex::new(row_index))?;
    let len_col = state.lines.len_col(row_index)?;
    if len_col == 0 {
        if row_index + 1 < state.lines.iter_row().count()
            && state.lines.len_col(row_index + 1).unwrap_or_default() > 0
        {
            return None;
        }
        if row_index > 0 {
            let prev_row = row_index - 1;
            let prev_len = state.lines.len_col(prev_row).unwrap_or_default();
            if prev_len > 0 {
                let prev_line = state.lines.get(RowIndex::new(prev_row))?;
                let mut end_col = prev_len - 1;
                while end_col > 0 && prev_line[end_col].is_ascii_whitespace() {
                    end_col -= 1;
                }
                if !prev_line[end_col].is_ascii_whitespace() {
                    return Some(TextRange::exclusive(
                        Index2::new(prev_row, end_col),
                        Index2::new(row_index, 1),
                    ));
                }
            }
        }
        return None;
    }
    if state.cursor.col >= len_col {
        return None;
    }

    let mut cursor_col = state.cursor.col;
    if line
        .get(cursor_col)
        .is_some_and(|ch| ch.is_ascii_whitespace())
    {
        let mut whitespace_start = cursor_col;
        while whitespace_start > 0
            && line
                .get(whitespace_start - 1)
                .is_some_and(|ch| ch.is_ascii_whitespace())
        {
            whitespace_start -= 1;
        }
        while cursor_col + 1 < len_col
            && line
                .get(cursor_col + 1)
                .is_some_and(|ch| ch.is_ascii_whitespace())
        {
            cursor_col += 1;
        }
        return Some(TextRange::inclusive(
            Index2::new(row_index, whitespace_start),
            Index2::new(row_index, cursor_col),
        ));
    }

    let start_char = *line.get(cursor_col)?;
    let class = CharacterClass::from(&start_char);
    let mut start = cursor_col;
    while start > 0
        && line
            .get(start - 1)
            .is_some_and(|ch| same_unit(*ch, class.clone()))
    {
        start -= 1;
    }
    let mut end = cursor_col;
    while end + 1 < len_col
        && line
            .get(end + 1)
            .is_some_and(|ch| same_unit(*ch, class.clone()))
    {
        end += 1;
    }
    Some(TextRange::inclusive(
        Index2::new(row_index, start),
        Index2::new(row_index, end),
    ))
}

fn around_word_range_by(
    state: &EditorState,
    same_unit: impl Fn(char, CharacterClass) -> bool,
) -> Option<TextRange> {
    let row_index = state.cursor.row;
    let line = state.lines.get(RowIndex::new(row_index))?;
    let len_col = state.lines.len_col(row_index)?;
    if len_col == 0 {
        if row_index + 1 < state.lines.iter_row().count() {
            let next_row = row_index + 1;
            let next_len = state.lines.len_col(next_row).unwrap_or_default();
            if next_len == 0 {
                return Some(TextRange::exclusive(
                    Index2::new(row_index, 0),
                    Index2::new(next_row, 0),
                ));
            }
            if next_len > 0 {
                let next_line = state.lines.get(RowIndex::new(next_row))?;
                let mut end_col = 0;
                while end_col < next_len && next_line[end_col].is_ascii_whitespace() {
                    end_col += 1;
                }
                if end_col < next_len {
                    let class = CharacterClass::from(&next_line[end_col]);
                    while end_col + 1 < next_len
                        && next_line
                            .get(end_col + 1)
                            .is_some_and(|ch| same_unit(*ch, class.clone()))
                    {
                        end_col += 1;
                    }
                    return Some(TextRange::exclusive(
                        Index2::new(row_index, 0),
                        Index2::new(next_row, end_col + 1),
                    ));
                }
            }
        }
        return None;
    }
    if state.cursor.col >= len_col {
        return None;
    }
    if line
        .get(state.cursor.col)
        .is_some_and(|ch| ch.is_ascii_whitespace())
    {
        let mut start = state.cursor.col;
        while start > 0
            && line
                .get(start - 1)
                .is_some_and(|ch| ch.is_ascii_whitespace())
        {
            start -= 1;
        }
        let mut end = state.cursor.col;
        while end + 1 < len_col && line.get(end + 1).is_some_and(|ch| ch.is_ascii_whitespace()) {
            end += 1;
        }
        if end + 1 >= len_col {
            return around_word_across_next_line(state, row_index, start, same_unit);
        }
        end += 1;
        let class = CharacterClass::from(&line[end]);
        while end + 1 < len_col
            && line
                .get(end + 1)
                .is_some_and(|ch| same_unit(*ch, class.clone()))
        {
            end += 1;
        }
        return Some(TextRange::inclusive(
            Index2::new(row_index, start),
            Index2::new(row_index, end),
        ));
    }
    include_around_whitespace(state, word_range_by(state, same_unit)?)
}

fn around_word_across_next_line(
    state: &EditorState,
    row_index: usize,
    start_col: usize,
    same_unit: impl Fn(char, CharacterClass) -> bool,
) -> Option<TextRange> {
    let next_row = row_index + 1;
    let next_len = state.lines.len_col(next_row).unwrap_or_default();
    if next_len == 0 {
        return None;
    }
    let next_line = state.lines.get(RowIndex::new(next_row))?;
    let mut end_col = 0;
    while end_col < next_len && next_line[end_col].is_ascii_whitespace() {
        end_col += 1;
    }
    if end_col >= next_len {
        return None;
    }
    let class = CharacterClass::from(&next_line[end_col]);
    while end_col + 1 < next_len
        && next_line
            .get(end_col + 1)
            .is_some_and(|ch| same_unit(*ch, class.clone()))
    {
        end_col += 1;
    }
    Some(TextRange::exclusive(
        Index2::new(row_index, start_col),
        Index2::new(next_row, end_col + 1),
    ))
}

fn include_around_whitespace(state: &EditorState, range: TextRange) -> Option<TextRange> {
    let row_index = range.end.row;
    let line_len = state.lines.len_col(row_index)?;
    let mut end = range.end;
    while end.col + 1 < line_len {
        let next_col = end.col + 1;
        let Some(ch) = state.lines.get(Index2::new(row_index, next_col)) else {
            break;
        };
        if !ch.is_ascii_whitespace() {
            break;
        }
        end.col = next_col;
    }
    if end.col != range.end.col {
        return Some(TextRange::inclusive(range.start, end));
    }
    let mut start = range.start;
    let mut whitespace_start = range.start;
    while whitespace_start.col > 0 {
        let prev_col = whitespace_start.col - 1;
        let Some(ch) = state.lines.get(Index2::new(row_index, prev_col)) else {
            break;
        };
        if !ch.is_ascii_whitespace() {
            break;
        }
        whitespace_start.col = prev_col;
    }
    if whitespace_start.col > 0 {
        start = whitespace_start;
    }
    Some(TextRange::inclusive(start, range.end))
}

fn between_range(
    state: &EditorState,
    opening: char,
    closing: char,
    around: bool,
) -> Option<TextRange> {
    let inner = select_between(
        &state.lines,
        state.cursor,
        |(ch, _)| *ch == opening,
        |(ch, _)| *ch == closing,
        |(_, _)| false,
        |(_, _)| false,
    )?;
    if around {
        let start = Index2::new(inner.start.row, inner.start.col.saturating_sub(1));
        let mut end = Index2::new(inner.end.row, inner.end.col.saturating_add(1));
        if opening == closing {
            let line_len = state.lines.len_col(end.row)?;
            while end.col + 1 < line_len {
                let next_col = end.col + 1;
                let Some(ch) = state.lines.get(Index2::new(end.row, next_col)) else {
                    break;
                };
                if !ch.is_ascii_whitespace() {
                    break;
                }
                end.col = next_col;
            }
        }
        Some(TextRange::inclusive(start, end))
    } else {
        Some(TextRange::inclusive(inner.start, inner.end))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectionRange {
    start: Index2,
    end: Index2,
}

fn select_between(
    lines: &Lines,
    cursor: Index2,
    opening_predicate_excl: impl Fn((&char, usize)) -> bool,
    closing_predicate_excl: impl Fn((&char, usize)) -> bool,
    opening_predicate_incl: impl Fn((&char, usize)) -> bool,
    closing_predicate_incl: impl Fn((&char, usize)) -> bool,
) -> Option<SelectionRange> {
    let len_col = lines.len_col(cursor.row)?;
    if cursor.col >= len_col {
        return None;
    }

    let row_index = cursor.row;
    let line = lines.get(RowIndex::new(row_index))?;
    let start_col = cursor.col;

    let mut opening: Option<usize> = None;
    let mut prev_col = start_col;
    for col in (0..=start_col).rev() {
        if let Some(ch) = line.get(col) {
            if opening_predicate_excl((ch, col)) {
                opening = Some(prev_col);
                break;
            }
            if opening_predicate_incl((ch, col)) {
                opening = Some(col);
                break;
            }
        }
        prev_col = col;
    }

    let mut closing: Option<usize> = None;
    let mut prev_col = start_col;
    for col in start_col..len_col {
        if let Some(ch) = line.get(col) {
            if closing_predicate_excl((ch, col)) {
                closing = Some(prev_col);
                break;
            }
            if closing_predicate_incl((ch, col)) {
                closing = Some(col);
                break;
            }
        }
        prev_col = col;
    }

    Some(SelectionRange {
        start: Index2::new(row_index, opening?),
        end: Index2::new(row_index, closing?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lines;

    fn make_state(text: &str, col: usize) -> EditorState {
        let mut state = EditorState::new(Lines::from(text));
        state.cursor = Index2::new(0, col);
        state
    }

    fn cols(range: Option<TextRange>) -> Option<(usize, usize)> {
        range.map(|range| (range.start.col, range.end.col))
    }

    #[test]
    fn word_ranges_select_inner_and_around_words() {
        let state = make_state("one two   three", 5);
        assert_eq!(cols(inner_word_range(&state)), Some((4, 6)));
        assert_eq!(cols(around_word_range(&state)), Some((4, 9)));
    }

    #[test]
    fn word_ranges_treat_whitespace_and_punctuation_as_objects() {
        let state = make_state("one  += two", 4);
        assert_eq!(cols(inner_word_range(&state)), Some((3, 4)));
        let state = make_state("one  += two", 6);
        assert_eq!(cols(inner_word_range(&state)), Some((5, 6)));
    }

    #[test]
    fn big_word_ranges_select_non_whitespace_only() {
        let state = make_state("one+two   three", 3);
        assert_eq!(cols(inner_big_word_range(&state)), Some((0, 6)));
        assert_eq!(cols(around_big_word_range(&state)), Some((0, 9)));
        let state = make_state("one two", 3);
        assert_eq!(inner_big_word_range(&state), None);
    }

    #[test]
    fn delimiter_ranges_support_inner_around_and_closing_aliases() {
        let state = make_state("foo(bar) baz", 5);
        assert_eq!(cols(inner_between_range(&state, '(', ')')), Some((4, 6)));
        assert_eq!(cols(around_between_range(&state, '(', ')')), Some((3, 7)));

        let state = make_state("foo[bar] baz", 5);
        assert_eq!(cols(inner_between_range(&state, '[', ']')), Some((4, 6)));
    }

    #[test]
    fn delimiter_ranges_return_none_when_not_found() {
        let state = make_state("foo bar", 2);
        assert_eq!(inner_between_range(&state, '(', ')'), None);
        assert_eq!(around_between_range(&state, '"', '"'), None);
    }
}
