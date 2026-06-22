use edtui::{
    SYNTAX_SET, SyntaxHighlighter, THEME_SET,
    syntect::{
        easy::HighlightLines,
        util::{LinesWithEndings, as_24_bit_terminal_escaped},
    },
};

pub(super) const PYTHON_SYNTAX_EXTENSION: &str = "py";
pub(super) const PYTHON_THEME_NAME: &str = "base16-ocean-dark";

pub(super) fn editor_python_syntax_highlighter() -> Option<SyntaxHighlighter> {
    SyntaxHighlighter::new(PYTHON_THEME_NAME, PYTHON_SYNTAX_EXTENSION).ok()
}

pub(super) fn highlighted_python_lines(code: &str, trim_line_endings: bool) -> Option<Vec<String>> {
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(PYTHON_SYNTAX_EXTENSION)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let theme = THEME_SET.themes.get(PYTHON_THEME_NAME)?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut highlighted = Vec::new();

    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, &SYNTAX_SET).ok()?;
        let mut rendered = as_24_bit_terminal_escaped(&ranges, false);
        if trim_line_endings {
            rendered = rendered.trim_end_matches(['\r', '\n']).to_string();
        }
        highlighted.push(rendered);
    }

    if highlighted.is_empty() {
        highlighted.push(String::new());
    }

    Some(highlighted)
}

#[cfg(test)]
mod tests {
    use super::{
        PYTHON_SYNTAX_EXTENSION, PYTHON_THEME_NAME, editor_python_syntax_highlighter,
        highlighted_python_lines,
    };

    #[test]
    fn shared_python_highlighting_config_is_available() {
        assert_eq!(PYTHON_THEME_NAME, "base16-ocean-dark");
        assert_eq!(PYTHON_SYNTAX_EXTENSION, "py");
        assert!(editor_python_syntax_highlighter().is_some());
        assert!(highlighted_python_lines("time.sleep(1)", true).is_some());
    }
}
