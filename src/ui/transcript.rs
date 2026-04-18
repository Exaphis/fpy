use std::{sync::LazyLock, time::Duration};

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

pub(super) fn runtime_line(duration: Duration) -> String {
    format!("{PROMPT_ANSI}[{}]{ANSI_RESET}", format_runtime(duration))
}

fn format_runtime(duration: Duration) -> String {
    let elapsed = duration.as_secs_f64();

    if elapsed < 0.001 {
        let micros = elapsed * 1e6;
        let decimals = if micros >= 100.0 {
            0
        } else if micros >= 10.0 {
            1
        } else {
            2
        };
        format!("{micros:.decimals$}µs")
    } else if elapsed < 1.0 {
        let millis = elapsed * 1e3;
        let decimals = if millis >= 100.0 {
            0
        } else if millis >= 10.0 {
            1
        } else {
            2
        };
        format!("{millis:.decimals$}ms")
    } else if elapsed < 60.0 {
        let decimals = if elapsed >= 10.0 { 1 } else { 2 };
        format!("{elapsed:.decimals$}s")
    } else {
        let total_seconds = duration.as_secs();
        if total_seconds < 3600 {
            let minutes = total_seconds / 60;
            let seconds = total_seconds % 60;
            format!("{minutes}m{seconds:02}s")
        } else if total_seconds < 86_400 {
            let hours = total_seconds / 3600;
            let minutes = (total_seconds % 3600) / 60;
            format!("{hours}h{minutes:02}m")
        } else {
            let days = total_seconds / 86_400;
            let hours = (total_seconds % 86_400) / 3600;
            format!("{days}d{hours:02}h")
        }
    }
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
    use std::time::Duration;

    use super::{format_runtime, highlighted_execute_input, rendered_line_count, runtime_line, strip_ansi};

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

    #[test]
    fn formats_sub_millisecond_runtime_in_microseconds() {
        assert_eq!(format_runtime(Duration::from_nanos(456_000)), "456µs");
        assert_eq!(format_runtime(Duration::from_nanos(12_340)), "12.3µs");
        assert_eq!(format_runtime(Duration::from_nanos(1_230)), "1.23µs");
    }

    #[test]
    fn formats_sub_second_runtime_in_milliseconds() {
        assert_eq!(format_runtime(Duration::from_millis(456)), "456ms");
        assert_eq!(format_runtime(Duration::from_micros(12_340)), "12.3ms");
        assert_eq!(format_runtime(Duration::from_micros(1_230)), "1.23ms");
    }

    #[test]
    fn formats_seconds_minutes_hours_and_days_like_zsh_prompt() {
        assert_eq!(format_runtime(Duration::from_millis(1500)), "1.50s");
        assert_eq!(format_runtime(Duration::from_secs(12)), "12.0s");
        assert_eq!(format_runtime(Duration::from_secs(125)), "2m05s");
        assert_eq!(format_runtime(Duration::from_secs(3720)), "1h02m");
        assert_eq!(format_runtime(Duration::from_secs(176_400)), "2d01h");
    }

    #[test]
    fn renders_runtime_line_with_ansi() {
        let rendered = runtime_line(Duration::from_millis(42));
        assert!(rendered.contains("[42.0ms]"));
        assert!(rendered.contains("\u{1b}["));
    }
}
