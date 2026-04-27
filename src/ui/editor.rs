use edtui::{EditorMode, EditorState, EditorTheme, Index2, Lines, SyntaxHighlighter};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

const INDENT_WIDTH: usize = 4;
const EDITOR_THEME_NAME: &str = "base16-ocean-dark";

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

pub(super) fn prompt_prefixes(awaiting_input: Option<&PendingStdin>) -> Option<(String, String)> {
    match awaiting_input {
        Some(stdin) if stdin.prompt().is_empty() && !stdin.password() => None,
        Some(stdin) if stdin.password() => {
            let first = format!("stdin (hidden) {}", stdin.prompt());
            let continuation = " ".repeat(super::transcript::display_width(&first));
            Some((first, continuation))
        }
        Some(stdin) => {
            let first = stdin.prompt().to_string();
            let continuation = " ".repeat(super::transcript::display_width(&first));
            Some((first, continuation))
        }
        None => None,
    }
}

pub(super) fn editor_gutter_width(
    awaiting_input: Option<&PendingStdin>,
    visible_lines: usize,
) -> u16 {
    if let Some((prompt_prefix, continuation_prefix)) = prompt_prefixes(awaiting_input) {
        prompt_gutter_width(&prompt_prefix, &continuation_prefix)
    } else if awaiting_input.is_some() {
        0
    } else {
        line_number_gutter_width(visible_lines)
    }
}

pub(super) fn editor_gutter_lines(
    awaiting_input: Option<&PendingStdin>,
    height: usize,
    visible_lines: usize,
    first_visible_line: usize,
) -> Vec<Line<'static>> {
    if let Some((prompt_prefix, continuation_prefix)) = prompt_prefixes(awaiting_input) {
        prompt_gutter_lines(&prompt_prefix, &continuation_prefix, height)
    } else if awaiting_input.is_some() {
        Vec::new()
    } else {
        line_number_gutter_lines(height, visible_lines, first_visible_line)
    }
}

pub(super) fn editor_theme() -> EditorTheme<'static> {
    EditorTheme::default()
        .base(Style::default())
        .hide_cursor()
        .selection_style(Style::default().bg(Color::DarkGray))
        .hide_status_line()
}

pub(super) fn editor_syntax_highlighter() -> Option<SyntaxHighlighter> {
    SyntaxHighlighter::new(EDITOR_THEME_NAME, "py").ok()
}

pub(super) fn editor_status_prefix(mode: EditorMode, detail: Option<&str>) -> Paragraph<'static> {
    let (label, style) = editor_mode_badge(mode);

    let mut spans = vec![Span::styled(format!(" {label} "), style)];
    if let Some(detail) = detail {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            detail.to_string(),
            Style::default().fg(Color::Cyan),
        ));
    }

    Paragraph::new(Line::from(spans))
}

pub(super) fn editor_palette_hint() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "  Ctrl-P palette",
        Style::default().fg(Color::DarkGray),
    )))
}

pub(super) fn editor_palette_hint_width() -> u16 {
    u16::try_from("  Ctrl-P palette".chars().count()).unwrap_or(u16::MAX)
}

pub(super) fn editor_status_prefix_width(mode: EditorMode, detail: Option<&str>) -> u16 {
    let mode_label = match mode {
        EditorMode::Insert => "INS",
        EditorMode::Normal => "NAV",
        EditorMode::Visual => "VIS",
        EditorMode::Search => "SRCH",
    };

    let mut width = 2 + mode_label.chars().count() + 1;
    if let Some(detail) = detail {
        width += 1 + detail.chars().count();
    }

    u16::try_from(width).unwrap_or(u16::MAX)
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

fn line_number_gutter_lines(
    height: usize,
    visible_lines: usize,
    first_visible_line: usize,
) -> Vec<Line<'static>> {
    let width = usize::from(line_number_gutter_width(visible_lines)).saturating_sub(1);
    (0..height.max(1))
        .map(|index| {
            let line_number = first_visible_line + index + 1;
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
        PendingStdin, build_editor_state, editor_gutter_lines, editor_syntax_highlighter,
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
        let stdin = PendingStdin::new("stdin> ".to_string(), false);
        let (first, continuation) = prompt_prefixes(Some(&stdin)).unwrap();
        assert_eq!(first, "stdin> ");
        assert_eq!(continuation, "       ");
    }

    #[test]
    fn builds_hidden_password_prompt_prefixes() {
        let stdin = PendingStdin::new("Password: ".to_string(), true);
        let (first, continuation) = prompt_prefixes(Some(&stdin)).unwrap();
        assert_eq!(first, "stdin (hidden) Password: ");
        assert_eq!(continuation, "                         ");
    }

    #[test]
    fn uses_line_numbers_for_normal_editor_gutter() {
        let lines = editor_gutter_lines(None, 2, 2, 0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "1 ");
        assert_eq!(lines[1].spans[0].content.as_ref(), "2 ");
    }

    #[test]
    fn offsets_line_numbers_by_first_visible_line() {
        let lines = editor_gutter_lines(None, 2, 12, 9);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "10 ");
        assert_eq!(lines[1].spans[0].content.as_ref(), "11 ");
    }

    #[test]
    fn uses_no_gutter_for_empty_stdin_prompt() {
        let stdin = PendingStdin::new("".to_string(), false);
        assert_eq!(super::editor_gutter_width(Some(&stdin), 2), 0);
        assert!(editor_gutter_lines(Some(&stdin), 2, 2, 0).is_empty());
    }

    #[test]
    fn uses_prompt_gutter_for_nonempty_stdin_prompt() {
        let stdin = PendingStdin::new("(Pdb) ".to_string(), false);
        assert_eq!(super::editor_gutter_width(Some(&stdin), 2), 6);
        let lines = editor_gutter_lines(Some(&stdin), 2, 2, 0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "(Pdb) ");
        assert_eq!(lines[1].spans[0].content.as_ref(), "      ");
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
    fn builds_stdin_prompt_prefixes() {
        let stdin = PendingStdin::new("In [3]: ".to_string(), false);
        let (first, continuation) = prompt_prefixes(Some(&stdin)).unwrap();
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
        assert_eq!(lines[0].spans[0].content.as_ref(), "In [1]: ");
        assert_eq!(lines[1].spans[0].content.as_ref(), "   ...: ");
    }
}
