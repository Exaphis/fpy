use crate::{state::selection::Selection, EditorMode, EditorState, Index2};

use super::range::{RangeKind, TextRange};

pub(crate) fn selection_range(state: &EditorState) -> Option<TextRange> {
    selection_to_range(state.selection.as_ref()?)
}

pub(crate) fn apply_range_as_selection(state: &mut EditorState, range: TextRange) {
    state.selection = Some(range_to_selection(range));
    state.mode = EditorMode::Visual;
}

pub(crate) fn select_current_line(state: &mut EditorState) {
    let row = state.cursor.row;
    if let Some(len_col) = state.lines.len_col(row) {
        state.selection = Some(
            Selection::new(
                Index2::new(row, 0),
                Index2::new(row, len_col.saturating_sub(1)),
            )
            .line_mode(),
        );
        state.mode = EditorMode::Visual;
    }
}

pub(crate) fn range_to_selection(range: TextRange) -> Selection {
    if range.kind == RangeKind::Linewise {
        return Selection::new(range.start, range.end).line_mode();
    }
    Selection::new(range.start, range.end)
}

pub(crate) fn selection_to_range(selection: &Selection) -> Option<TextRange> {
    if selection.line_mode {
        return Some(TextRange::linewise(
            selection.start().row,
            selection.end().row,
        ));
    }
    Some(TextRange::inclusive(selection.start(), selection.end()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{state::selection::Selection, Index2};

    #[test]
    fn converts_forward_and_reversed_visual_selection_to_inclusive_range() {
        let selection = Selection::new(Index2::new(0, 1), Index2::new(0, 3));
        let range = selection_to_range(&selection).unwrap();
        assert_eq!(
            range,
            TextRange::inclusive(Index2::new(0, 1), Index2::new(0, 3))
        );

        let selection = Selection::new(Index2::new(0, 3), Index2::new(0, 1));
        let range = selection_to_range(&selection).unwrap();
        assert_eq!(
            range,
            TextRange::inclusive(Index2::new(0, 1), Index2::new(0, 3))
        );
    }

    #[test]
    fn converts_linewise_selection_to_linewise_range() {
        let selection = Selection::new(Index2::new(3, 0), Index2::new(1, 5)).line_mode();
        let range = selection_to_range(&selection).unwrap();
        assert_eq!(range, TextRange::linewise(1, 3));
    }
}
