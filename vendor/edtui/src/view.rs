mod internal;
pub(crate) mod line_wrapper;
mod render_line;
pub mod status_line;
#[cfg(feature = "syntax-highlighting")]
pub mod syntax_higlighting;
pub mod theme;

use render_line::RenderLine;
#[cfg(feature = "syntax-highlighting")]
use syntax_higlighting::SyntaxHighlighter;

use crate::{
    helper::max_col,
    state::{highlight::Highlight, selection::Selection, EditorState},
    EditorMode, Index2,
};

#[cfg(feature = "syntax-highlighting")]
use internal::highlighted_spans_with_selections;
use internal::line_into_spans_with_selections;
use jagged::index::RowIndex;
use line_wrapper::LineWrapper;
use ratatui_core::{
    buffer::Buffer,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};
pub use status_line::EditorStatusLine;
use theme::EditorTheme;

/// Configuration for line numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineNumbers {
    /// Line numbers are disabled (default).
    #[default]
    None,
    /// Display absolute line numbers.
    Absolute,
    /// Display relative line numbers.
    Relative,
}

/// Terminal-agnostic render output for an [`EditorView`].
///
/// This exposes the rows, styles, cursor, and viewport metadata that the
/// `ratatui` widget also uses to paint itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorRenderPlan {
    pub rows: Vec<EditorRenderRow>,
    pub cursor: Option<EditorRenderCursor>,
    pub viewport_offset: (usize, usize),
    pub screen_area: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorRenderRow {
    pub spans: Vec<Span<'static>>,
    pub gutter: Option<Span<'static>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorRenderCursor {
    pub position: Position,
    pub style: Style,
}

/// Creates the view for the editor. [`EditorView`] and [`EditorState`] are
/// the core classes of edtui.
///
/// ## Example
///
/// ```rust
/// use edtui::EditorState;
/// use edtui::EditorTheme;
/// use edtui::EditorView;
///
/// let theme = EditorTheme::default();
/// let mut state = EditorState::default();
///
/// EditorView::new(&mut state)
///     .wrap(true)
///     .theme(theme);
/// ```
pub struct EditorView<'a, 'b> {
    /// The editor state.
    pub(crate) state: &'a mut EditorState,

    /// The editor theme.
    pub(crate) theme: EditorTheme<'b>,

    /// An optional syntax highlighter.
    #[cfg(feature = "syntax-highlighting")]
    pub(crate) syntax_highlighter: Option<SyntaxHighlighter>,
}

impl<'a, 'b> EditorView<'a, 'b> {
    /// Creates a new instance of [`EditorView`].
    #[must_use]
    pub fn new(state: &'a mut EditorState) -> Self {
        Self {
            state,
            theme: EditorTheme::default(),
            #[cfg(feature = "syntax-highlighting")]
            syntax_highlighter: None,
        }
    }

    /// Set the theme for the [`EditorView`]
    /// See [`EditorTheme`] for the customizable parameters.
    #[must_use]
    pub fn theme(mut self, theme: EditorTheme<'b>) -> Self {
        self.theme = theme;
        self
    }

    #[cfg(feature = "syntax-highlighting")]
    /// Set the syntax highlighter for the [`EditorView`]
    /// See [`SyntaxHighlighter`] for the more information.
    ///
    /// ```rust
    /// use edtui::EditorState;
    /// use edtui::EditorView;
    /// use edtui::SyntaxHighlighter;
    ///
    /// let mut state = EditorState::default();
    /// let syntax_highlighter = SyntaxHighlighter::new("dracula", "rs").unwrap();
    ///
    /// EditorView::new(&mut state).syntax_highlighter(Some(syntax_highlighter));
    /// ```
    #[must_use]
    pub fn syntax_highlighter(mut self, syntax_highlighter: Option<SyntaxHighlighter>) -> Self {
        self.syntax_highlighter = syntax_highlighter;
        self
    }

    /// Enables single-line mode, which blocks newline insertion.
    ///
    /// When enabled, pressing Enter, Ctrl+J, Ctrl+M, or any other key combination
    /// that would insert a newline will be ignored. This is useful for search boxes,
    /// single-line input fields, and similar use cases.
    ///
    /// # Example
    ///
    /// ```rust
    /// use edtui::{EditorState, EditorView};
    ///
    /// let mut state = EditorState::default();
    ///
    /// // Create a single-line input field
    /// EditorView::new(&mut state).single_line(true);
    /// ```
    #[must_use]
    pub fn single_line(self, single_line: bool) -> Self {
        self.state.view.single_line = single_line;
        self
    }

    /// Sets whether overflowing lines should wrap onto the next line.
    ///
    /// # Note
    /// Line wrapping currently has issues when used with mouse events.
    #[must_use]
    pub fn wrap(self, wrap: bool) -> Self {
        self.state.view.wrap = wrap;
        self
    }

    pub(super) fn get_wrap(&self) -> bool {
        self.state.view.wrap
    }

    /// Sets the number of spaces used for rendering tabs.
    #[must_use]
    pub fn tab_width(self, tab_width: usize) -> Self {
        self.state.view.tab_width = tab_width;
        self
    }

    /// Returns the tab width configuration.
    pub(super) fn get_tab_width(&self) -> usize {
        self.state.view.tab_width
    }

    /// Configures line numbers. Disabled by default.
    ///
    /// # Example
    ///
    /// ```rust
    /// use edtui::{EditorState, EditorView, LineNumbers};
    ///
    /// let mut state = EditorState::default();
    ///
    /// // Enable absolute line numbers
    /// EditorView::new(&mut state).line_numbers(LineNumbers::Absolute);
    ///
    /// // Enable relative line numbers
    /// EditorView::new(&mut state).line_numbers(LineNumbers::Relative);
    /// ```
    #[must_use]
    pub fn line_numbers(self, line_numbers: LineNumbers) -> Self {
        self.state.view.line_numbers = line_numbers;
        self
    }

    /// Returns the line numbers configuration.
    pub(super) fn get_line_numbers(&self) -> LineNumbers {
        self.state.view.line_numbers
    }

    /// Returns a reference to the [`EditorState`].
    #[must_use]
    pub fn get_state(&'a self) -> &'a EditorState {
        self.state
    }

    /// Returns a mutable reference to the [`EditorState`].
    #[must_use]
    pub fn get_state_mut(&'a mut self) -> &'a mut EditorState {
        self.state
    }

    /// Calculate the width needed for the line number gutter.
    fn line_number_width(&self) -> u16 {
        if self.state.view.line_numbers == LineNumbers::None {
            return 0;
        }

        let total_lines = self.state.lines.len().max(1);
        let digits = total_lines.to_string().len();
        (digits + 1) as u16
    }

    #[allow(clippy::too_many_lines)]
    pub fn render_plan(self, area: Rect) -> EditorRenderPlan {
        let area = match &self.theme.block {
            Some(b) => b.inner(area),
            None => area,
        };

        let [main, _status] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::from(self.theme.status_line.is_some())),
        ])
        .areas(area);

        let line_number_width = self.line_number_width();
        let (_gutter_area, content_main) = if line_number_width > 0 {
            let [gutter, content] =
                Layout::horizontal([Constraint::Length(line_number_width), Constraint::Min(0)])
                    .areas(main);
            (Some(gutter), content)
        } else {
            (None, main)
        };

        let width = content_main.width as usize;
        let height = content_main.height as usize;
        let wrap_lines = self.get_wrap();
        let tab_width = self.get_tab_width();
        let line_numbers = self.get_line_numbers();
        let lines = &self.state.lines;

        let max_col = max_col(&self.state.lines, &self.state.cursor, self.state.mode);
        let cursor = Index2::new(self.state.cursor.row, self.state.cursor.col.min(max_col));

        self.state.view.set_screen_area(content_main);

        let view_state = &mut self.state.view;
        let (offset_x, offset_y) = if wrap_lines {
            (
                0,
                view_state.update_viewport_vertical_wrap(width, height, cursor.row, lines),
            )
        } else {
            let line = lines.get(RowIndex::new(cursor.row));
            (
                view_state.update_viewport_horizontal(width, cursor.col, line),
                view_state.update_viewport_vertical(height, cursor.row),
            )
        };

        let mut search_selection: Option<Selection> = None;
        if self.state.mode == EditorMode::Search {
            search_selection = (&self.state.search).into();
        };
        let selections = vec![&self.state.selection, &search_selection];
        let pair_highlights = bracket_pair_highlights(lines, cursor);
        let mut highlights = self.state.highlights.clone();
        highlights.extend(pair_highlights);

        #[cfg(feature = "syntax-highlighting")]
        let syntax_spans = self.syntax_highlighter.as_ref().map(|syntax| {
            syntax.highlight_lines(
                lines.iter_row().map(|line| line.iter().collect::<String>()),
                &self.theme.base,
            )
        });

        let mut cursor_position: Option<Position> = None;
        let mut rows = Vec::new();
        let mut num_rendered_rows = 0;

        let line_numbers_enabled = line_numbers != LineNumbers::None;
        let is_relative = line_numbers == LineNumbers::Relative;

        let mut row_index = offset_y;
        for line in lines.iter_row().skip(row_index) {
            if rows.len() >= height {
                break;
            }

            let col_skips = offset_x;
            num_rendered_rows += 1;

            let spans = generate_spans(
                line,
                &selections,
                &highlights,
                row_index,
                col_skips,
                &self.theme.base,
                &self.theme.selection_style,
                #[cfg(feature = "syntax-highlighting")]
                syntax_spans
                    .as_ref()
                    .and_then(|spans_by_row| spans_by_row.get(row_index))
                    .map(Vec::as_slice),
            );

            let render_line = if wrap_lines {
                RenderLine::Wrapped(LineWrapper::wrap_spans(spans, width, tab_width))
            } else {
                RenderLine::Single(spans)
            };

            if row_index == cursor.row {
                let content_area = Rect::new(
                    content_main.x,
                    content_main.y + rows.len() as u16,
                    content_main.width,
                    content_main.height.saturating_sub(rows.len() as u16),
                );
                cursor_position = Some(render_line.data_coordinate_to_screen_coordinate(
                    cursor.col.saturating_sub(offset_x),
                    content_area,
                    tab_width,
                ));
            }

            let gutter = if line_numbers_enabled {
                let is_cursor_line = row_index == cursor.row;
                let line_num = if is_relative {
                    if is_cursor_line {
                        row_index + 1
                    } else {
                        row_index.abs_diff(cursor.row)
                    }
                } else {
                    row_index + 1
                };

                let num_str = if is_relative && is_cursor_line {
                    format!(
                        "{:<width$}",
                        line_num,
                        width = (line_number_width - 1) as usize
                    )
                } else {
                    format!(
                        "{:>width$}",
                        line_num,
                        width = (line_number_width - 1) as usize
                    )
                };
                Some(Span::styled(num_str, self.theme.line_numbers_style))
            } else {
                None
            };

            for (index, spans) in render_line.into_rows(tab_width).into_iter().enumerate() {
                if rows.len() >= height {
                    break;
                }
                rows.push(EditorRenderRow {
                    spans,
                    gutter: (index == 0).then(|| gutter.clone()).flatten(),
                });
            }

            row_index += 1;
        }

        let final_cursor_position = cursor_position.unwrap_or(Position::new(
            content_main.left(),
            content_main.top() + self.state.cursor.row as u16,
        ));

        self.state.view.cursor_screen_position = Some(final_cursor_position);
        self.state.view.update_num_rows(num_rendered_rows);

        EditorRenderPlan {
            rows,
            cursor: Some(EditorRenderCursor {
                position: final_cursor_position,
                style: self.theme.cursor_style,
            }),
            viewport_offset: (offset_x, offset_y),
            screen_area: content_main,
        }
    }
}

impl Widget for EditorView<'_, '_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Draw the border.
        buf.set_style(area, self.theme.base);
        let block = self.theme.block.clone();
        let status_line = self.theme.status_line.clone();
        let mode = self.state.mode;
        let search = (mode == EditorMode::Search).then(|| self.state.search_pattern());
        let line_numbers_style = self.theme.line_numbers_style;

        let area = match &block {
            Some(b) => {
                let inner_area = b.inner(area);
                b.clone().render(area, buf);
                inner_area
            }
            None => area,
        };

        let [main, status] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::from(status_line.is_some())),
        ])
        .areas(area);

        let plan = self.render_plan(area);
        let line_number_width = plan.screen_area.x.saturating_sub(main.x);
        let (gutter_area, content_main) = if line_number_width > 0 {
            let [gutter, content] =
                Layout::horizontal([Constraint::Length(line_number_width), Constraint::Min(0)])
                    .areas(main);
            // Fill the entire gutter with the line numbers style
            buf.set_style(gutter, line_numbers_style);
            (Some(gutter), content)
        } else {
            (None, main)
        };

        for (row_index, row) in plan.rows.into_iter().enumerate() {
            let y = content_main.y + row_index as u16;
            if y >= content_main.bottom() {
                break;
            }
            Line::from(row.spans).render(Rect::new(content_main.x, y, content_main.width, 1), buf);
            if let (Some(gutter), Some(gutter_area)) = (row.gutter, gutter_area) {
                buf.set_span(gutter_area.x, y, &gutter, gutter_area.width);
            }
        }

        if let Some(cursor) = plan.cursor {
            if let Some(cell) = buf.cell_mut(cursor.position) {
                cell.set_style(cursor.style);
            }
        }

        if let Some(s) = status_line {
            s.mode(mode.name()).search(search).render(status, buf);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_spans<'a>(
    line: &[char],
    selections: &[&Option<Selection>],
    highlights: &[Highlight],
    row_index: usize,
    col_skips: usize,
    base_style: &Style,
    selection_style: &Style,
    #[cfg(feature = "syntax-highlighting")] syntax_spans: Option<&[internal::InternalSpan]>,
) -> Vec<Span<'a>> {
    #[cfg(feature = "syntax-highlighting")]
    if let Some(syntax_spans) = syntax_spans {
        return highlighted_spans_with_selections(
            syntax_spans,
            selections,
            highlights,
            row_index,
            col_skips,
            selection_style,
        );
    }
    line_into_spans_with_selections(
        line,
        selections,
        highlights,
        row_index,
        col_skips,
        base_style,
        selection_style,
    )
}

fn bracket_pair_highlights(lines: &jagged::Jagged<char>, cursor: Index2) -> Vec<Highlight> {
    let style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    let candidates = [
        Some(cursor),
        cursor
            .col
            .checked_sub(1)
            .map(|col| Index2::new(cursor.row, col)),
    ];

    for position in candidates.into_iter().flatten() {
        let Some(ch) = char_at(lines, position) else {
            continue;
        };
        let Some(matching) = find_matching_bracket(lines, position, ch) else {
            continue;
        };
        return vec![
            Highlight::new(position, position, style),
            Highlight::new(matching, matching, style),
        ];
    }

    Vec::new()
}

fn char_at(lines: &jagged::Jagged<char>, position: Index2) -> Option<char> {
    lines
        .iter_row()
        .nth(position.row)
        .and_then(|line| line.get(position.col))
        .copied()
}

fn find_matching_bracket(
    lines: &jagged::Jagged<char>,
    position: Index2,
    ch: char,
) -> Option<Index2> {
    let (open, close, forward) = match ch {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => ('(', ')', false),
        ']' => ('[', ']', false),
        '}' => ('{', '}', false),
        _ => return None,
    };

    let mut positions: Vec<(Index2, char)> = lines
        .iter_row()
        .enumerate()
        .flat_map(|(row, line)| {
            line.iter()
                .copied()
                .enumerate()
                .map(move |(col, ch)| (Index2::new(row, col), ch))
        })
        .collect();

    if !forward {
        positions.reverse();
    }

    let start = positions.iter().position(|(pos, _)| *pos == position)?;
    let mut depth = 0usize;
    for (pos, current) in positions.into_iter().skip(start + 1) {
        if forward {
            if current == open {
                depth += 1;
            } else if current == close {
                if depth == 0 {
                    return Some(pos);
                }
                depth -= 1;
            }
        } else if current == close {
            depth += 1;
        } else if current == open {
            if depth == 0 {
                return Some(pos);
            }
            depth -= 1;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorState, Lines};
    use ratatui_core::{buffer::Buffer, widgets::Widget};

    fn row_text(row: &EditorRenderRow) -> String {
        row.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn render_plan_exposes_rows_cursor_and_viewport() {
        let mut state = EditorState::new(Lines::from("abc\ndef"));
        state.cursor = Index2::new(1, 2);

        let plan = EditorView::new(&mut state)
            .theme(EditorTheme::default().hide_status_line())
            .render_plan(Rect::new(0, 0, 20, 4));

        assert_eq!(
            plan.rows.iter().map(row_text).collect::<Vec<_>>(),
            vec!["abc".to_string(), "def".to_string()]
        );
        assert_eq!(
            plan.cursor.map(|cursor| cursor.position),
            Some(Position::new(2, 1))
        );
        assert_eq!(plan.viewport_offset, (0, 0));
        assert_eq!(plan.screen_area, Rect::new(0, 0, 20, 4));
    }

    #[test]
    fn bracket_pair_highlights_match_adjacent_pairs() {
        let mut state = EditorState::new(Lines::from("foo(bar)\n[x]"));
        state.cursor = Index2::new(0, 3);

        let plan = EditorView::new(&mut state)
            .theme(EditorTheme::default().hide_status_line())
            .render_plan(Rect::new(0, 0, 20, 2));

        assert_eq!(plan.rows[0].spans[1].content.as_ref(), "(");
        assert_eq!(plan.rows[0].spans[3].content.as_ref(), ")");
    }

    #[test]
    fn widget_renders_content_from_render_plan_rows() {
        let mut state = EditorState::new(Lines::from("abc"));
        let mut expected_state = state.clone();
        let plan = EditorView::new(&mut expected_state)
            .theme(EditorTheme::default().hide_status_line())
            .render_plan(Rect::new(0, 0, 20, 1));

        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 1));
        EditorView::new(&mut state)
            .theme(EditorTheme::default().hide_status_line())
            .render(Rect::new(0, 0, 20, 1), &mut buffer);

        let rendered = (0..3).map(|x| buffer[(x, 0)].symbol()).collect::<String>();
        assert_eq!(rendered, row_text(&plan.rows[0]));
    }
}
