pub(crate) fn transcript_lines(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized
        .split('\n')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if normalized.ends_with('\n') && lines.len() > 1 {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(crate) fn rendered_line_count(lines: &[String], width: usize) -> u16 {
    lines
        .iter()
        .map(|line| visible_width(line).max(1).div_ceil(width))
        .sum::<usize>()
        .clamp(1, u16::MAX as usize) as u16
}

pub(crate) fn visible_width(text: &str) -> usize {
    strip_ansi(text).chars().count()
}

pub(crate) fn strip_ansi(text: &str) -> String {
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
mod tests {
    use super::transcript_lines;

    #[test]
    fn preserves_real_blank_lines_but_not_trailing_line_terminator() {
        assert_eq!(transcript_lines("a\n\nb\n"), vec!["a", "", "b"]);
    }
}
