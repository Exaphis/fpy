use crossterm::event::KeyEvent;
use edtui::{
    EditorEventHandler, EditorMode, EditorState, Index2,
    actions::{Chainable, InsertChar, LineBreak, SwitchMode},
};

use super::editor::{PendingStdin, build_editor_state, indent_width};
use crate::extensions::{ExtensionContext, ExtensionManager, ExtensionOutcome};

pub(super) struct EditorController {
    editor: EditorState,
    editor_events: EditorEventHandler,
    history: Vec<String>,
    history_index: Option<usize>,
    pending_stdin: Option<PendingStdin>,
    extensions: ExtensionManager,
}

impl EditorController {
    pub(super) fn new() -> Self {
        Self {
            editor: build_editor_state(""),
            editor_events: EditorEventHandler::default(),
            history: Vec::new(),
            history_index: None,
            pending_stdin: None,
            extensions: ExtensionManager::with_defaults(),
        }
    }

    pub(super) fn begin_input_request(&mut self, prompt: String, password: bool) {
        self.pending_stdin = Some(PendingStdin::new(prompt, password));
        self.reset();
    }

    pub(super) fn clear_input_request(&mut self) {
        self.pending_stdin = None;
    }

    pub(super) fn take_pending_stdin(&mut self) -> Option<PendingStdin> {
        self.pending_stdin.take()
    }

    pub(super) fn pending_stdin(&self) -> Option<&PendingStdin> {
        self.pending_stdin.as_ref()
    }

    pub(super) fn awaiting_input(&self) -> bool {
        self.pending_stdin.is_some()
    }

    pub(super) fn mode(&self) -> EditorMode {
        self.editor.mode
    }

    pub(super) fn render_state(&self) -> EditorState {
        self.editor.clone()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.editor.lines.to_string().is_empty()
    }

    pub(super) fn is_single_line(&self) -> bool {
        self.editor.lines.len() <= 1
    }

    pub(super) fn reset(&mut self) {
        self.editor = build_editor_state("");
        self.editor_events = EditorEventHandler::default();
    }

    fn set_text(&mut self, text: &str) {
        self.editor = build_editor_state(text);
        self.editor_events = EditorEventHandler::default();
    }

    pub(super) fn take_text(&mut self) -> String {
        let text = self.editor.lines.to_string();
        self.reset();
        text
    }

    pub(super) fn text(&self) -> String {
        self.editor.lines.to_string()
    }

    fn cursor_byte(&self) -> usize {
        let text = self.text();
        cursor_to_byte(&text, self.editor.cursor)
    }

    pub(super) fn ghost_suggestion_suffix(&self) -> Option<String> {
        if self.editor.mode != EditorMode::Insert {
            return None;
        }
        let text = self.text();
        if text.is_empty() || self.cursor_byte() != text.len() {
            return None;
        }
        let line_prefix = text
            .rsplit_once('\n')
            .map_or(text.as_str(), |(_, line)| line);
        self.history.iter().rev().find_map(|entry| {
            history_suggestion_suffix_for_line_prefix(entry, line_prefix).map(str::to_owned)
        })
    }

    pub(super) fn accept_ghost_suggestion(&mut self) -> bool {
        let Some(suffix) = self.ghost_suggestion_suffix() else {
            return false;
        };
        let text = format!("{}{}", self.text(), suffix);
        self.set_text(&text);
        true
    }

    fn set_text_and_cursor_byte(&mut self, text: &str, cursor_byte: usize) {
        self.set_text(text);
        self.editor.cursor = byte_to_cursor(text, cursor_byte);
    }

    pub(super) fn handle_extension_key(&mut self, key: KeyEvent) -> bool {
        let cell = self.editor.lines.to_string();
        match self.extensions.on_key(
            key,
            ExtensionContext {
                cell: &cell,
                cursor_byte: self.cursor_byte(),
            },
        ) {
            ExtensionOutcome::Ignored => false,
            ExtensionOutcome::ReplaceCell(edit) => {
                self.set_text_and_cursor_byte(&edit.text, edit.cursor_byte);
                true
            }
        }
    }

    pub(super) fn history_up(&mut self) {
        let next = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => self.history.len().saturating_sub(1),
        };
        self.history_index = Some(next);
        let text = self.history[next].clone();
        self.set_text(&text);
    }

    pub(super) fn history_down(&mut self) {
        match self.history_index {
            Some(index) if index + 1 < self.history.len() => {
                self.history_index = Some(index + 1);
                let text = self.history[index + 1].clone();
                self.set_text(&text);
            }
            Some(_) => {
                self.history_index = None;
                self.reset();
            }
            None => {}
        }
    }

    pub(super) fn extend_history(&mut self, history: impl IntoIterator<Item = String>) {
        self.history.extend(history);
        self.history_index = None;
    }

    pub(super) fn push_history(&mut self, text: String) {
        self.history.push(text);
        self.history_index = None;
    }

    pub(super) fn pop_history_if_last(&mut self, text: &str) {
        if self.history.last().is_some_and(|entry| entry == text) {
            self.history.pop();
            self.history_index = None;
        }
    }

    pub(super) fn select_history(&mut self, index: usize) {
        if let Some(text) = self.history.get(index).cloned() {
            self.history_index = Some(index);
            self.set_text(&text);
        }
    }

    pub(super) fn has_history(&self) -> bool {
        !self.history.is_empty()
    }

    pub(super) fn history_position(&self) -> Option<(usize, usize)> {
        self.history_index
            .map(|index| (index + 1, self.history.len()))
    }

    pub(super) fn on_paste(&mut self, text: String) {
        self.editor_events.on_paste_event(text, &mut self.editor);
    }

    pub(super) fn on_key(&mut self, key: KeyEvent) {
        self.editor_events.on_key_event(key, &mut self.editor);
    }

    pub(super) fn insert_indent(&mut self) {
        for _ in 0..indent_width() {
            self.editor.execute(InsertChar(' '));
        }
    }

    pub(super) fn insert_line_break(&mut self) {
        self.editor
            .execute(SwitchMode(EditorMode::Insert).chain(LineBreak(1)));
    }
}

fn cursor_to_byte(text: &str, cursor: Index2) -> usize {
    let mut byte = 0;
    for (row, line) in text.split('\n').enumerate() {
        if row == cursor.row {
            return byte
                + line
                    .char_indices()
                    .nth(cursor.col)
                    .map_or(line.len(), |(index, _)| index);
        }
        byte += line.len() + 1;
    }
    text.len()
}

fn byte_to_cursor(text: &str, byte: usize) -> Index2 {
    let target = byte.min(text.len());
    let mut offset = 0;
    for (row, line) in text.split('\n').enumerate() {
        let line_end = offset + line.len();
        if target <= line_end {
            let col = line[..target.saturating_sub(offset)].chars().count();
            return Index2::new(row, col);
        }
        offset = line_end + 1;
    }
    let row = text.split('\n').count().saturating_sub(1);
    let col = text
        .rsplit('\n')
        .next()
        .map_or(0, |line| line.chars().count());
    Index2::new(row, col)
}

fn history_suggestion_suffix_for_line_prefix<'a>(entry: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    entry.lines().find_map(|line| {
        line.strip_prefix(prefix)
            .filter(|suffix| !suffix.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::history_suggestion_suffix_for_line_prefix;

    #[test]
    fn ghost_suggestion_matches_line_suffixes_only() {
        assert_eq!(
            history_suggestion_suffix_for_line_prefix("print('hello')", "pri"),
            Some("nt('hello')")
        );
        assert_eq!(
            history_suggestion_suffix_for_line_prefix("def foo():\n    pass", "def foo"),
            Some("():")
        );
        assert_eq!(
            history_suggestion_suffix_for_line_prefix("x = 1\nprint(x)", "pri"),
            Some("nt(x)")
        );
        assert_eq!(
            history_suggestion_suffix_for_line_prefix("print('hello')", ""),
            None
        );
        assert_eq!(
            history_suggestion_suffix_for_line_prefix("print('hello')", "zzz"),
            None
        );
    }
}
