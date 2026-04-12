use edtui::{
    EditorMode, EditorState, EditorTheme, Index2, Lines, SyntaxHighlighter,
    actions::{MoveDown, MoveUp},
};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

const INDENT_WIDTH: usize = 4;
const EDITOR_THEME_NAME: &str = "base16-ocean-dark";

pub(super) fn build_editor_state(text: &str) -> EditorState {
    let mut lines = Lines::from(text);
    if lines.is_empty() {
        // edtui treats "" as a zero-row buffer, which makes the first normal-mode `o`
        // create the initial row instead of opening a row below the current one.
        lines.push(Vec::new());
    }

    let mut editor = EditorState::new(lines);
    editor.mode = EditorMode::Insert;

    let rows = text.split('\n').collect::<Vec<_>>();
    let row = rows.len().saturating_sub(1);
    let col = rows.last().map_or(0, |line| line.chars().count());
    editor.cursor = Index2::new(row, col);
    editor
}

pub(super) fn move_editor_to_row(editor: &mut EditorState, target_row: usize) {
    let max_row = editor.lines.len().saturating_sub(1);
    let target_row = target_row.min(max_row);
    let current_row = editor.cursor.row;
    if target_row >= current_row {
        editor.execute(MoveDown(target_row - current_row));
    } else {
        editor.execute(MoveUp(current_row - target_row));
    }
}

pub(super) fn prompt_prefixes(awaiting_input: &Option<(String, bool)>) -> Option<(String, String)> {
    match awaiting_input {
        Some((prompt, true)) => {
            let first = format!("stdin (hidden) {prompt}");
            let continuation = " ".repeat(super::transcript::display_width(&first));
            Some((first, continuation))
        }
        Some((prompt, false)) => {
            let first = prompt.clone();
            let continuation = " ".repeat(super::transcript::display_width(&first));
            Some((first, continuation))
        }
        None => None,
    }
}

pub(super) fn editor_gutter_width(
    awaiting_input: &Option<(String, bool)>,
    visible_lines: usize,
) -> u16 {
    if let Some((prompt_prefix, continuation_prefix)) = prompt_prefixes(awaiting_input) {
        prompt_gutter_width(&prompt_prefix, &continuation_prefix)
    } else {
        line_number_gutter_width(visible_lines)
    }
}

pub(super) fn editor_gutter_lines(
    awaiting_input: &Option<(String, bool)>,
    height: usize,
    visible_lines: usize,
) -> Vec<Line<'static>> {
    if let Some((prompt_prefix, continuation_prefix)) = prompt_prefixes(awaiting_input) {
        prompt_gutter_lines(&prompt_prefix, &continuation_prefix, height)
    } else {
        line_number_gutter_lines(height, visible_lines)
    }
}

pub(super) fn editor_theme() -> EditorTheme<'static> {
    EditorTheme::default()
        .base(Style::default())
        .cursor_style(Style::default().add_modifier(Modifier::REVERSED))
        .selection_style(Style::default().bg(Color::DarkGray))
        .hide_status_line()
}

pub(super) fn editor_syntax_highlighter() -> Option<SyntaxHighlighter> {
    SyntaxHighlighter::new(EDITOR_THEME_NAME, "py").ok()
}

pub(super) fn editor_status_line(mode: EditorMode, detail: Option<&str>) -> Paragraph<'static> {
    let (label, style) = editor_mode_badge(mode);

    let mut spans = vec![Span::styled(format!(" {label} "), style)];
    if let Some(detail) = detail {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            detail.to_string(),
            Style::default().fg(Color::Cyan),
        ));
    }
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        "Ctrl-P palette",
        Style::default().fg(Color::DarkGray),
    ));

    Paragraph::new(Line::from(spans))
}

pub(super) fn status_label<'a>(
    awaiting_input: &'a Option<(String, bool)>,
    prompt_label: &'a str,
) -> Option<&'a str> {
    if awaiting_input.is_some() {
        Some("stdin")
    } else {
        Some(prompt_label)
    }
}

pub(super) fn indent_width() -> usize {
    INDENT_WIDTH
}

fn prompt_gutter_width(prompt_prefix: &str, continuation_prefix: &str) -> u16 {
    u16::try_from(
        super::transcript::display_width(prompt_prefix)
            .max(super::transcript::display_width(continuation_prefix)),
    )
    .unwrap_or(u16::MAX)
    .max(1)
}

fn prompt_gutter_lines(
    prompt_prefix: &str,
    continuation_prefix: &str,
    height: usize,
) -> Vec<Line<'static>> {
    let width = super::transcript::display_width(prompt_prefix)
        .max(super::transcript::display_width(continuation_prefix));
    (0..height.max(1))
        .map(|index| {
            let prefix = if index == 0 {
                prompt_prefix
            } else {
                continuation_prefix
            };
            Line::from(Span::styled(
                format!("{prefix:<width$}"),
                Style::default().fg(Color::Cyan),
            ))
        })
        .collect()
}

fn line_number_gutter_width(visible_lines: usize) -> u16 {
    let digits = visible_lines.max(1).to_string().len();
    u16::try_from(digits + 1).unwrap_or(u16::MAX).max(2)
}

fn line_number_gutter_lines(height: usize, visible_lines: usize) -> Vec<Line<'static>> {
    let width = usize::from(line_number_gutter_width(visible_lines)).saturating_sub(1);
    (0..height.max(1))
        .map(|index| {
            let line_number = index + 1;
            Line::from(Span::styled(
                format!("{line_number:>width$} ", width = width),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect()
}

fn editor_mode_badge(mode: EditorMode) -> (&'static str, Style) {
    match mode {
        EditorMode::Insert => ("INS", Style::default().fg(Color::Black).bg(Color::Cyan)),
        EditorMode::Normal => ("NAV", Style::default().fg(Color::Black).bg(Color::Yellow)),
        EditorMode::Visual => ("VIS", Style::default().fg(Color::Black).bg(Color::Magenta)),
        EditorMode::Search => ("SRCH", Style::default().fg(Color::Black).bg(Color::Green)),
    }
}

#[cfg(test)]
mod tests {
    use edtui::{EditorMode, Index2};

    use super::{
        build_editor_state, editor_gutter_lines, editor_syntax_highlighter, move_editor_to_row,
        prompt_gutter_lines, prompt_prefixes, status_label,
    };

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
    fn builds_ipython_prompt_prefixes() {
        let (first, continuation) = prompt_prefixes(&Some(("stdin> ".to_string(), false))).unwrap();
        assert_eq!(first, "stdin> ");
        assert_eq!(continuation, "       ");
    }

    #[test]
    fn uses_line_numbers_for_normal_editor_gutter() {
        let lines = editor_gutter_lines(&None, 2, 2);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn uses_stdin_label_in_status_bar() {
        assert_eq!(
            status_label(&Some(("input".to_string(), false)), "In [3]"),
            Some("stdin")
        );
    }

    #[test]
    fn uses_prompt_label_in_status_bar() {
        assert_eq!(status_label(&None, "In [3]"), Some("In [3]"));
    }

    #[test]
    fn builds_stdin_prompt_prefixes() {
        let (first, continuation) =
            prompt_prefixes(&Some(("In [3]: ".to_string(), false))).unwrap();
        assert_eq!(first, "In [3]: ");
        assert_eq!(continuation, "        ");
    }

    #[test]
    fn builds_python_syntax_highlighter() {
        assert!(editor_syntax_highlighter().is_some());
    }

    #[test]
    fn builds_prompt_gutter_lines() {
        let lines = prompt_gutter_lines("In [1]: ", "   ...: ", 2);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn moves_editor_to_target_row_and_clamps_to_end() {
        let mut editor = build_editor_state("a\nb\nc");
        move_editor_to_row(&mut editor, 1);
        assert_eq!(editor.cursor.row, 1);

        move_editor_to_row(&mut editor, 99);
        assert_eq!(editor.cursor.row, 2);
    }
}
