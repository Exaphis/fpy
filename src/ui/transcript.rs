use std::time::Duration;

use edtui::syntect::util::LinesWithEndings;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    prompt::{continuation_prompt, input_prompt, styled_input_prompt},
    style::{StyledLine, StyledSegment, UiStyle, render_styled_line},
    syntax::highlighted_python_lines,
};

pub(super) fn highlighted_execute_input(execution_count: Option<u32>, code: &str) -> String {
    let prompt = input_prompt(execution_count);
    let continuation = continuation_prompt(&prompt);
    let highlighted = highlighted_python_lines(code, false);
    let mut rendered = String::new();

    for (index, line) in LinesWithEndings::from(code).enumerate() {
        if index > 0 {
            rendered.push_str(&styled_input_prompt(&continuation));
        } else {
            rendered.push_str(&styled_input_prompt(&prompt));
        }

        rendered.push_str(
            highlighted
                .as_ref()
                .and_then(|lines| lines.get(index))
                .map_or(line, String::as_str),
        );
    }

    if rendered.is_empty() {
        rendered.push_str(&styled_input_prompt(&prompt));
    }

    rendered
}

pub(super) fn runtime_line(duration: Duration) -> String {
    render_styled_line(&StyledLine::new(vec![StyledSegment::semantic(
        format!("[{}]", format_runtime(duration)),
        UiStyle::Runtime,
    )]))
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
    UnicodeWidthStr::width(strip_ansi(text).as_str())
}

pub(super) fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && consume_ansi_sequence(&mut chars) {
            continue;
        }

        result.push(ch);
    }

    result
}

pub(super) fn wrap_ansi_to_width(text: &str, width: u16) -> Vec<String> {
    let width = width.max(1) as usize;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut active_sgr = String::new();
    let mut chars = normalized.chars().peekable();

    let push_row = |rows: &mut Vec<String>,
                    current: &mut String,
                    current_width: &mut usize,
                    active_sgr: &str| {
        if !active_sgr.is_empty() && !current.is_empty() {
            current.push_str("\u{1b}[0m");
        }
        rows.push(std::mem::take(current));
        if !active_sgr.is_empty() {
            current.push_str(active_sgr);
        }
        *current_width = 0;
    };

    while let Some(ch) = chars.next() {
        if ch == '\n' {
            push_row(&mut rows, &mut current, &mut current_width, &active_sgr);
            continue;
        }

        if ch == '\u{1b}' {
            if let Some(sequence) = read_ansi_sequence(&mut chars) {
                update_active_sgr(&sequence, &mut active_sgr);
                current.push(ch);
                current.push_str(&sequence);
            } else {
                current.push(ch);
            }
            continue;
        }

        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if char_width > 0 && current_width > 0 && current_width + char_width > width {
            push_row(&mut rows, &mut current, &mut current_width, &active_sgr);
        }

        current.push(ch);
        current_width += char_width;
    }

    if !current.is_empty() || rows.is_empty() {
        if !active_sgr.is_empty() && !current.is_empty() {
            current.push_str("\u{1b}[0m");
        }
        rows.push(current);
    }

    rows
}

fn consume_ansi_sequence<I>(chars: &mut std::iter::Peekable<I>) -> bool
where
    I: Iterator<Item = char>,
{
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            true
        }
        Some(']') => {
            chars.next();
            let mut saw_escape = false;
            for next in chars.by_ref() {
                if next == '\u{7}' {
                    break;
                }
                if saw_escape && next == '\\' {
                    break;
                }
                saw_escape = next == '\u{1b}';
            }
            true
        }
        _ => false,
    }
}

fn read_ansi_sequence<I>(chars: &mut std::iter::Peekable<I>) -> Option<String>
where
    I: Iterator<Item = char>,
{
    let mut output = String::new();
    match chars.peek().copied() {
        Some('[') => {
            output.push(chars.next().expect("peeked CSI introducer"));
            for next in chars.by_ref() {
                output.push(next);
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            Some(output)
        }
        Some(']') => {
            output.push(chars.next().expect("peeked OSC introducer"));
            let mut saw_escape = false;
            for next in chars.by_ref() {
                output.push(next);
                if next == '\u{7}' {
                    break;
                }
                if saw_escape && next == '\\' {
                    break;
                }
                saw_escape = next == '\u{1b}';
            }
            Some(output)
        }
        _ => None,
    }
}

fn update_active_sgr(sequence: &str, active_sgr: &mut String) {
    if !sequence.starts_with('[') || !sequence.ends_with('m') {
        return;
    }
    let parameters = &sequence[1..sequence.len().saturating_sub(1)];
    if parameters.is_empty()
        || parameters
            .split(';')
            .any(|parameter| parameter.parse::<u16>().ok() == Some(0))
    {
        active_sgr.clear();
    } else {
        active_sgr.push('\u{1b}');
        active_sgr.push_str(sequence);
    }
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
        let visible_width = display_width(line);
        line_count += visible_width.max(1).div_ceil(width);
    }

    line_count.clamp(1, u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        display_width, format_runtime, highlighted_execute_input, rendered_line_count,
        runtime_line, strip_ansi, wrap_ansi_to_width,
    };

    #[test]
    fn strips_basic_ansi_sequences() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
    }

    #[test]
    fn strips_osc_hyperlink_sequences() {
        assert_eq!(
            strip_ansi("\u{1b}]8;;https://example.com\u{7}link\u{1b}]8;;\u{7}"),
            "link"
        );
    }

    #[test]
    fn display_width_counts_wide_characters_as_terminal_columns() {
        assert_eq!(display_width("λ語"), 3);
    }

    #[test]
    fn wraps_ansi_text_by_terminal_display_width() {
        let rows = wrap_ansi_to_width("\u{1b}[31mab語c\u{1b}[0m", 3);

        assert_eq!(
            rows.iter().map(|row| strip_ansi(row)).collect::<Vec<_>>(),
            vec!["ab", "語c"]
        );
    }

    #[test]
    fn wrapped_ansi_rows_replay_and_reset_active_sgr_styles() {
        let rows = wrap_ansi_to_width("\u{1b}[31mabcdef\u{1b}[0m", 3);

        assert_eq!(
            rows.iter().map(|row| strip_ansi(row)).collect::<Vec<_>>(),
            vec!["abc", "def"]
        );
        assert!(rows[0].starts_with("\u{1b}[31m"));
        assert!(rows[0].ends_with("\u{1b}[0m"));
        assert!(rows[1].starts_with("\u{1b}[31m"));
        assert!(rows[1].ends_with("\u{1b}[0m"));
    }

    #[test]
    fn counts_wrapped_rendered_lines() {
        assert_eq!(rendered_line_count("abcdef", 3), 2);
        assert_eq!(rendered_line_count("a\nbc", 10), 2);
        assert_eq!(rendered_line_count("a\n", 10), 1);
        assert_eq!(rendered_line_count("語語", 3), 2);
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
