use std::sync::LazyLock;

use edtui::syntect::{
    easy::HighlightLines,
    highlighting::ThemeSet,
    parsing::SyntaxSet,
    util::{LinesWithEndings, as_24_bit_terminal_escaped},
};

const TRANSCRIPT_THEME_NAME: &str = "base16-ocean.dark";
const PROMPT_ANSI: &str = "\x1b[36m";
const ANSI_RESET: &str = "\x1b[0m";

static TRANSCRIPT_SYNTAX_SET: LazyLock<SyntaxSet> =
    LazyLock::new(SyntaxSet::load_defaults_newlines);
static TRANSCRIPT_THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

pub(super) fn highlighted_execute_input(execution_count: Option<u32>, code: &str) -> String {
    let prompt = execution_count
        .map(|count| format!("In [{count}]: "))
        .unwrap_or_else(|| "In [?]: ".to_string());
    let continuation = format!("{:>width$}", "...: ", width = prompt.len());
    let syntax = TRANSCRIPT_SYNTAX_SET
        .find_syntax_by_extension("py")
        .unwrap_or_else(|| TRANSCRIPT_SYNTAX_SET.find_syntax_plain_text());
    let theme = &TRANSCRIPT_THEME_SET.themes[TRANSCRIPT_THEME_NAME];
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut rendered = String::new();

    for (index, line) in LinesWithEndings::from(code).enumerate() {
        if index > 0 {
            rendered.push_str(PROMPT_ANSI);
            rendered.push_str(&continuation);
            rendered.push_str(ANSI_RESET);
        } else {
            rendered.push_str(PROMPT_ANSI);
            rendered.push_str(&prompt);
            rendered.push_str(ANSI_RESET);
        }

        match highlighter.highlight_line(line, &TRANSCRIPT_SYNTAX_SET) {
            Ok(ranges) => rendered.push_str(&as_24_bit_terminal_escaped(&ranges, false)),
            Err(_) => rendered.push_str(line),
        }
    }

    if rendered.is_empty() {
        rendered.push_str(PROMPT_ANSI);
        rendered.push_str(&prompt);
        rendered.push_str(ANSI_RESET);
    }

    rendered
}

pub(super) fn display_width(text: &str) -> usize {
    strip_ansi(text).chars().count()
}

pub(super) fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }

        result.push(ch);
    }

    result
}

#[cfg(test)]
pub(super) fn rendered_line_count(text: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut logical_lines = normalized.split('\n').collect::<Vec<_>>();
    if normalized.ends_with('\n') && logical_lines.len() > 1 {
        logical_lines.pop();
    }
    let mut line_count = 0usize;

    for line in logical_lines {
        let visible_width = strip_ansi(line).chars().count();
        line_count += visible_width.max(1).div_ceil(width);
    }

    line_count.clamp(1, u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::{highlighted_execute_input, rendered_line_count, strip_ansi};

    #[test]
    fn strips_basic_ansi_sequences() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
    }

    #[test]
    fn counts_wrapped_rendered_lines() {
        assert_eq!(rendered_line_count("abcdef", 3), 2);
        assert_eq!(rendered_line_count("a\nbc", 10), 2);
        assert_eq!(rendered_line_count("a\n", 10), 1);
    }

    #[test]
    fn highlights_execute_input_with_prompt_and_ansi() {
        let rendered = highlighted_execute_input(Some(2), "x = 1");
        assert!(rendered.contains("In [2]: "));
        assert!(rendered.contains("\u{1b}["));
    }

    #[test]
    fn highlights_multiline_execute_input_with_ipython_continuation_prompt() {
        let rendered = strip_ansi(&highlighted_execute_input(Some(2), "x = 1\ny = 2"));
        assert!(rendered.contains("In [2]: x = 1\n   ...: y = 2"));
    }
}
