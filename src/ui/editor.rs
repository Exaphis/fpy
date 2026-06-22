use edtui::{EditorMode, EditorState, EditorTheme, Index2, Lines, SyntaxHighlighter};
use ratatui::style::Style;

use super::{
    style::{UiStyle, semantic_style},
    syntax::editor_python_syntax_highlighter,
};

const INDENT_WIDTH: usize = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PendingStdin {
    prompt: String,
    password: bool,
}

impl PendingStdin {
    pub(super) fn new(prompt: String, password: bool) -> Self {
        Self { prompt, password }
    }

    pub(super) fn prompt(&self) -> &str {
        &self.prompt
    }

    pub(super) fn password(&self) -> bool {
        self.password
    }

    pub(super) fn visible_prompt(&self) -> Option<&str> {
        (!self.prompt.is_empty()).then_some(self.prompt())
    }
}

pub(super) fn build_editor_state(text: &str) -> EditorState {
    let lines = Lines::from(text);
    let mut editor = EditorState::new(lines);
    editor.mode = EditorMode::Insert;

    let rows = text.split('\n').collect::<Vec<_>>();
    let row = rows.len().saturating_sub(1);
    let col = rows.last().map_or(0, |line| line.chars().count());
    editor.cursor = Index2::new(row, col);
    editor
}

pub(super) fn editor_theme() -> EditorTheme<'static> {
    EditorTheme::default()
        .base(Style::default())
        .hide_cursor()
        .selection_style(semantic_style(UiStyle::Selection))
        .hide_status_line()
}

pub(super) fn editor_syntax_highlighter() -> Option<SyntaxHighlighter> {
    editor_python_syntax_highlighter()
}

pub(super) fn status_label(
    awaiting_input: Option<&PendingStdin>,
    prompt_label: &str,
    history_position: Option<(usize, usize)>,
) -> Option<String> {
    if awaiting_input.is_some() {
        return None;
    }

    Some(match history_position {
        Some((current, total)) => format!("{prompt_label} [{current}/{total}]"),
        None => prompt_label.to_string(),
    })
}

pub(super) fn indent_width() -> usize {
    INDENT_WIDTH
}

#[cfg(test)]
mod tests {
    use edtui::{EditorMode, Index2};

    use super::{PendingStdin, build_editor_state, editor_syntax_highlighter, status_label};

    #[test]
    fn editor_state_starts_in_insert_mode_at_end() {
        let editor = build_editor_state("a\nbc");
        assert_eq!(editor.mode, EditorMode::Insert);
        assert_eq!(editor.cursor, Index2::new(1, 2));
    }

    #[test]
    fn empty_editor_starts_with_one_blank_row() {
        let editor = build_editor_state("");
        assert_eq!(editor.lines.len(), 1);
        assert_eq!(editor.cursor, Index2::new(0, 0));
        assert_eq!(editor.lines.to_string(), "");
    }

    #[test]
    fn hides_stdin_label_in_status_bar() {
        let stdin = PendingStdin::new("input".to_string(), false);
        assert_eq!(status_label(Some(&stdin), "In [3]", None), None);
    }

    #[test]
    fn uses_prompt_label_in_status_bar() {
        assert_eq!(
            status_label(None, "In [3]", None),
            Some("In [3]".to_string())
        );
    }

    #[test]
    fn appends_history_position_in_status_bar() {
        assert_eq!(
            status_label(None, "In [3]", Some((2, 10))),
            Some("In [3] [2/10]".to_string())
        );
    }

    #[test]
    fn builds_python_syntax_highlighter() {
        assert!(editor_syntax_highlighter().is_some());
    }
}
