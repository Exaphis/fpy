use crate::actions::motion::CharacterClass;
use crate::{
    helper::{skip_whitespace, skip_whitespace_rev},
    EditorState,
};
use jagged::Index2;

use super::range::TextRange;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MotionKind {
    WordForward,
    WordEnd,
    WordBackward,
    BigWordForward,
    BigWordEnd,
    BigWordBackward,
    LineStart,
    FirstNonWhitespace,
    LineEnd,
    Up,
    Down,
    FirstRow,
    LastRow,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CharMotionKind {
    FindForward,
    TillForward,
    FindBackward,
    TillBackward,
}

pub(crate) fn char_motion_range(
    state: &EditorState,
    motion: CharMotionKind,
    target: char,
    count: usize,
) -> Option<TextRange> {
    match motion {
        CharMotionKind::FindForward => char_forward_range(state, target, count, false),
        CharMotionKind::TillForward => char_forward_range(state, target, count, true),
        CharMotionKind::FindBackward => char_backward_range(state, target, count, false),
        CharMotionKind::TillBackward => char_backward_range(state, target, count, true),
    }
}

pub(crate) fn operator_range(
    state: &EditorState,
    motion: MotionKind,
    count: usize,
) -> Option<TextRange> {
    match motion {
        MotionKind::WordForward => counted_range(count, state, word_forward_range),
        MotionKind::WordEnd => counted_range(count, state, word_end_range),
        MotionKind::WordBackward => counted_range(count, state, word_backward_range),
        MotionKind::BigWordForward => counted_range(count, state, big_word_forward_range),
        MotionKind::BigWordEnd => counted_range(count, state, big_word_end_range),
        MotionKind::BigWordBackward => counted_range(count, state, big_word_backward_range),
        MotionKind::LineStart => line_start_range(state),
        MotionKind::LineEnd => line_end_range(state),
        MotionKind::Down => line_down_range(state, count),
        MotionKind::Up => line_up_range(state, count),
        MotionKind::LastRow => to_last_line_range(state),
        MotionKind::FirstRow => to_first_line_range(state),
        MotionKind::FirstNonWhitespace | MotionKind::Left | MotionKind::Right => None,
    }
}

fn counted_range(
    count: usize,
    editor: &EditorState,
    motion: fn(&EditorState) -> Option<TextRange>,
) -> Option<TextRange> {
    let mut scratch = editor.clone();
    let mut combined: Option<TextRange> = None;
    for _ in 0..count {
        let range = motion(&scratch)?;
        scratch.cursor = range.end;
        combined = Some(match combined {
            Some(mut combined_range) => {
                combined_range.end = range.end;
                combined_range
            }
            None => range,
        });
    }
    combined
}

pub(crate) fn motion_destination(
    state: &EditorState,
    motion: MotionKind,
    count: usize,
) -> Option<Index2> {
    motion_effect(state, motion, count).map(|(cursor, _)| cursor)
}

pub(crate) fn motion_effect(
    state: &EditorState,
    motion: MotionKind,
    count: usize,
) -> Option<(Index2, Option<usize>)> {
    let mut scratch = state.clone();
    let iterations = count.max(1);
    for _ in 0..iterations {
        apply_motion_once(&mut scratch, motion)?;
    }
    Some((scratch.cursor, scratch.preferred_col))
}

fn apply_motion_once(state: &mut EditorState, motion: MotionKind) -> Option<()> {
    use crate::actions::{Execute, MoveDown, MoveUp};

    match motion {
        MotionKind::WordForward => move_word_forward_once(state),
        MotionKind::WordEnd => move_word_end_once(state),
        MotionKind::WordBackward => move_word_backward_once(state),
        MotionKind::BigWordForward => move_big_word_forward_once(state),
        MotionKind::BigWordEnd => move_big_word_end_once(state),
        MotionKind::BigWordBackward => move_big_word_backward_once(state),
        MotionKind::LineStart => {
            state.preferred_col = None;
            state.cursor.col = 0;
        }
        MotionKind::FirstNonWhitespace => {
            state.preferred_col = None;
            state.cursor.col = first_non_whitespace_col_or_last_blank(&state.lines, state.cursor.row);
        }
        MotionKind::LineEnd => {
            use crate::helper::max_col;
            state.preferred_col = Some(usize::MAX);
            state.cursor.col = max_col(&state.lines, &state.cursor, state.mode);
        }
        MotionKind::Up => MoveUp(1).execute(state),
        MotionKind::Down => MoveDown(1).execute(state),
        MotionKind::FirstRow => {
            let col = state.preferred_col.unwrap_or(state.cursor.col);
            state.preferred_col = Some(col);
            state.cursor.row = 0;
            state.cursor.col = col.min(state.lines.len_col(state.cursor.row).unwrap_or_default().saturating_sub(1));
        }
        MotionKind::LastRow => {
            let col = state.preferred_col.unwrap_or(state.cursor.col);
            state.preferred_col = Some(col);
            state.cursor.row = state.lines.len().saturating_sub(1);
            state.cursor.col = col.min(state.lines.len_col(state.cursor.row).unwrap_or_default().saturating_sub(1));
        }
        MotionKind::Left => {
            use crate::actions::MoveBackward;
            MoveBackward(1).execute(state);
        }
        MotionKind::Right => {
            use crate::actions::MoveForward;
            MoveForward(1).execute(state);
        }
    }
    Some(())
}

fn move_word_forward_once(state: &mut EditorState) {
    use crate::actions::{Execute, MoveWordForward};

    let start = state.cursor;
    MoveWordForward(1).execute(state);
    if state.cursor.row > start.row
        && state.lines.iter_row().nth(state.cursor.row).is_some_and(|line| {
            !line.is_empty() && line.iter().all(|ch| ch.is_ascii_whitespace())
        })
    {
        state.cursor.col = start.col.min(
            state
                .lines
                .len_col(state.cursor.row)
                .unwrap_or_default()
                .saturating_sub(1),
        );
    }
}

fn first_non_whitespace_col_or_last_blank(lines: &crate::Lines, row: usize) -> usize {
    let Some(line) = lines.iter_row().nth(row) else {
        return 0;
    };
    line.iter()
        .position(|ch| !ch.is_ascii_whitespace())
        .unwrap_or_else(|| line.len().saturating_sub(1))
}

fn move_word_end_once(state: &mut EditorState) {
    use crate::actions::{Execute, MoveWordForwardToEndOfWord};

    if state.lines.len_col(state.cursor.row).unwrap_or_default() == 0
        || state.lines.iter_row().nth(state.cursor.row).is_some_and(|row| {
            !row.is_empty() && row.iter().all(|ch| ch.is_ascii_whitespace())
        })
    {
        move_to_next_nonempty_first_word_end(state);
        return;
    }

    let before = state.cursor;
    MoveWordForwardToEndOfWord(1).execute(state);
    if before.col == 0
        && state.cursor.row > before.row
        && state
            .lines
            .iter_row()
            .nth(before.row)
            .is_some_and(|row| row.is_empty() || row.iter().all(|ch| ch.is_ascii_whitespace()))
    {
        state.cursor.col = 0;
    }
    if state.cursor.row > before.row
        && state.lines.len_col(state.cursor.row).unwrap_or_default() == 0
        && state.lines.len_col(before.row).unwrap_or_default() > 0
    {
        move_to_next_nonempty_word_end(state);
        return;
    }
    if state.cursor != before || before.row + 1 >= state.lines.iter_row().count() {
        return;
    }
    move_to_next_nonempty_word_end(state);
}

fn move_to_next_nonempty_word_end(state: &mut EditorState) {
    use crate::actions::{Execute, MoveWordForwardToEndOfWord};

    if move_to_next_nonempty_line_start(state) {
        MoveWordForwardToEndOfWord(1).execute(state);
    }
}

fn move_to_next_nonempty_first_word_end(state: &mut EditorState) -> bool {
    let mut row = state.cursor.row + 1;
    while row < state.lines.iter_row().count() && state.lines.len_col(row).unwrap_or_default() == 0 {
        row += 1;
    }
    let Some(line) = state.lines.iter_row().nth(row) else {
        return false;
    };
    let Some(start) = line.iter().position(|ch| !ch.is_ascii_whitespace()) else {
        return false;
    };
    let class = CharacterClass::from(line.get(start));
    let mut end = start;
    while end + 1 < line.len() && CharacterClass::from(line.get(end + 1)) == class {
        end += 1;
    }
    state.preferred_col = None;
    state.cursor = Index2::new(row, end);
    true
}

fn move_to_next_nonempty_line_start(state: &mut EditorState) -> bool {
    let mut row = state.cursor.row + 1;
    while row < state.lines.iter_row().count()
        && (state.lines.len_col(row).unwrap_or_default() == 0
            || (state.lines.len_col(row).unwrap_or_default() == 1
                && row + 1 < state.lines.iter_row().count()))
    {
        row += 1;
    }
    if row >= state.lines.iter_row().count() {
        if state.cursor.row + 1 < state.lines.iter_row().count() {
            state.cursor = Index2::new(state.cursor.row + 1, 0);
        }
        return false;
    }
    state.cursor = Index2::new(row, 0);
    true
}

fn move_word_backward_once(state: &mut EditorState) {
    state.preferred_col = None;
    let Some(mut pos) = previous_big_word_scan_start(state) else {
        return;
    };
    if state.lines.len_col(pos.row).unwrap_or_default() == 0 {
        state.cursor = pos;
        return;
    }
    if pos == Index2::new(0, 0) && state.lines.get(pos).is_some_and(|ch| ch.is_ascii_whitespace()) {
        state.cursor = pos;
        return;
    }
    pos = match previous_non_whitespace(state, pos) {
        Some(pos) => pos,
        None => return,
    };
    let Some(ch) = state.lines.get(pos) else {
        return;
    };
    let class = CharacterClass::from(ch);
    let mut start = pos;
    while start.col > 0 {
        let prev = Index2::new(start.row, start.col - 1);
        let Some(prev_ch) = state.lines.get(prev) else {
            break;
        };
        if prev_ch.is_ascii_whitespace() || CharacterClass::from(prev_ch) != class {
            break;
        }
        start = prev;
    }
    state.cursor = start;
}

fn move_big_word_end_once(state: &mut EditorState) {
    use crate::actions::{Execute, MoveBigWordForwardToEndOfWord};

    if state.lines.len_col(state.cursor.row).unwrap_or_default() == 0
        && state.cursor.row + 1 < state.lines.iter_row().count()
    {
        move_to_next_nonempty_big_word_end(state);
        return;
    }
    MoveBigWordForwardToEndOfWord(1).execute(state);
}

fn move_to_next_nonempty_big_word_end(state: &mut EditorState) -> bool {
    let mut row = state.cursor.row + 1;
    while row < state.lines.iter_row().count() && state.lines.len_col(row).unwrap_or_default() == 0 {
        row += 1;
    }
    let Some(line) = state.lines.iter_row().nth(row) else {
        return false;
    };
    let Some(start) = line.iter().position(|ch| !ch.is_ascii_whitespace()) else {
        return false;
    };
    let mut end = start;
    while end + 1 < line.len() && !line[end + 1].is_ascii_whitespace() {
        end += 1;
    }
    state.preferred_col = None;
    state.cursor = Index2::new(row, end);
    true
}

fn move_big_word_forward_once(state: &mut EditorState) {
    use crate::actions::{Execute, MoveBigWordForward};

    let start_row = state.cursor.row;
    let start_col = state.cursor.col;
    MoveBigWordForward(1).execute(state);
    if state.cursor.row <= start_row {
        return;
    }
    for row in start_row + 1..=state.cursor.row {
        if state.lines.len_col(row).unwrap_or_default() == 0 {
            state.preferred_col = None;
            state.cursor.row = row;
            state.cursor.col = 0;
            return;
        }
        if state.lines.iter_row().nth(row).is_some_and(|line| {
            !line.is_empty() && line.iter().all(|ch| ch.is_ascii_whitespace())
        }) {
            state.preferred_col = None;
            state.cursor.row = row;
            state.cursor.col = start_col.min(
                state
                    .lines
                    .len_col(row)
                    .unwrap_or_default()
                    .saturating_sub(1),
            );
            return;
        }
    }
}

fn move_big_word_backward_once(state: &mut EditorState) {
    state.preferred_col = None;
    let Some(pos) = big_word_backward_destination(state) else {
        return;
    };
    state.cursor = pos;
}

fn big_word_backward_destination(state: &EditorState) -> Option<Index2> {
    let row_count = state.lines.iter_row().count();
    if row_count == 0 || state.cursor == Index2::new(0, 0) {
        return None;
    }

    let mut pos = previous_big_word_scan_start(state)?;
    loop {
        if state.lines.len_col(pos.row).unwrap_or_default() == 0
            || (pos == Index2::new(0, 0)
                && state.lines.get(pos).is_some_and(|ch| ch.is_ascii_whitespace()))
        {
            return Some(pos);
        }
        pos = previous_non_whitespace(state, pos)?;
        let mut start = pos;
        while start.col > 0 {
            let prev = Index2::new(start.row, start.col - 1);
            if state.lines.get(prev).is_none_or(|ch| ch.is_ascii_whitespace()) {
                break;
            }
            start = prev;
        }
        return Some(start);
    }
}

fn previous_big_word_scan_start(state: &EditorState) -> Option<Index2> {
    let row_len = state.lines.len_col(state.cursor.row).unwrap_or_default();
    let before_cursor_is_whitespace = state
        .lines
        .iter_row()
        .nth(state.cursor.row)
        .is_some_and(|row| {
            row.iter()
                .take(state.cursor.col.min(row_len))
                .all(|ch| ch.is_ascii_whitespace())
        });
    if state.cursor.col == 0 || before_cursor_is_whitespace {
        if state.cursor.row == 0 {
            return Some(Index2::new(0, 0));
        }
        return last_char_on_or_before_row(state, state.cursor.row - 1);
    }
    if state.cursor.col > 0 {
        return Some(Index2::new(state.cursor.row, state.cursor.col - 1));
    }
    None
}

fn previous_non_whitespace(state: &EditorState, mut pos: Index2) -> Option<Index2> {
    loop {
        if let Some(ch) = state.lines.get(pos) {
            if !ch.is_ascii_whitespace() {
                return Some(pos);
            }
        }
        pos = previous_index(state, pos)?;
    }
}

fn previous_index(state: &EditorState, pos: Index2) -> Option<Index2> {
    if pos.col > 0 {
        return Some(Index2::new(pos.row, pos.col - 1));
    }
    if pos.row == 0 {
        return None;
    }
    last_char_on_or_before_row(state, pos.row - 1)
}

fn last_char_on_or_before_row(state: &EditorState, row: usize) -> Option<Index2> {
    loop {
        let len = state.lines.len_col(row).unwrap_or_default();
        if len > 0 {
            return Some(Index2::new(row, len - 1));
        }
        return Some(Index2::new(row, 0));
    }
}

pub(crate) fn word_forward_range(state: &EditorState) -> Option<TextRange> {
    let start = state.cursor;
    if state.lines.len_col(start.row).unwrap_or_default() == 0
        && start.row + 1 < state.lines.iter_row().count()
    {
        return Some(TextRange::exclusive(start, Index2::new(start.row + 1, 0)));
    }
    let start_char = state.lines.get(start)?;
    let mut end = start;
    let start_class = CharacterClass::from(start_char);

    for (ch, idx) in state.lines.iter().from(start) {
        if idx.row != start.row || CharacterClass::from(ch) != start_class {
            break;
        }
        end = idx;
    }
    end.col += 1;
    if let Some(line) = state.lines.get(jagged::index::RowIndex::new(end.row)) {
        while end.col < line.len() && line[end.col].is_ascii_whitespace() {
            end.col += 1;
        }
    }
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
