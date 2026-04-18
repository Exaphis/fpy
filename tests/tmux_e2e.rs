use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn ctrl_d_preserves_transcript() {
    let Some(output) = run_repro("ctrl-d-preserves", "ctrl-d", &[]) else {
        return;
    };
    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
}

#[test]
fn kernel_exit_returns_shell() {
    let Some(output) = run_repro("kernel-exit", "exitpy", &[]) else {
        return;
    };
    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
}

#[test]
fn multiline_growth_bottom_pinned() {
    let Some(output) = run_repro(
        "multiline-growth-bottom",
        "paste",
        &[
            ("TMUX_SIZE", "120x40"),
            ("PRE_INPUT", "1+1\n!ls -lah\n!ls -lah"),
            ("INPUTS", "1+1\n!ls -lah\n!ls -lah"),
            ("PASTE_TEXT", "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [2]: !ls -lah");
    assert_contains(&output.after, "In [3]: !ls -lah");
    assert_contains(&output.after, " 1 a");
    assert_contains(&output.after, "12 l");
    assert_line_contains_all(&output.after, &["INS", "In [4]", "Ctrl-P palette"]);
}

#[test]
fn bottom_of_screen_result_still_visible() {
    let Some(output) = run_repro(
        "bottom-result-visible",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            ("PRE_INPUT", "!ls -lah\n!ls -lah\n!ls -lah\n!ls -lah\n1+1"),
            ("INPUTS", "!ls -lah\n!ls -lah\n!ls -lah\n!ls -lah\n1+1"),
            ("EXIT_WAIT", "1"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [5]: 1+1");
    assert_contains(&output.after, "Out[5]: 2");
    assert_line_contains_all(&output.after, &["INS", "In [6]", "Ctrl-P palette"]);
}

#[test]
fn bottom_pinned_transcript_repaint_clears_stale_busy_status() {
    let Some(output) = run_repro(
        "bottom-pinned-stale-busy",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            ("PRE_INPUT", "!ls -lah\n!ls -lah\n!ls -lah\n!ls -lah"),
            ("INPUTS", "!ls -lah\n!ls -lah\n!ls -lah\n!ls -lah"),
            ("EXIT_WAIT", "1"),
            ("CAPTURE_LINES", "80"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [4]: !ls -lah");
    assert_contains(&output.after, "[");
    assert_line_contains_all(&output.after, &["INS", "In [5]", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
}

#[test]
fn runtime_line_appears_after_output() {
    let Some(output) = run_repro(
        "runtime-line",
        "none",
        &[("PRE_INPUT", "1+1"), ("INPUTS", "1+1"), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "Out[1]: 2");
    assert_bracketed_runtime_after(&output.after, "Out[1]: 2");
}

#[test]
fn stdin_reply_is_sent_on_enter() {
    let Some(output) = run_repro(
        "stdin-reply",
        "stdin-reply",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "bob");
    assert_not_contains(&output.after, "stdin>");
}

#[test]
fn stdin_empty_reply_is_sent_on_enter() {
    let Some(output) = run_repro(
        "stdin-empty-reply",
        "stdin-empty-reply",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: repr(input())");
    assert_contains(&output.after, "Out[1]: \"''\"");
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
}

#[test]
fn stdin_prompt_is_flush_left() {
    let Some(output) = run_repro(
        "stdin-prompt",
        "stdin-prompt",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_line_contains_all(&output.after, &["INS", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "stdin");
    assert_no_line_starts_with(&output.after, "1 ");
}

#[test]
fn stdin_shift_enter_keeps_prompt_clean() {
    let Some(output) = run_repro(
        "stdin-shift-enter",
        "stdin-shift-enter",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: input()");
    assert_line_contains_all(&output.after, &["INS", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "stdin");
    assert_not_contains(&output.after, "■");
    assert_not_contains(&output.after, "█");
}

#[test]
fn ctrl_d_does_not_exit_during_stdin_prompt() {
    let Some(output) = run_repro(
        "stdin-ctrl-d",
        "stdin-ctrl-d",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: input()");
    assert_contains(&output.after, "Out[1]: 'x'");
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "command not found");
}

#[test]
fn ctrl_c_interrupts_during_stdin_prompt() {
    let Some(output) = run_repro(
        "stdin-ctrl-c",
        "stdin-ctrl-c",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: input()");
    assert_contains(&output.after, "KeyboardInterrupt");
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
}

#[test]
fn pdb_prompt_and_commands_are_visible() {
    let Some(output) = run_repro(
        "pdb-basic",
        "pdb-basic",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: import pdb; pdb.set_trace(); print(\"after\")");
    assert!(
        output.after.contains("(Pdb) where") || output.after.contains("ipdb> where"),
        "expected a visible debugger prompt for `where` in output:\n{}",
        output.after
    );
    assert!(
        output.after.contains("(Pdb) p 1+1") || output.after.contains("ipdb> p 1+1"),
        "expected a visible debugger prompt for `p 1+1` in output:\n{}",
        output.after
    );
    assert_contains(&output.after, "2");
    assert!(
        output.after.contains("(Pdb) c") || output.after.contains("ipdb> c"),
        "expected a visible debugger prompt for `c` in output:\n{}",
        output.after
    );
    assert_contains(&output.after, "after");
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
}

#[test]
fn multiline_paste_preserves_all_lines() {
    let Some(output) = run_repro(
        "multiline-paste",
        "paste",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            (
                "PASTE_TEXT",
                "use edtui::{EditorState, EditorTheme, EditorView};\nuse ratatui::widgets::Widget;\n\nlet mut state = EditorState::default();\nEditorView::new(&mut state)\n        .theme(EditorTheme::default())\n        .wrap(true)\n        .syntax_highlighter(None)\n        .tab_width(2)\n        .render(area, buf);",
            ),
        ],
    ) else {
        return;
    };

    assert_contains(
        &output.after,
        "1 use edtui::{EditorState, EditorTheme, EditorView};",
    );
    assert_contains(&output.after, "2 use ratatui::widgets::Widget;");
    assert_contains(&output.after, "4 let mut state = EditorState::default();");
    assert_contains(&output.after, "9         .tab_width(2)");
    assert_contains(&output.after, "10         .render(area, buf)");
}

#[test]
fn can_compose_while_kernel_is_busy() {
    let Some(output) = run_repro(
        "compose-while-busy",
        "compose-while-busy",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: import time; time.sleep(3); 42");
    assert_contains(&output.after, "1 1+1");
    assert_line_contains_all(
        &output.after,
        &["INS", "In [2]", "Kernel busy. Ctrl-C to interrupt", "Ctrl-P palette"],
    );
}

#[test]
fn shift_enter_creates_multiline_editor() {
    let Some(output) = run_repro(
        "shift-enter",
        "shift-enter",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "1 abc");
    assert_contains(&output.after, "2");
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
}

#[test]
fn ctrl_c_after_multiline_resets_prompt_spacing() {
    let Some(output) = run_repro(
        "ctrl-c-multiline",
        "ctrl-c-multiline",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_no_line_starts_with(&output.after, "2 ");
    assert_line_contains_all(&output.after, &["1"]);
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
}

#[test]
fn ctrl_c_after_multiline_leaves_gap_below_prompt() {
    let Some(output) = run_repro(
        "ctrl-c-multiline-bottom",
        "ctrl-c-multiline-bottom",
        &[("TMUX_SIZE", "120x20"), ("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_line_contains_all(&output.after, &["1"]);
    assert_line_contains_all(&output.after, &["INS", "In [5]", "Ctrl-P palette"]);
    assert_blank_line_after_contains(&output.after, "Ctrl-P palette");
}

#[test]
fn vim_open_below_grows_on_first_try() {
    let Some(output) = run_repro(
        "vim-open-below",
        "vim-open-below",
        &[("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "1");
    assert_contains(&output.after, "2");
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
}

#[test]
fn history_up_reruns_previous_cell() {
    let Some(output) = run_repro(
        "history-up",
        "history-up",
        &[("PRE_INPUT", "1+1\n2+2"), ("INPUTS", "1+1\n2+2"), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [2]: 2+2");
    assert_contains(&output.after, "Out[2]: 4");
    assert_contains(&output.after, "In [3]: 2+2");
    assert_contains(&output.after, "Out[3]: 4");
}

#[test]
fn palette_clears_underlying_empty_prompt() {
    let Some(output) = run_repro(
        "palette-empty",
        "palette",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "Command Palette");
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_no_line_contains_all(&output.after, &["Ctrl-P palette", "│"]);
}

#[test]
fn palette_close_reopen_does_not_leave_stale_status_cells() {
    let Some(output) = run_repro(
        "palette-cycle",
        "palette-cycle",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "Command Palette");
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_no_line_contains_all(&output.after, &["Ctrl-P palette", "│"]);
    assert_no_line_contains_all(&output.after, &["Quit", "In [1]"]);
}

#[test]
fn palette_move_close_reopen_does_not_mix_with_status_row() {
    let Some(output) = run_repro(
        "palette-move-cycle",
        "palette-move-cycle",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    ) else {
        return;
    };

    assert_contains(&output.after, "Command Palette");
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_no_line_contains_all(&output.after, &["Ctrl-P palette", "│"]);
    assert_no_line_contains_all(&output.after, &["Quit", "In [1]"]);
    assert_no_line_contains_all(&output.after, &["Interrupt Kernel", "In [1]"]);
}

struct ReproOutput {
    #[allow(dead_code)]
    before: String,
    after: String,
}

fn run_repro(name: &str, action: &str, extra_env: &[(&str, &str)]) -> Option<ReproOutput> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(reason) = missing_prerequisites(&repo_root) {
        eprintln!("skipping tmux e2e test: {reason}");
        return None;
    }

    let unique = unique_id();
    let target_dir = repo_root.join("target");
    let before_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.before.log"));
    let after_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.after.log"));
    let session = format!("fpy-e2e-{name}-{unique}");
    let fpy_bin = option_env!("CARGO_BIN_EXE_fpy")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .unwrap_or_else(|| repo_root.join("target/debug/fpy"));

    let mut command = Command::new(repo_root.join("scripts/fpy-tmux-repro.sh"));
    command.current_dir(&repo_root).arg(action);
    command.env("SESSION", &session);
    command.env("BEFORE_LOG", &before_log);
    command.env("AFTER_LOG", &after_log);
    if fpy_bin.exists() {
        command.env("FPY_BIN", &fpy_bin);
    }
    for (key, value) in extra_env {
        command.env(key, value);
    }

    let output = command.output().unwrap_or_else(|error| {
        panic!("failed to start tmux repro script: {error}");
    });
    if !output.status.success() {
        panic!(
            "tmux repro failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Some(ReproOutput {
        before: read_log(&before_log),
        after: read_log(&after_log),
    })
}

fn read_log(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    })
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected to find {needle:?} in output:\n{haystack}"
    );
}

fn assert_not_contains(haystack: &str, needle: &str) {
    assert!(
        !haystack.contains(needle),
        "did not expect to find {needle:?} in output:\n{haystack}"
    );
}

fn assert_line_count(haystack: &str, needle: &str, expected: usize) {
    let count = haystack.lines().filter(|line| line.contains(needle)).count();
    assert_eq!(
        count, expected,
        "expected {needle:?} to appear {expected} time(s) in output:\n{haystack}"
    );
}

fn assert_line_contains_all(haystack: &str, needles: &[&str]) {
    assert!(
        haystack
            .lines()
            .any(|line| needles.iter().all(|needle| line.contains(needle))),
        "expected some output line to contain all of {:?}:\n{}",
        needles,
        haystack
    );
}

fn assert_no_line_starts_with(haystack: &str, prefix: &str) {
    assert!(
        haystack.lines().all(|line| !line.starts_with(prefix)),
        "expected no output line to start with {:?}:\n{}",
        prefix,
        haystack
    );
}

fn assert_blank_line_after_contains(haystack: &str, needle: &str) {
    let lines = haystack.lines().collect::<Vec<_>>();
    let index = lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("expected to find line containing {:?} in output:\n{}", needle, haystack));
    assert!(
        index + 1 < lines.len() && lines[index + 1].trim().is_empty(),
        "expected a blank line immediately after a line containing {:?}:\n{}",
        needle,
        haystack
    );
}

fn assert_bracketed_runtime_after(haystack: &str, needle: &str) {
    let lines = haystack.lines().collect::<Vec<_>>();
    let index = lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("expected to find line containing {:?} in output:\n{}", needle, haystack));
    let runtime_line = lines
        .iter()
        .skip(index + 1)
        .find(|line| !line.trim().is_empty())
        .unwrap_or_else(|| panic!("expected a runtime line after {:?}:\n{}", needle, haystack));
    assert!(
        runtime_line.starts_with('[')
            && runtime_line.ends_with(']')
            && ["µs", "ms", "s", "m", "h", "d"]
                .iter()
                .any(|unit| runtime_line.contains(unit)),
        "expected a bracketed runtime line after {:?}, got {:?}\n{}",
        needle,
        runtime_line,
        haystack
    );
}

fn assert_no_line_contains_all(haystack: &str, needles: &[&str]) {
    assert!(
        haystack
            .lines()
            .all(|line| !needles.iter().all(|needle| line.contains(needle))),
        "expected no output line to contain all of {:?}:\n{}",
        needles,
        haystack
    );
}

fn missing_prerequisites(repo_root: &Path) -> Option<String> {
    let repro = repo_root.join("scripts/fpy-tmux-repro.sh");
    if !repro.exists() {
        return Some(format!("missing {}", repro.display()));
    }

    let python = repo_root.join(".venv/bin/python");
    let python_ok = python.exists()
        || matches!(
            Command::new("python3").arg("--version").status(),
            Ok(status) if status.success()
        );
    if !python_ok {
        return Some("python3 is not available".to_string());
    }

    let tmux = Command::new("tmux").arg("-V").status();
    if !matches!(tmux, Ok(status) if status.success()) {
        return Some("tmux is not available".to_string());
    }

    None
}

fn unique_id() -> u64 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos() as u64;
    time ^ u64::from(std::process::id()) ^ NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
