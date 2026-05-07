use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellEdit {
    pub(crate) text: String,
    pub(crate) cursor_byte: usize,
}

impl CellEdit {
    pub(crate) fn new(text: String, cursor_byte: usize) -> Self {
        Self { text, cursor_byte }
    }
}

pub(crate) struct ExtensionContext<'a> {
    pub(crate) cell: &'a str,
    pub(crate) cursor_byte: usize,
}

pub(crate) enum ExtensionOutcome {
    Ignored,
    ReplaceCell(CellEdit),
}

pub(crate) trait FpyExtension {
    fn on_key(&mut self, _key: KeyEvent, _ctx: ExtensionContext<'_>) -> ExtensionOutcome {
        ExtensionOutcome::Ignored
    }
}

pub(crate) struct ExtensionManager {
    extensions: Vec<Box<dyn FpyExtension>>,
}

impl ExtensionManager {
    pub(crate) fn with_defaults() -> Self {
        let mut manager = Self {
            extensions: Vec::new(),
        };
        manager.register(Box::new(FormatCellExtension));
        manager
    }

    pub(crate) fn register(&mut self, extension: Box<dyn FpyExtension>) {
        self.extensions.push(extension);
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent, ctx: ExtensionContext<'_>) -> ExtensionOutcome {
        for extension in &mut self.extensions {
            match extension.on_key(
                key,
                ExtensionContext {
                    cell: ctx.cell,
                    cursor_byte: ctx.cursor_byte,
                },
            ) {
                ExtensionOutcome::Ignored => {}
                outcome => return outcome,
            }
        }
        ExtensionOutcome::Ignored
    }
}

struct FormatCellExtension;

impl FpyExtension for FormatCellExtension {
    fn on_key(&mut self, key: KeyEvent, ctx: ExtensionContext<'_>) -> ExtensionOutcome {
        if key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let formatted = normalize_cell_indentation(ctx.cell);
            let cursor_byte = ctx.cursor_byte.min(formatted.len());
            ExtensionOutcome::ReplaceCell(CellEdit::new(formatted, cursor_byte))
        } else {
            ExtensionOutcome::Ignored
        }
    }
}

fn normalize_cell_indentation(cell: &str) -> String {
    let mut lines: Vec<&str> = cell.lines().map(strip_prompt_prefix).collect();

    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let common_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count()
        })
        .min()
        .unwrap_or(0);

    lines
        .into_iter()
        .map(|line| strip_indent(line, common_indent).trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_prompt_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix(">>> ") {
        return rest;
    }
    if trimmed == ">>>" {
        return "";
    }
    if let Some(rest) = trimmed.strip_prefix("... ") {
        return rest;
    }
    if let Some(rest) = trimmed.strip_prefix("...: ") {
        return rest;
    }
    if trimmed == "..." || trimmed == "...:" {
        return "";
    }

    strip_ipython_prompt(trimmed).unwrap_or(line)
}

fn strip_ipython_prompt(line: &str) -> Option<&str> {
    let mut rest = line;
    if let Some(after_mode) = strip_mode_prefix(rest) {
        rest = after_mode;
    }
    let after_in = rest.strip_prefix("In [")?;
    let close = after_in.find("]:")?;
    let number = &after_in[..close];
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(
        after_in[close + 2..]
            .strip_prefix(' ')
            .unwrap_or(&after_in[close + 2..]),
    )
}

fn strip_mode_prefix(line: &str) -> Option<&str> {
    let after_open = line.strip_prefix('[')?;
    let close = after_open.find("] ")?;
    let mode = &after_open[..close];
    (!mode.is_empty() && mode.chars().all(|ch| ch.is_ascii_alphabetic()))
        .then_some(&after_open[close + 2..])
}

fn strip_indent(line: &str, width: usize) -> &str {
    if width == 0 {
        return line;
    }

    for (stripped, (byte_index, ch)) in line.char_indices().enumerate() {
        if stripped == width || (ch != ' ' && ch != '\t') {
            return &line[byte_index..];
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::normalize_cell_indentation;

    #[test]
    fn format_cell_trims_blank_edges_and_dedents() {
        assert_eq!(
            normalize_cell_indentation("\n    x = 1  \n    if x:\n        print(x)\n\n"),
            "x = 1\nif x:\n    print(x)"
        );
    }

    #[test]
    fn format_cell_preserves_relative_indent() {
        assert_eq!(
            normalize_cell_indentation("  def f():\n      return 1"),
            "def f():\n    return 1"
        );
    }

    #[test]
    fn format_cell_strips_repl_prompts() {
        assert_eq!(
            normalize_cell_indentation(">>> def f():\n...     return 1\n>>> f()"),
            "def f():\n    return 1\nf()"
        );
    }

    #[test]
    fn format_cell_strips_ipython_prompts() {
        assert_eq!(
            normalize_cell_indentation(
                "In [12]: x = 1\n[ins] In [13]: y = 2\n[nav] In [14]: z = 3"
            ),
            "x = 1\ny = 2\nz = 3"
        );
    }

    #[test]
    fn format_cell_strips_multiline_ipython_prompts_before_dedent() {
        assert_eq!(
            normalize_cell_indentation(
                "In [12]:     def f():\n   ...:         return 1\n[nav] In [13]:     f()"
            ),
            "def f():\n    return 1\nf()"
        );
    }
}
