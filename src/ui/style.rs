use ratatui::style::{Color, Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UiStyle {
    InputPrompt,
    OutputPrompt,
    Runtime,
    FooterHint,
    ModeInsert,
    ModeNormal,
    ModeVisual,
    ModeSearch,
    Selection,
    Plain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextStyle {
    Semantic(UiStyle),
    Raw(Style),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StyledSegment {
    pub text: String,
    pub style: TextStyle,
}

impl StyledSegment {
    pub(super) fn semantic(text: impl Into<String>, style: UiStyle) -> Self {
        Self {
            text: text.into(),
            style: TextStyle::Semantic(style),
        }
    }

    pub(super) fn raw(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style: TextStyle::Raw(style),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StyledLine {
    pub segments: Vec<StyledSegment>,
}

impl StyledLine {
    pub(super) fn new(segments: Vec<StyledSegment>) -> Self {
        Self { segments }
    }
}

pub(super) fn semantic_style(style: UiStyle) -> Style {
    match style {
        UiStyle::InputPrompt => Style::default().fg(Color::Cyan),
        UiStyle::OutputPrompt => Style::default().fg(Color::Red),
        UiStyle::Runtime => Style::default().fg(Color::Indexed(244)),
        UiStyle::FooterHint => Style::default().fg(Color::DarkGray),
        UiStyle::ModeInsert => Style::default().fg(Color::Black).bg(Color::Cyan),
        UiStyle::ModeNormal => Style::default().fg(Color::Black).bg(Color::Yellow),
        UiStyle::ModeVisual => Style::default().fg(Color::Black).bg(Color::Magenta),
        UiStyle::ModeSearch => Style::default().fg(Color::Black).bg(Color::Green),
        UiStyle::Selection => Style::default().bg(Color::DarkGray),
        UiStyle::Plain => Style::default(),
    }
}

pub(super) fn text_style(style: TextStyle) -> Style {
    match style {
        TextStyle::Semantic(style) => semantic_style(style),
        TextStyle::Raw(style) => style,
    }
}

pub(super) fn render_styled_line(line: &StyledLine) -> String {
    let mut rendered = String::new();
    for segment in &line.segments {
        rendered.push_str(&render_styled_segment(&segment.text, segment.style));
    }
    rendered
}

pub(super) fn styled_line_width(line: &StyledLine) -> usize {
    line.segments
        .iter()
        .map(|segment| UnicodeWidthStr::width(segment.text.as_str()))
        .sum()
}

pub(super) fn wrap_styled_line(line: &StyledLine, width: u16) -> Vec<StyledLine> {
    let width = width.max(1) as usize;
    let mut rows = Vec::with_capacity(styled_line_width(line).div_ceil(width).max(1));
    let mut current = Vec::new();
    let mut current_width = 0usize;

    for segment in &line.segments {
        let mut chunk = String::new();
        for ch in segment.text.chars() {
            if ch == '\n' {
                if !chunk.is_empty() {
                    current.push(StyledSegment {
                        text: std::mem::take(&mut chunk),
                        style: segment.style,
                    });
                }
                rows.push(StyledLine::new(std::mem::take(&mut current)));
                current_width = 0;
                continue;
            }

            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if char_width > 0 && current_width > 0 && current_width + char_width > width {
                if !chunk.is_empty() {
                    current.push(StyledSegment {
                        text: std::mem::take(&mut chunk),
                        style: segment.style,
                    });
                }
                rows.push(StyledLine::new(std::mem::take(&mut current)));
                current_width = 0;
            }

            chunk.push(ch);
            current_width += char_width;
        }

        if !chunk.is_empty() {
            current.push(StyledSegment {
                text: chunk,
                style: segment.style,
            });
        }
    }

    if !current.is_empty() || rows.is_empty() {
        rows.push(StyledLine::new(current));
    }

    rows
}

pub(super) fn render_styled_segment(text: &str, style: TextStyle) -> String {
    let style = text_style(style);
    if style == Style::default() {
        return text.to_string();
    }

    format!("{}{}\x1b[0m", style_to_ansi(style), text)
}

pub(super) fn style_to_ansi(style: Style) -> String {
    let mut rendered = String::new();
    if let Some(fg) = style.fg {
        rendered.push_str(&ansi_color(fg, false));
    }
    if let Some(bg) = style.bg {
        rendered.push_str(&ansi_color(bg, true));
    }
    rendered.push_str(&modifier_ansi(style.add_modifier));
    rendered
}

fn modifier_ansi(modifier: Modifier) -> String {
    let mut rendered = String::new();
    if modifier.contains(Modifier::BOLD) {
        rendered.push_str("\x1b[1m");
    }
    if modifier.contains(Modifier::ITALIC) {
        rendered.push_str("\x1b[3m");
    }
    if modifier.contains(Modifier::UNDERLINED) {
        rendered.push_str("\x1b[4m");
    }
    if modifier.contains(Modifier::REVERSED) {
        rendered.push_str("\x1b[7m");
    }
    rendered
}

fn ansi_color(color: Color, background: bool) -> String {
    let base = if background { 40 } else { 30 };
    match color {
        Color::Reset => {
            let code = if background { 49 } else { 39 };
            format!("\x1b[{code}m")
        }
        Color::Black => format!("\x1b[{base}m"),
        Color::Red => format!("\x1b[{}m", base + 1),
        Color::Green => format!("\x1b[{}m", base + 2),
        Color::Yellow => format!("\x1b[{}m", base + 3),
        Color::Blue => format!("\x1b[{}m", base + 4),
        Color::Magenta => format!("\x1b[{}m", base + 5),
        Color::Cyan => format!("\x1b[{}m", base + 6),
        Color::Gray => format!("\x1b[{}m", base + 7),
        Color::DarkGray => format!("\x1b[{}m", if background { 100 } else { 90 }),
        Color::LightRed => format!("\x1b[{}m", if background { 101 } else { 91 }),
        Color::LightGreen => format!("\x1b[{}m", if background { 102 } else { 92 }),
        Color::LightYellow => format!("\x1b[{}m", if background { 103 } else { 93 }),
        Color::LightBlue => format!("\x1b[{}m", if background { 104 } else { 94 }),
        Color::LightMagenta => format!("\x1b[{}m", if background { 105 } else { 95 }),
        Color::LightCyan => format!("\x1b[{}m", if background { 106 } else { 96 }),
        Color::White => format!("\x1b[{}m", if background { 107 } else { 97 }),
        Color::Indexed(index) => {
            let code = if background { 48 } else { 38 };
            format!("\x1b[{code};5;{index}m")
        }
        Color::Rgb(red, green, blue) => {
            let code = if background { 48 } else { 38 };
            format!("\x1b[{code};2;{red};{green};{blue}m")
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier, Style};

    use super::{
        StyledLine, StyledSegment, TextStyle, UiStyle, render_styled_line, render_styled_segment,
        semantic_style, style_to_ansi, styled_line_width, wrap_styled_line,
    };

    #[test]
    fn maps_semantic_styles_to_ratatui_styles() {
        assert_eq!(semantic_style(UiStyle::InputPrompt).fg, Some(Color::Cyan));
        assert_eq!(semantic_style(UiStyle::OutputPrompt).fg, Some(Color::Red));
        assert_eq!(
            semantic_style(UiStyle::Runtime).fg,
            Some(Color::Indexed(244))
        );
        assert_eq!(
            semantic_style(UiStyle::FooterHint).fg,
            Some(Color::DarkGray)
        );
        assert_eq!(semantic_style(UiStyle::ModeInsert).bg, Some(Color::Cyan));
    }

    #[test]
    fn converts_ratatui_style_to_ansi() {
        let style = Style::default()
            .fg(Color::Rgb(1, 2, 3))
            .bg(Color::Indexed(4))
            .add_modifier(Modifier::BOLD | Modifier::ITALIC);

        assert_eq!(
            style_to_ansi(style),
            "\x1b[38;2;1;2;3m\x1b[48;5;4m\x1b[1m\x1b[3m"
        );
    }

    #[test]
    fn reset_colors_use_targeted_sgr_codes() {
        assert_eq!(style_to_ansi(Style::default().fg(Color::Reset)), "\x1b[39m");
        assert_eq!(style_to_ansi(Style::default().bg(Color::Reset)), "\x1b[49m");
    }

    #[test]
    fn renders_whole_line_with_resets_between_segments() {
        let rendered = render_styled_line(&StyledLine::new(vec![
            StyledSegment::semantic("in", UiStyle::InputPrompt),
            StyledSegment::raw("raw", Style::default().fg(Color::Red)),
            StyledSegment::semantic("plain", UiStyle::Plain),
        ]));

        assert_eq!(rendered, "\x1b[36min\x1b[0m\x1b[31mraw\x1b[0mplain");
    }

    #[test]
    fn renders_plain_after_styled_without_style_bleed() {
        let rendered = render_styled_line(&StyledLine::new(vec![
            StyledSegment::semantic("styled", UiStyle::OutputPrompt),
            StyledSegment::semantic("plain", UiStyle::Plain),
        ]));

        assert_eq!(rendered, "\x1b[31mstyled\x1b[0mplain");
    }

    #[test]
    fn resets_line_end_for_styled_segment() {
        let rendered = render_styled_segment("x", TextStyle::Semantic(UiStyle::InputPrompt));

        assert!(rendered.ends_with("\x1b[0m"));
    }

    #[test]
    fn measures_styled_line_by_visible_width() {
        let line = StyledLine::new(vec![
            StyledSegment::semantic("ab", UiStyle::InputPrompt),
            StyledSegment::raw("語", Style::default().fg(Color::Red)),
        ]);

        assert_eq!(styled_line_width(&line), 4);
    }

    #[test]
    fn wraps_styled_line_preserving_segment_styles() {
        let rows = wrap_styled_line(
            &StyledLine::new(vec![
                StyledSegment::semantic("abc", UiStyle::InputPrompt),
                StyledSegment::semantic("def", UiStyle::OutputPrompt),
            ]),
            4,
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].segments[0].text, "abc");
        assert_eq!(rows[0].segments[1].text, "d");
        assert_eq!(
            rows[0].segments[1].style,
            TextStyle::Semantic(UiStyle::OutputPrompt)
        );
        assert_eq!(rows[1].segments[0].text, "ef");
        assert_eq!(styled_line_width(&rows[0]), 4);
        assert_eq!(styled_line_width(&rows[1]), 2);
    }

    #[test]
    fn wraps_styled_line_on_newlines() {
        let rows = wrap_styled_line(
            &StyledLine::new(vec![StyledSegment::semantic("a\nb", UiStyle::InputPrompt)]),
            10,
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].segments[0].text, "a");
        assert_eq!(rows[1].segments[0].text, "b");
    }
}
