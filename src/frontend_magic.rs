use std::{
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Result, anyhow};

use crate::kernel::StreamName;

#[derive(Debug, Default)]
pub(crate) struct OutputCaptureStore {
    active_execution_count: Option<u32>,
    cells: Vec<CellOutputCapture>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellOutputCapture {
    pub(crate) execution_count: u32,
    pub(crate) code: String,
    pub(crate) output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedOutput<'a> {
    pub(crate) execution_count: u32,
    pub(crate) output: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputTarget {
    Previous,
    ExecutionCount(u32),
    RelativePrevious(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolveOutputError {
    MissingPrevious,
    MissingExecutionCount(u32),
    MissingRelativePrevious(usize),
    EmptyOutput(u32),
}

impl OutputCaptureStore {
    pub(crate) fn begin_cell(&mut self, execution_count: Option<u32>, code: &str) {
        self.active_execution_count = execution_count;
        if let Some(execution_count) = execution_count {
            self.cells.push(CellOutputCapture {
                execution_count,
                code: code.to_string(),
                output: String::new(),
            });
        }
    }

    pub(crate) fn append_stream(&mut self, name: StreamName, text: &str) {
        if name != StreamName::Stdout {
            return;
        }

        let Some(active_execution_count) = self.active_execution_count else {
            return;
        };

        if let Some(cell) = self
            .cells
            .iter_mut()
            .rev()
            .find(|cell| cell.execution_count == active_execution_count)
        {
            cell.output.push_str(text);
        }
    }

    pub(crate) fn append_execute_result(&mut self, execution_count: Option<u32>, text: &str) {
        let execution_count = execution_count.or(self.active_execution_count);
        let Some(execution_count) = execution_count else {
            return;
        };

        if let Some(cell) = self
            .cells
            .iter_mut()
            .rev()
            .find(|cell| cell.execution_count == execution_count)
        {
            append_captured_text(&mut cell.output, text);
        }
    }

    pub(crate) fn finish_active(&mut self) {
        self.active_execution_count = None;
    }

    pub(crate) fn resolve_output(
        &self,
        target: OutputTarget,
    ) -> std::result::Result<ResolvedOutput<'_>, ResolveOutputError> {
        let cell = match target {
            OutputTarget::Previous => self
                .cells
                .last()
                .ok_or(ResolveOutputError::MissingPrevious)?,
            OutputTarget::ExecutionCount(execution_count) => self
                .cells
                .iter()
                .rev()
                .find(|cell| cell.execution_count == execution_count)
                .ok_or(ResolveOutputError::MissingExecutionCount(execution_count))?,
            OutputTarget::RelativePrevious(index_from_end) => {
                let Some(index) = self.cells.len().checked_sub(index_from_end) else {
                    return Err(ResolveOutputError::MissingRelativePrevious(index_from_end));
                };
                self.cells
                    .get(index)
                    .ok_or(ResolveOutputError::MissingRelativePrevious(index_from_end))?
            }
        };

        if cell.output.is_empty() {
            return Err(ResolveOutputError::EmptyOutput(cell.execution_count));
        }

        Ok(ResolvedOutput {
            execution_count: cell.execution_count,
            output: &cell.output,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrontendMagicParse {
    NotFrontendMagic,
    Magic(FrontendMagic),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrontendMagic {
    PrintOutput { target: OutputTarget },
    ClipOutput { target: OutputTarget },
}

pub(crate) fn parse_frontend_magic(code: &str) -> FrontendMagicParse {
    let trimmed = code.trim();
    if trimmed.contains('\n') {
        return FrontendMagicParse::NotFrontendMagic;
    }

    let mut parts = trimmed.split_ascii_whitespace();
    let Some(command) = parts.next() else {
        return FrontendMagicParse::NotFrontendMagic;
    };

    let magic_kind = match command {
        "%fpy_out" => FrontendMagicKind::PrintOutput,
        "%fpy_clipout" => FrontendMagicKind::ClipOutput,
        _ => return FrontendMagicParse::NotFrontendMagic,
    };

    let target = match (parts.next(), parts.next()) {
        (None, None) => OutputTarget::Previous,
        (Some(raw), None) => match parse_output_target(raw) {
            Ok(target) => target,
            Err(message) => return FrontendMagicParse::Error(message),
        },
        (Some(_), Some(_)) => {
            return FrontendMagicParse::Error(format!("{command} accepts at most one argument"));
        }
        (None, Some(_)) => unreachable!("split iterator cannot yield a second item first"),
    };

    FrontendMagicParse::Magic(match magic_kind {
        FrontendMagicKind::PrintOutput => FrontendMagic::PrintOutput { target },
        FrontendMagicKind::ClipOutput => FrontendMagic::ClipOutput { target },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontendMagicKind {
    PrintOutput,
    ClipOutput,
}

fn parse_output_target(raw: &str) -> std::result::Result<OutputTarget, String> {
    let value = raw
        .parse::<i64>()
        .map_err(|_| format!("invalid output target: {raw}"))?;

    if value > 0 {
        let execution_count =
            u32::try_from(value).map_err(|_| format!("output target is too large: {raw}"))?;
        return Ok(OutputTarget::ExecutionCount(execution_count));
    }

    if value < 0 {
        let relative = usize::try_from(value.unsigned_abs())
            .map_err(|_| format!("output target is too large: {raw}"))?;
        return Ok(OutputTarget::RelativePrevious(relative));
    }

    Err("output target must be a positive execution count or negative relative offset".to_string())
}

fn append_captured_text(captured: &mut String, text: &str) {
    if !captured.is_empty() && !captured.ends_with('\n') {
        captured.push('\n');
    }
    captured.push_str(text);
}

pub(crate) fn copy_to_clipboard(text: &str) -> Result<()> {
    for command in clipboard_commands() {
        if try_clipboard_command(*command, text).is_ok() {
            return Ok(());
        }
    }

    Err(anyhow!(
        "clipboard unavailable: install pbcopy, wl-copy, xclip, or xsel"
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClipboardCommand {
    program: &'static str,
    args: &'static [&'static str],
}

fn clipboard_commands() -> &'static [ClipboardCommand] {
    &[
        ClipboardCommand {
            program: "pbcopy",
            args: &[],
        },
        ClipboardCommand {
            program: "wl-copy",
            args: &[],
        },
        ClipboardCommand {
            program: "xclip",
            args: &["-selection", "clipboard"],
        },
        ClipboardCommand {
            program: "xsel",
            args: &["--clipboard", "--input"],
        },
    ]
}

fn try_clipboard_command(command: ClipboardCommand, text: &str) -> Result<()> {
    let mut child = Command::new(command.program)
        .args(command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let Some(mut stdin) = child.stdin.take() else {
        return Err(anyhow!("clipboard command stdin unavailable"));
    };
    stdin.write_all(text.as_bytes())?;
    drop(stdin);

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("clipboard command exited with {status}"))
    }
}

pub(crate) fn resolve_error_message(error: ResolveOutputError) -> String {
    match error {
        ResolveOutputError::MissingPrevious => {
            "fpy: no captured output for previous cell".to_string()
        }
        ResolveOutputError::MissingExecutionCount(execution_count) => {
            format!("fpy: no captured output for In [{execution_count}]")
        }
        ResolveOutputError::MissingRelativePrevious(index_from_end) => {
            format!("fpy: no captured output for previous cell -{index_from_end}")
        }
        ResolveOutputError::EmptyOutput(execution_count) => {
            format!("fpy: In [{execution_count}] produced no captured output")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CellOutputCapture, FrontendMagic, FrontendMagicParse, OutputCaptureStore, OutputTarget,
        ResolveOutputError, parse_frontend_magic,
    };
    use crate::kernel::StreamName;

    #[test]
    fn parser_accepts_output_magics() {
        assert_eq!(
            parse_frontend_magic("%fpy_out"),
            FrontendMagicParse::Magic(FrontendMagic::PrintOutput {
                target: OutputTarget::Previous,
            })
        );
        assert_eq!(
            parse_frontend_magic("  %fpy_out 3  "),
            FrontendMagicParse::Magic(FrontendMagic::PrintOutput {
                target: OutputTarget::ExecutionCount(3),
            })
        );
        assert_eq!(
            parse_frontend_magic("%fpy_out -1"),
            FrontendMagicParse::Magic(FrontendMagic::PrintOutput {
                target: OutputTarget::RelativePrevious(1),
            })
        );
        assert_eq!(
            parse_frontend_magic("%fpy_clipout"),
            FrontendMagicParse::Magic(FrontendMagic::ClipOutput {
                target: OutputTarget::Previous,
            })
        );
    }

    #[test]
    fn parser_ignores_multiline_and_unknown_magics() {
        assert_eq!(
            parse_frontend_magic("%fpy_out\nprint('x')"),
            FrontendMagicParse::NotFrontendMagic
        );
        assert_eq!(
            parse_frontend_magic("%time print('x')"),
            FrontendMagicParse::NotFrontendMagic
        );
        assert_eq!(
            parse_frontend_magic("x = 1"),
            FrontendMagicParse::NotFrontendMagic
        );
    }

    #[test]
    fn parser_reports_invalid_frontend_magic_arguments() {
        assert!(matches!(
            parse_frontend_magic("%fpy_out abc"),
            FrontendMagicParse::Error(_)
        ));
        assert!(matches!(
            parse_frontend_magic("%fpy_clipout 1 2"),
            FrontendMagicParse::Error(_)
        ));
        assert!(matches!(
            parse_frontend_magic("%fpy_out 0"),
            FrontendMagicParse::Error(_)
        ));
    }

    #[test]
    fn capture_store_records_stdout_stream_for_execution_count() {
        let mut store = OutputCaptureStore::default();
        store.begin_cell(Some(1), "print('hello')");
        store.append_stream(StreamName::Stdout, "hello\n");

        assert_eq!(
            store.cells,
            vec![CellOutputCapture {
                execution_count: 1,
                code: "print('hello')".to_string(),
                output: "hello\n".to_string(),
            }]
        );
    }

    #[test]
    fn capture_store_appends_multiple_stdout_stream_chunks() {
        let mut store = OutputCaptureStore::default();
        store.begin_cell(Some(1), "print('hello')");
        store.append_stream(StreamName::Stdout, "hel");
        store.append_stream(StreamName::Stdout, "lo\n");

        assert_eq!(
            store
                .resolve_output(OutputTarget::Previous)
                .expect("output")
                .output,
            "hello\n"
        );
    }

    #[test]
    fn capture_store_records_execute_result_text() {
        let mut store = OutputCaptureStore::default();
        store.begin_cell(Some(1), "1");
        store.append_execute_result(Some(1), "1");

        assert_eq!(
            store
                .resolve_output(OutputTarget::Previous)
                .expect("result")
                .output,
            "1"
        );
    }

    #[test]
    fn capture_store_combines_stdout_stream_and_execute_result_text() {
        let mut store = OutputCaptureStore::default();
        store.begin_cell(Some(1), "print('hello')\n2");
        store.append_stream(StreamName::Stdout, "hello\n");
        store.append_execute_result(Some(1), "2");

        assert_eq!(
            store
                .resolve_output(OutputTarget::Previous)
                .expect("combined output")
                .output,
            "hello\n2"
        );
    }

    #[test]
    fn capture_store_ignores_stderr() {
        let mut store = OutputCaptureStore::default();
        store.begin_cell(Some(1), "import sys");
        store.append_stream(StreamName::Stderr, "error\n");

        assert_eq!(
            store.resolve_output(OutputTarget::Previous),
            Err(ResolveOutputError::EmptyOutput(1))
        );
    }

    #[test]
    fn capture_store_resolves_positive_and_negative_targets() {
        let mut store = OutputCaptureStore::default();
        store.begin_cell(Some(1), "print('one')");
        store.append_stream(StreamName::Stdout, "one\n");
        store.finish_active();
        store.begin_cell(Some(2), "print('two')");
        store.append_stream(StreamName::Stdout, "two\n");

        assert_eq!(
            store
                .resolve_output(OutputTarget::ExecutionCount(1))
                .expect("count")
                .output,
            "one\n"
        );
        assert_eq!(
            store
                .resolve_output(OutputTarget::RelativePrevious(2))
                .expect("relative")
                .output,
            "one\n"
        );
    }

    #[test]
    fn duplicate_execution_count_resolves_to_most_recent_capture() {
        let mut store = OutputCaptureStore::default();
        store.begin_cell(Some(1), "print('old')");
        store.append_stream(StreamName::Stdout, "old\n");
        store.finish_active();
        store.begin_cell(Some(1), "print('new')");
        store.append_stream(StreamName::Stdout, "new\n");

        assert_eq!(
            store
                .resolve_output(OutputTarget::ExecutionCount(1))
                .expect("count")
                .output,
            "new\n"
        );
    }

    #[test]
    fn empty_output_is_distinguishable_from_missing_cell() {
        let mut store = OutputCaptureStore::default();
        assert_eq!(
            store.resolve_output(OutputTarget::Previous),
            Err(ResolveOutputError::MissingPrevious)
        );

        store.begin_cell(Some(12), "1 + 1");
        assert_eq!(
            store.resolve_output(OutputTarget::ExecutionCount(12)),
            Err(ResolveOutputError::EmptyOutput(12))
        );
        assert_eq!(
            store.resolve_output(OutputTarget::ExecutionCount(13)),
            Err(ResolveOutputError::MissingExecutionCount(13))
        );
    }
}
