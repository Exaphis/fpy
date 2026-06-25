use std::cmp::min;

use jagged::{index::RowIndex, Index2};

use crate::{
    clipboard::ClipboardTrait,
    helper::{append_str, insert_char, insert_str, max_row},
    EditorMode, EditorState,
};

use super::{delete::delete_selection, Execute};

#[derive(Clone, Debug)]
pub struct Paste;

impl Execute for Paste {
    fn execute(&mut self, state: &mut EditorState) {
        let s = state.clip.get_text();
        if s.is_empty() {
            return;
        }

        state.capture();
        state.clamp_column();

        // In single-line mode, replace newlines with spaces
        if state.view.single_line {
            let s = s.replace('\n', " ").replace('\r', "");
            if state.mode == EditorMode::Insert {
                paste_at_cursor(state, &s);
            } else {
                append_str(&mut state.lines, &mut state.cursor, &s);
            }
            return;
        }

        if state.mode == EditorMode::Insert {
            paste_at_cursor(state, &s);
            return;
        }

        let s = if let Some(stripped) = s.strip_prefix('\n') {
            if !state.lines.to_string().is_empty() {
                state.cursor = Index2::new(min(max_row(state), state.cursor.row + 1), 0);
                state.lines.insert(RowIndex::new(state.cursor.row), vec![]);
            }
            stripped
        } else {
            state.clamp_column();
            &s
        };

        append_str(&mut state.lines, &mut state.cursor, s);
    }
}

fn paste_at_cursor(state: &mut EditorState, text: &str) {
    for ch in text.chars() {
        insert_char(&mut state.lines, &mut state.cursor, ch, false);
    }
}

#[derive(Clone, Debug)]
pub struct PasteOverSelection;

impl Execute for PasteOverSelection {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(selection) = state.selection.take() {
            state.capture();
            state.clamp_column();
            let _ = delete_selection(state, &selection);

            // In single-line mode, replace newlines with spaces
            let text = state.clip.get_text();
            let text = if state.view.single_line {
                text.replace('\n', " ").replace('\r', "")
            } else {
                text
            };
            insert_str(&mut state.lines, &mut state.cursor, &text);
        }
    }
}

#[derive(Clone, Debug)]
pub struct CopyLine;

impl Execute for CopyLine {
    fn execute(&mut self, state: &mut EditorState) {
        if let Some(line) = state.lines.get(RowIndex::new(state.cursor.row)) {
            let text = String::from('\n') + &line.iter().collect::<String>();
            state.clip.set_text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::actions::Undo;
    use crate::clipboard::{ClipboardTrait, InternalClipboard};
    use crate::state::selection::Selection;
    use crate::Index2;
    use crate::Lines;

    use super::*;
    fn test_state() -> EditorState {
        let mut state = EditorState::new(Lines::from("Hello World!\n\n123."));
        state.set_clipboard(InternalClipboard::default());
        state
    }

    #[test]
    fn test_copy_paste() {
        let mut state = test_state();
        state.clip.set_text("Hel".to_string());
        Paste.execute(&mut state);

        assert_eq!(state.cursor, Index2::new(0, 3));
        assert_eq!(state.lines, Lines::from("HHelello World!\n\n123."));
    }

    #[test]
    fn test_insert_mode_paste_inserts_at_cursor() {
        let mut state = test_state();
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 5);
        state.clip.set_text(", brave".to_string());

        Paste.execute(&mut state);

        assert_eq!(state.cursor, Index2::new(0, 12));
        assert_eq!(state.lines, Lines::from("Hello, brave World!\n\n123."));
    }

    #[test]
    fn test_insert_mode_single_line_paste_inserts_at_cursor() {
        let mut state = EditorState::new(Lines::from("Hello World"));
        state.set_clipboard(InternalClipboard::default());
        state.set_single_line(true);
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 5);
        state.clip.set_text(",\nbrave".to_string());

        Paste.execute(&mut state);

        assert_eq!(state.cursor, Index2::new(0, 12));
        assert_eq!(state.lines, Lines::from("Hello, brave World"));
    }

    #[test]
    fn test_insert_mode_paste_leading_newline_splits_at_cursor() {
        let mut state = EditorState::new(Lines::from("foo baz"));
        state.set_clipboard(InternalClipboard::default());
        state.mode = EditorMode::Insert;
        state.cursor = Index2::new(0, 3);
        state.clip.set_text("\nbar".to_string());

        Paste.execute(&mut state);

        assert_eq!(state.cursor, Index2::new(1, 3));
        assert_eq!(state.lines, Lines::from("foo\nbar baz"));
    }

    #[test]
    fn test_paste_with_newline_into_empty_buffer() {
        let mut state = EditorState::default();
        state.set_clipboard(InternalClipboard::default());
        state.clip.set_text("\ntext".to_string());

        Paste.execute(&mut state);

        assert_eq!(state.cursor, Index2::new(0, 3));
        assert_eq!(state.lines, Lines::from("text"));
    }

    #[test]
    fn test_paste_over_selection() {
        let mut state = test_state();
        state.selection = Some(Selection::new(Index2::new(0, 6), Index2::new(0, 10)));
        state.clip.set_text(String::from("Earth"));
        state.mode = EditorMode::Visual;

        PasteOverSelection.execute(&mut state);

        assert_eq!(state.lines, Lines::from("Hello Earth!\n\n123."));
        assert_eq!(state.cursor, Index2::new(0, 10));
        assert_eq!(state.mode, EditorMode::Visual);

        Undo.execute(&mut state);

        assert_eq!(state.lines, Lines::from("Hello World!\n\n123."));
        assert_eq!(state.mode, EditorMode::Visual);
    }
}
