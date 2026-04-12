use std::{fs, path::Path};

use crate::kernel::LocalKernel;

pub(super) fn startup_failure_message(
    status: std::process::ExitStatus,
    local: &LocalKernel,
    parse_error: Option<&anyhow::Error>,
) -> String {
    let mut details = vec![
        format!("Python: {}", local.launch.python),
        format!("Status: {status}"),
    ];
    append_startup_context(&mut details, local, parse_error);
    format_diagnostic("Kernel startup failed", &details)
}

pub(super) fn local_exit_message(status: std::process::ExitStatus, local: &LocalKernel) -> String {
    let mut details = vec![
        format!("Python: {}", local.launch.python),
        format!("Status: {status}"),
    ];

    if let Some(log_output) = read_stderr_log(local.stderr_log.path()) {
        details.push(format!("Kernel stderr:\n{log_output}"));
    }

    format_diagnostic("Kernel exited unexpectedly", &details)
}

pub(super) fn startup_timeout_message(
    local: &LocalKernel,
    parse_error: Option<&anyhow::Error>,
) -> String {
    let mut details = vec![
        format!("Python: {}", local.launch.python),
        "Reason: timed out waiting for the connection file".to_string(),
    ];
    append_startup_context(&mut details, local, parse_error);
    format_diagnostic("Kernel startup timed out", &details)
}

pub(super) fn append_startup_context(
    details: &mut Vec<String>,
    local: &LocalKernel,
    parse_error: Option<&anyhow::Error>,
) {
    if let Some(error) = parse_error {
        details.push(format!("Connection file: {error}"));
    }

    if let Some(log_output) = read_stderr_log(local.stderr_log.path()) {
        details.push(format!("Kernel stderr:\n{log_output}"));
    }
}

fn format_diagnostic(title: &str, details: &[String]) -> String {
    let mut lines = vec![title.to_string()];
    lines.extend(details.iter().map(|detail| indent_detail(detail)));
    lines.join("\n")
}

fn indent_detail(detail: &str) -> String {
    let mut lines = detail.lines();
    let Some(first) = lines.next() else {
        return "  ".to_string();
    };

    let mut formatted = format!("  {first}");
    for line in lines {
        formatted.push('\n');
        formatted.push_str("    ");
        formatted.push_str(line);
    }
    formatted
}

fn read_stderr_log(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
