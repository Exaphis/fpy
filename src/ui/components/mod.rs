use edtui::EditorView;
use ratatui::layout::{Position, Rect};

use super::{
    display::{
        EditorModel, FooterModel, FrameCursorStyle, OverlayModel, TranscriptEntry, TranscriptModel,
    },
    editor::{build_editor_state, editor_syntax_highlighter, editor_theme},
    prompt::{input_prompt, output_prompt, styled_input_prompt, styled_output_prompt},
    style::{StyledLine, StyledSegment, UiStyle, render_styled_line, wrap_styled_line},
    transcript::{display_width, highlighted_execute_input, wrap_ansi_to_width},
};

pub(super) trait Component {
    fn render(&mut self, width: u16) -> Vec<RenderedLine>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderedLine {
    pub text: String,
    pub cursor_marker: Option<CursorMarker>,
}

impl RenderedLine {
    pub(super) fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            cursor_marker: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CursorMarker {
    pub position: Position,
    pub style: FrameCursorStyle,
}

pub(super) struct TranscriptComponent<'a> {
    transcript: &'a TranscriptModel,
}

impl<'a> TranscriptComponent<'a> {
    pub(super) fn new(transcript: &'a TranscriptModel) -> Self {
        Self { transcript }
    }
}

impl Component for TranscriptComponent<'_> {
    fn render(&mut self, width: u16) -> Vec<RenderedLine> {
        self.transcript
            .visible_entries()
            .iter()
            .flat_map(|entry| render_transcript_entry(entry, width))
            .map(RenderedLine::new)
            .collect()
    }
}

pub(super) struct EditorComponent<'a> {
    editor: &'a EditorModel,
}

impl<'a> EditorComponent<'a> {
    pub(super) fn new(editor: &'a EditorModel) -> Self {
        Self { editor }
    }
}

impl Component for EditorComponent<'_> {
    fn render(&mut self, width: u16) -> Vec<RenderedLine> {
        let default_prompt;
        let prompt = if self.editor.prompt.is_empty() {
            default_prompt = input_prompt(None);
            default_prompt.as_str()
        } else {
            &self.editor.prompt
        };
        let rendered_prompt = render_editor_prompt(prompt);
        let prompt_width = display_width(prompt);
        if prompt_width >= width as usize {
            return render_wrapped_editor_fallback(&rendered_prompt, &self.editor.text, width);
        }

        let content_width = width.saturating_sub(prompt_width as u16).max(1);
        let mut state = self
            .editor
            .render_state
            .clone()
            .unwrap_or_else(|| build_editor_state(&self.editor.text));
        let plan = EditorView::new(&mut state)
            .theme(editor_theme())
            .wrap(true)
            .syntax_highlighter(editor_syntax_highlighter())
            .render_plan(Rect::new(0, 0, content_width, u16::MAX));

        let mut rows = plan
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let prefix = if index == 0 {
                    rendered_prompt.clone()
                } else {
                    " ".repeat(prompt_width)
                };
                format!("{prefix}{}", spans_to_ansi(&row.spans))
            })
            .collect::<Vec<_>>();

        if rows.is_empty() {
            rows.push(rendered_prompt);
        }

        let cursor_row = plan
            .cursor
            .map(|cursor| cursor.position.y as usize)
            .unwrap_or_default()
            .min(rows.len().saturating_sub(1));
        let cursor_col = plan
            .cursor
            .map(|cursor| prompt_width + cursor.position.x as usize)
            .unwrap_or(prompt_width)
            .min(width.saturating_sub(1) as usize) as u16;

        rows.into_iter()
            .enumerate()
            .map(|(row, text)| RenderedLine {
                text,
                cursor_marker: (row == cursor_row).then_some(CursorMarker {
                    position: Position::new(cursor_col, row as u16),
                    style: cursor_style_for_mode(state.mode),
                }),
            })
            .collect()
    }
}

fn render_editor_prompt(prompt: &str) -> String {
    if is_ipython_prompt(prompt) {
        styled_input_prompt(prompt)
    } else {
        prompt.to_string()
    }
}

fn is_ipython_prompt(prompt: &str) -> bool {
    prompt.starts_with("In [") && prompt.ends_with(": ")
}

fn cursor_style_for_mode(mode: edtui::EditorMode) -> FrameCursorStyle {
    match mode {
        edtui::EditorMode::Insert | edtui::EditorMode::Search => FrameCursorStyle::Bar,
        edtui::EditorMode::Normal | edtui::EditorMode::Visual => FrameCursorStyle::Block,
    }
}

fn render_wrapped_editor_fallback(prompt: &str, text: &str, width: u16) -> Vec<RenderedLine> {
    let rows = wrap_ansi_to_width(&format!("{prompt}{text}"), width);
    let cursor_row = rows.len().saturating_sub(1);
    let cursor_col = rows
        .get(cursor_row)
        .map(|row| display_width(row))
        .unwrap_or_default()
        .min(width.saturating_sub(1) as usize) as u16;

    rows.into_iter()
        .enumerate()
        .map(|(row, text)| RenderedLine {
            text,
            cursor_marker: (row == cursor_row).then_some(CursorMarker {
                position: Position::new(cursor_col, row as u16),
                style: FrameCursorStyle::Bar,
            }),
        })
        .collect()
}

fn spans_to_ansi(spans: &[ratatui::text::Span<'static>]) -> String {
    render_styled_line(&StyledLine::new(
        spans
            .iter()
            .map(|span| StyledSegment::raw(span.content.to_string(), span.style))
            .collect(),
    ))
}

pub(super) struct FooterComponent<'a> {
    footer: &'a FooterModel,
}

impl<'a> FooterComponent<'a> {
    pub(super) fn new(footer: &'a FooterModel) -> Self {
        Self { footer }
    }
}

impl Component for FooterComponent<'_> {
    fn render(&mut self, width: u16) -> Vec<RenderedLine> {
        self.footer
            .text
            .as_ref()
            .map(|text| {
                wrap_styled_line(&styled_footer_line(text), width)
                    .into_iter()
                    .map(|line| render_styled_line(&line))
                    .map(RenderedLine::new)
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn styled_footer_line(text: &str) -> StyledLine {
    let Some((mode, rest)) = text.split_once(' ') else {
        return StyledLine::new(vec![StyledSegment::semantic(text, UiStyle::Plain)]);
    };

    let mode_style = match mode {
        "INS" => Some(UiStyle::ModeInsert),
        "NAV" => Some(UiStyle::ModeNormal),
        "VIS" => Some(UiStyle::ModeVisual),
        "SRCH" => Some(UiStyle::ModeSearch),
        _ => None,
    };
    let Some(mode_style) = mode_style else {
        return StyledLine::new(vec![StyledSegment::semantic(text, UiStyle::Plain)]);
    };

    let (rest, palette_hint) = if let Some(before) = rest.strip_suffix("Ctrl-P palette") {
        (before.trim_end(), Some("Ctrl-P palette"))
    } else {
        (rest, None)
    };

    let mut segments = vec![StyledSegment::semantic(format!(" {mode} "), mode_style)];
    if !rest.is_empty() {
        segments.push(StyledSegment::semantic(format!(" {rest}"), UiStyle::Plain));
    }
    if let Some(hint) = palette_hint {
        segments.push(StyledSegment::semantic(" ", UiStyle::Plain));
        segments.push(StyledSegment::semantic(hint, UiStyle::FooterHint));
    }
    StyledLine::new(segments)
}

pub(super) struct OverlayComponent<'a> {
    overlay: &'a OverlayModel,
}

impl<'a> OverlayComponent<'a> {
    pub(super) fn new(overlay: &'a OverlayModel) -> Self {
        Self { overlay }
    }
}

impl Component for OverlayComponent<'_> {
    fn render(&mut self, width: u16) -> Vec<RenderedLine> {
        match self.overlay {
            OverlayModel::None => Vec::new(),
            OverlayModel::Palette(palette) => {
                let mut rows = vec![RenderedLine::new("Command Palette")];
                rows.extend(palette.items.iter().enumerate().flat_map(|(index, item)| {
                    let marker = if index == palette.selected {
                        "> "
                    } else {
                        "  "
                    };
                    wrap_ansi_to_width(&format!("{marker}{item}"), width)
                        .into_iter()
                        .map(RenderedLine::new)
                }));
                rows
            }
            OverlayModel::HistorySearch(search) => {
                let mut rows = vec![RenderedLine::new("History Search")];
                rows.extend(
                    wrap_ansi_to_width(&format!("query: {}", search.query), width)
                        .into_iter()
                        .map(RenderedLine::new),
                );
                if search.results.is_empty() {
                    rows.push(RenderedLine::new("no history matches"));
                } else {
                    rows.extend(search.results.iter().flat_map(|result| {
                        let marker = if result.selected { "> " } else { "  " };
                        wrap_ansi_to_width(&format!("{marker}{}", result.summary), width)
                            .into_iter()
                            .map(RenderedLine::new)
                    }));
                }
                if !search.preview_lines.is_empty() {
                    rows.push(RenderedLine::new("preview"));
                    rows.extend(
                        search
                            .preview_lines
                            .iter()
                            .flat_map(|line| wrap_ansi_to_width(line, width))
                            .map(RenderedLine::new),
                    );
                }
                rows
            }
        }
    }
}

fn render_transcript_entry(entry: &TranscriptEntry, width: u16) -> Vec<String> {
    match entry {
        TranscriptEntry::Input(input) => wrap_ansi_to_width(
            &highlighted_execute_input(input.execution_count, &input.code),
            width,
        ),
        TranscriptEntry::Stream(stream) => wrap_ansi_to_width(&stream.text, width),
        TranscriptEntry::ExecuteResult(output) | TranscriptEntry::DisplayData(output) => output
            .mime
            .text_plain
            .as_deref()
            .map(|text| {
                let prompt = styled_output_prompt(&output_prompt(output.execution_count));
                wrap_ansi_to_width(&format!("{prompt}: {text}"), width)
            })
            .unwrap_or_else(|| vec![String::new()]),
        TranscriptEntry::Error(error) => wrap_ansi_to_width(&error.traceback.join("\n"), width),
        TranscriptEntry::Stdin(stdin) => {
            let value = if stdin.password {
                ""
            } else {
                stdin.value.as_deref().unwrap_or_default()
            };
            wrap_ansi_to_width(&format!("{}{value}", stdin.prompt), width)
        }
        TranscriptEntry::System(system) => wrap_ansi_to_width(&system.text, width),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        display::{MimeBundle, StreamName},
        transcript::strip_ansi,
    };
    use edtui::{EditorMode, Index2};

    #[test]
    fn transcript_component_renders_structured_entries() {
        let mut transcript = TranscriptModel::default();
        transcript.push_input(Some(1), "x = 1");
        transcript.push_execute_result(Some(1), MimeBundle::plain("1"));

        let rows = TranscriptComponent::new(&transcript).render(80);
        let text = rows
            .iter()
            .map(|row| strip_ansi(&row.text))
            .collect::<Vec<_>>();

        assert_eq!(text, vec!["In [1]: x = 1", "Out[1]: 1"]);
        assert!(
            rows[1].text.starts_with("\u{1b}[31mOut[1]\u{1b}[0m: "),
            "Out prompt was not styled: {:?}",
            rows[1].text
        );
    }

    #[test]
    fn transcript_component_coalesces_stream_rows_from_model() {
        let mut transcript = TranscriptModel::default();
        transcript.push_stream(StreamName::Stdout, "a");
        transcript.push_stream(StreamName::Stdout, "b");

        let rows = TranscriptComponent::new(&transcript).render(80);

        assert_eq!(rows, vec![RenderedLine::new("ab")]);
    }

    #[test]
    fn transcript_component_preserves_raw_stream_ansi() {
        let mut transcript = TranscriptModel::default();
        transcript.push_stream(StreamName::Stdout, "\u{1b}[31mred\u{1b}[0m");

        let rows = TranscriptComponent::new(&transcript).render(80);

        assert_eq!(strip_ansi(&rows[0].text), "red");
        assert!(rows[0].text.contains("\u{1b}[31mred\u{1b}[0m"));
    }

    #[test]
    fn transcript_component_hides_password_stdin_values() {
        let mut transcript = TranscriptModel::default();
        let index = transcript.push_stdin("Password: ".to_string(), true);
        transcript.fill_stdin_value(index, "secret".to_string());

        let rows = TranscriptComponent::new(&transcript).render(80);
        let text = rows
            .iter()
            .map(|row| strip_ansi(&row.text))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            text.contains("Password: "),
            "missing password prompt:\n{text}"
        );
        assert!(
            !text.contains("secret"),
            "password leaked into transcript:\n{text}"
        );
    }

    #[test]
    fn editor_component_marks_cursor_on_last_rendered_row() {
        let editor = EditorModel {
            text: "abcdef".to_string(),
            prompt: "In [1]: ".to_string(),
            ..EditorModel::default()
        };

        let rows = EditorComponent::new(&editor).render(5);

        assert_eq!(
            rows.iter()
                .map(|row| strip_ansi(&row.text))
                .collect::<Vec<_>>(),
            vec!["In [1", "]: ab", "cdef"]
        );
        assert_eq!(
            rows.last()
                .and_then(|row| row.cursor_marker)
                .map(|marker| marker.position),
            Some(Position::new(4, 2))
        );
    }

    #[test]
    fn editor_component_styles_live_ipython_prompt() {
        let editor = EditorModel {
            text: "x = 1".to_string(),
            prompt: "In [3]: ".to_string(),
            ..EditorModel::default()
        };

        let rows = EditorComponent::new(&editor).render(80);

        assert_eq!(strip_ansi(&rows[0].text), "In [3]: x = 1");
        assert!(rows[0].text.starts_with("\u{1b}[36mIn [3]: \u{1b}[0m"));
    }

    #[test]
    fn editor_and_transcript_use_same_python_highlight_colors() {
        let editor = EditorModel {
            text: "time.sleep(1)".to_string(),
            prompt: "In [1]: ".to_string(),
            ..EditorModel::default()
        };
        let editor_rows = EditorComponent::new(&editor).render(80);
        let transcript = highlighted_execute_input(Some(1), "time.sleep(1)");

        assert_eq!(
            rgb_sequences(after_prompt_reset(&editor_rows[0].text)),
            rgb_sequences(after_prompt_reset(&transcript))
        );
    }

    #[test]
    fn editor_component_cursor_position_ignores_prompt_and_syntax_ansi() {
        let editor = EditorModel {
            text: "time.sleep(1)".to_string(),
            prompt: "In [1]: ".to_string(),
            ..EditorModel::default()
        };

        let rows = EditorComponent::new(&editor).render(80);

        assert!(rows[0].text.contains("\u{1b}[36mIn [1]: \u{1b}[0m"));
        assert!(rows[0].text.contains("\u{1b}[38;2;"));
        assert_eq!(
            rows[0].cursor_marker.map(|marker| marker.position),
            Some(Position::new("In [1]: time.sleep(1)".len() as u16, 0))
        );
    }

    #[test]
    fn editor_component_uses_edtui_render_plan_for_normal_widths() {
        let editor = EditorModel {
            text: "abcdef".to_string(),
            prompt: "In [1]: ".to_string(),
            ..EditorModel::default()
        };

        let rows = EditorComponent::new(&editor).render(12);

        assert_eq!(
            rows.iter()
                .map(|row| strip_ansi(&row.text))
                .collect::<Vec<_>>(),
            vec!["In [1]: abcd", "        ef"]
        );
        assert_eq!(
            rows.last()
                .and_then(|row| row.cursor_marker)
                .map(|marker| marker.position),
            Some(Position::new(10, 1))
        );
    }

    fn after_prompt_reset(text: &str) -> &str {
        text.split_once("\u{1b}[0m")
            .map(|(_, rest)| rest)
            .unwrap_or(text)
    }

    fn rgb_sequences(text: &str) -> Vec<String> {
        let mut colors = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find("\u{1b}[38;2;") {
            rest = &rest[start + "\u{1b}[".len()..];
            let Some(end) = rest.find('m') else {
                break;
            };
            colors.push(rest[..=end].to_string());
            rest = &rest[end + 1..];
        }
        colors
    }

    #[test]
    fn editor_component_renders_multiline_prompt_continuation_padding() {
        let editor = EditorModel {
            text: "abc\ndef".to_string(),
            prompt: "In [3]: ".to_string(),
            ..EditorModel::default()
        };

        let rows = EditorComponent::new(&editor).render(80);

        assert_eq!(
            rows.iter()
                .map(|row| strip_ansi(&row.text))
                .collect::<Vec<_>>(),
            vec!["In [3]: abc", "        def"]
        );
    }

    #[test]
    fn editor_component_renders_stdin_prompt_without_line_number_gutter() {
        let editor = EditorModel {
            text: "Ada".to_string(),
            prompt: "Name: ".to_string(),
            ..EditorModel::default()
        };

        let rows = EditorComponent::new(&editor).render(80);

        assert_eq!(strip_ansi(&rows[0].text), "Name: Ada");
        assert!(!rows[0].text.starts_with("\u{1b}[36m"));
    }

    #[test]
    fn editor_component_renders_password_prompt_without_extra_gutter() {
        let editor = EditorModel {
            text: "secret".to_string(),
            prompt: "Password: ".to_string(),
            ..EditorModel::default()
        };

        let rows = EditorComponent::new(&editor).render(80);

        assert_eq!(strip_ansi(&rows[0].text), "Password: secret");
    }

    #[test]
    fn editor_component_preserves_render_state_cursor_and_mode() {
        let mut render_state = build_editor_state("abcdef");
        render_state.cursor = Index2::new(0, 1);
        render_state.mode = EditorMode::Normal;
        let editor = EditorModel {
            text: "abcdef".to_string(),
            prompt: "In [1]: ".to_string(),
            render_state: Some(render_state),
        };

        let rows = EditorComponent::new(&editor).render(20);

        assert_eq!(
            rows.first()
                .and_then(|row| row.cursor_marker)
                .map(|marker| marker.position),
            Some(Position::new(9, 0))
        );
        assert_eq!(
            rows.first()
                .and_then(|row| row.cursor_marker)
                .map(|marker| marker.style),
            Some(FrameCursorStyle::Block)
        );
    }

    #[test]
    fn footer_component_renders_optional_footer_text() {
        let footer = FooterModel {
            text: Some("busy".to_string()),
        };

        let rows = FooterComponent::new(&footer).render(80);

        assert_eq!(rows, vec![RenderedLine::new("busy")]);
    }

    #[test]
    fn footer_component_styles_mode_badge_and_palette_hint() {
        let footer = FooterModel {
            text: Some("INS In [2] Ctrl-P palette".to_string()),
        };

        let rows = FooterComponent::new(&footer).render(80);

        assert_eq!(strip_ansi(&rows[0].text), " INS  In [2] Ctrl-P palette");
        assert!(rows[0].text.contains("\u{1b}[30m\u{1b}[46m INS \u{1b}[0m"));
        assert!(rows[0].text.contains("\u{1b}[90mCtrl-P palette\u{1b}[0m"));
    }

    #[test]
    fn footer_component_wraps_before_rendering_style_ansi() {
        let footer = FooterModel {
            text: Some("INS In [123] Ctrl-P palette".to_string()),
        };

        let rows = FooterComponent::new(&footer).render(15);
        let text = rows
            .iter()
            .map(|row| strip_ansi(&row.text))
            .collect::<Vec<_>>();

        assert_eq!(text, vec![" INS  In [123] ", "Ctrl-P palette"]);
        assert!(
            rows[0]
                .text
                .starts_with("\u{1b}[30m\u{1b}[46m INS \u{1b}[0m")
        );
        assert!(
            rows[1]
                .text
                .starts_with("\u{1b}[90mCtrl-P palette\u{1b}[0m")
        );
    }

    #[test]
    fn overlay_component_renders_palette_selection() {
        let overlay = OverlayModel::Palette(crate::ui::display::PaletteOverlayModel {
            items: vec!["Quit".to_string(), "Restart".to_string()],
            selected: 1,
        });

        let rows = OverlayComponent::new(&overlay).render(80);

        assert_eq!(
            rows,
            vec![
                RenderedLine::new("Command Palette"),
                RenderedLine::new("  Quit"),
                RenderedLine::new("> Restart"),
            ]
        );
    }

    #[test]
    fn overlay_component_renders_history_search_summary_and_preview() {
        let overlay = OverlayModel::HistorySearch(crate::ui::display::HistorySearchOverlayModel {
            query: "fib".to_string(),
            results: vec![crate::ui::display::HistorySearchResultModel {
                summary: "def fibonacci(n): ...".to_string(),
                selected: true,
            }],
            selected: 0,
            preview_lines: vec!["def fibonacci(n):".to_string(), "    return n".to_string()],
        });

        let rows = OverlayComponent::new(&overlay).render(80);

        assert_eq!(
            rows,
            vec![
                RenderedLine::new("History Search"),
                RenderedLine::new("query: fib"),
                RenderedLine::new("> def fibonacci(n): ..."),
                RenderedLine::new("preview"),
                RenderedLine::new("def fibonacci(n):"),
                RenderedLine::new("    return n"),
            ]
        );
    }

    #[test]
    fn overlay_component_preserves_history_result_ansi_highlighting() {
        let overlay = OverlayModel::HistorySearch(crate::ui::display::HistorySearchOverlayModel {
            query: "x".to_string(),
            results: vec![crate::ui::display::HistorySearchResultModel {
                summary: "\u{1b}[38;2;1;2;3mx\u{1b}[0m = 1".to_string(),
                selected: true,
            }],
            selected: 0,
            preview_lines: Vec::new(),
        });

        let rows = OverlayComponent::new(&overlay).render(80);
        let result = rows
            .iter()
            .find(|row| strip_ansi(&row.text) == "> x = 1")
            .expect("highlighted result row");

        assert!(result.text.contains("\u{1b}[38;2;1;2;3m"));
    }

    #[test]
    fn overlay_component_preserves_history_preview_ansi_highlighting() {
        let overlay = OverlayModel::HistorySearch(crate::ui::display::HistorySearchOverlayModel {
            query: "x".to_string(),
            results: vec![crate::ui::display::HistorySearchResultModel {
                summary: "x = 1".to_string(),
                selected: true,
            }],
            selected: 0,
            preview_lines: vec!["\u{1b}[38;2;1;2;3mx\u{1b}[0m = 1".to_string()],
        });

        let rows = OverlayComponent::new(&overlay).render(80);
        let preview = rows
            .iter()
            .find(|row| strip_ansi(&row.text) == "x = 1")
            .expect("highlighted preview row");

        assert!(preview.text.contains("\u{1b}[38;2;1;2;3m"));
    }
}
