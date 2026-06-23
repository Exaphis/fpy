use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tempfile::TempDir;
use uuid::Uuid;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn idle_prompt_emits_no_redundant_terminal_output() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(reason) = missing_prerequisites(&repo_root) {
        panic!("tmux e2e prerequisites missing: {reason}");
    }

    let unique = unique_id();
    let session = format!("fpy-e2e-idle-{unique}");
    let history_dir = TempDir::new().expect("test history dir");
    let ansi_log = repo_root
        .join("target")
        .join(format!("tmux-e2e-idle-{unique}.ansi.log"));
    let fpy_bin = option_env!("CARGO_BIN_EXE_fpy")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .unwrap_or_else(|| repo_root.join("target/debug/fpy"));
    let python = repo_root.join(".venv/bin/python");
    let python_bin = if python.exists() {
        python
    } else {
        PathBuf::from("python3")
    };

    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let status = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-x",
            "120",
            "-y",
            "40",
            &format!("env FPY_HISTORY_DIR='{}' zsh", history_dir.path().display()),
        ])
        .status()
        .expect("start tmux session");
    assert!(status.success(), "failed to start tmux session");

    let pipe_command = format!("cat > '{}'", ansi_log.display());
    let status = Command::new("tmux")
        .args(["pipe-pane", "-t", &session, "-o", &pipe_command])
        .status()
        .expect("pipe tmux pane");
    assert!(status.success(), "failed to pipe tmux pane");

    let launch = format!(
        "'{}' run --python '{}'",
        fpy_bin.display(),
        python_bin.display()
    );
    let status = Command::new("tmux")
        .args([
            "send-keys",
            "-t",
            &session,
            &format!("cd '{}'", repo_root.display()),
            "Enter",
        ])
        .status()
        .expect("send repo root");
    assert!(status.success(), "failed to send repo root");
    let status = Command::new("tmux")
        .args(["send-keys", "-t", &session, &launch, "Enter"])
        .status()
        .expect("launch fpy");
    assert!(status.success(), "failed to launch fpy");

    wait_for_submit_ready(&session, Duration::from_secs(20));
    wait_for_stable_file_size(
        &ansi_log,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let baseline = file_len(&ansi_log);
    thread::sleep(Duration::from_millis(350));
    let after_idle = file_len(&ansi_log);

    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    assert_eq!(
        after_idle,
        baseline,
        "expected no additional terminal output while idle; baseline={baseline}, after_idle={after_idle}, ansi log={} ",
        ansi_log.display()
    );
}

#[test]
fn ctrl_d_preserves_transcript() {
    let Some(output) = run_repro("ctrl-d-preserves", "ctrl-d", &[]) else {
        return;
    };
    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
}

#[test]
fn frame_backend_basic_transcript_smoke() {
    let Some(output) = run_repro("frame-backend-basic", "none", &[("EXIT_WAIT", "0.05")]) else {
        return;
    };
    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
    assert_last_prompt_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
    assert_contains(&output.after_ansi, "\u{1b}[90m      1 ");
}

#[test]
fn frame_backend_exit_returns_shell_prompt_without_blank_gap() {
    let Some(output) = run_repro(
        "frame-backend-exit-shell-adjacent",
        "ctrl-d",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("CAPTURE_VISIBLE_ONLY", "1"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_shell_prompt_immediately_after_last_prompt(&output.after);
}

#[test]
fn frame_backend_second_invocation_after_exit_echoes_typed_input() {
    let Some(output) = run_repro(
        "frame-backend-relaunch-after-exit",
        "relaunch-after-ctrl-d",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("CAPTURE_VISIBLE_ONLY", "1"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "fpy 0.1.0");
    assert_line_contains_all(&output.after, &["1", "abc"]);
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
}

#[test]
fn frame_backend_exit_restores_shell_tty_modes() {
    let Some(output) = run_repro(
        "frame-backend-exit-tty-cleanup",
        "ctrl-d-terminal-cleanup",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("CAPTURE_LINES", "80"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "__FPY_POST_EXIT_STTY__");
    assert_contains(&output.after, "__FPY_POST_EXIT_DONE__");
    assert_line_contains_all(
        &output.after,
        &["__FPY_POST_EXIT_STTY__", "icanon", " echo "],
    );
    assert_no_line_contains_all(&output.after, &["__FPY_POST_EXIT_STTY__", "-icanon"]);
}

#[test]
fn frame_backend_kernel_exit_returns_shell_prompt_without_blank_gap() {
    let Some(output) = run_repro(
        "frame-backend-kernel-exit-shell-adjacent",
        "exitpy",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("CAPTURE_VISIBLE_ONLY", "1"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "Kernel exited unexpectedly");
    assert_shell_prompt_immediately_after_last_prompt(&output.after);
}

#[test]
fn frame_backend_starts_below_shell_prompt_instead_of_top_of_screen() {
    let sentinel = "__FPY_VISUAL_SENTINEL__";
    let Some(output) = run_repro(
        "frame-backend-startup-anchor",
        "none",
        &[
            ("VISUAL_SENTINEL", sentinel),
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("CAPTURE_VISIBLE_ONLY", "1"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, sentinel);
    let sentinel_row = output.meta_value("sentinel_row");
    let fpy_row = output.meta_value("fpy_row");
    assert!(
        fpy_row > sentinel_row,
        "expected fpy to render below shell sentinel row; sentinel_row={sentinel_row}, fpy_row={fpy_row}\n{}",
        output.after
    );
}

#[test]
fn simple_expression_preserves_existing_scrollback() {
    let sentinel = "simple-one-sentinel";
    let Some(output) = run_repro(
        "simple-expression-preserves-scrollback",
        "none",
        &[
            ("VISUAL_SENTINEL", sentinel),
            ("INPUTS", "1"),
            ("CAPTURE_LINES", "120"),
            ("EXIT_WAIT", "0.2"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, sentinel);
    let sentinel_row = output.meta_value("sentinel_row");
    let fpy_row = output.meta_value("fpy_row");
    assert!(
        fpy_row > sentinel_row,
        "expected simple expression render to stay below existing shell output; sentinel_row={sentinel_row}, fpy_row={fpy_row}\n{}",
        output.after
    );
}

#[test]
fn frame_backend_footer_keeps_ansi_status_highlighting() {
    let Some(output) = run_repro(
        "frame-backend-footer-ansi",
        "none",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
    ) else {
        return;
    };

    assert_contains(&output.after, "INS");
    assert_contains(&output.after_ansi, "\u{1b}[30m\u{1b}[46m INS ");
    assert_contains(&output.after_ansi, "\u{1b}[90mCtrl-P palette");
}

#[test]
fn frame_backend_palette_smoke() {
    let Some(output) = run_repro(
        "frame-backend-palette",
        "palette",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
    ) else {
        return;
    };
    assert_contains(&output.after, "Command Palette");
    assert_contains(&output.after, "Interrupt Kernel");
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_no_line_contains_all(&output.after, &["Ctrl-P palette", "│"]);
}

#[test]
fn ctrl_d_requires_confirmation() {
    let Some(output) = run_repro("ctrl-d-confirmation", "ctrl-d-confirmation", &[]) else {
        return;
    };
    assert_contains(&output.after, "press Ctrl-D again to exit");
    assert_contains(&output.after, "Ctrl-P palette");
}

#[test]
fn question_mark_introspection_displays_help() {
    let Some(output) = run_repro(
        "question-mark-introspection",
        "none",
        &[("INPUTS", "len?\nlen??"), ("CAPTURE_LINES", "160")],
    ) else {
        return;
    };
    assert_contains(&output.after, "In [1]: len?");
    assert_contains(&output.after, "In [2]: len??");
    assert_contains(&output.after, "Signature: len(obj, /)");
    assert_contains(
        &output.after,
        "Docstring: Return the number of items in a container.",
    );
    assert_not_contains(&output.after, "Out[?]:");
}

#[test]
fn kernel_exit_returns_shell() {
    let Some(output) = run_repro("kernel-exit", "exitpy", &[]) else {
        return;
    };
    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
    assert_contains(&output.after, "Kernel exited unexpectedly");
}

#[test]
fn multiline_growth_bottom_pinned() {
    let Some(output) = run_repro(
        "multiline-growth-bottom",
        "paste",
        &[
            ("TMUX_SIZE", "120x40"),
            (
                "PRE_INPUT",
                "1+1\nprint(\"\\n\".join(f\"fill {i}\" for i in range(30)))",
            ),
            (
                "INPUTS",
                "1+1\nprint(\"\\n\".join(f\"fill {i}\" for i in range(30)))",
            ),
            ("PASTE_TEXT", "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl"),
            ("CAPTURE_LINES", "120"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "fill 29");
    assert!(
        output.after.contains(" 1 a") || output.after.contains("In [3]: a"),
        "expected first pasted line in output:\n{}",
        output.after
    );
    assert!(
        output.after.contains("12 l") || output.after.contains("        l"),
        "expected final pasted line in output:\n{}",
        output.after
    );
    assert_line_contains_all(&output.after, &["INS", "In [3]", "Ctrl-P palette"]);
}

#[test]
fn bottom_of_screen_result_still_visible() {
    let Some(output) = run_repro(
        "bottom-result-visible",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            (
                "PRE_INPUT",
                "print(\"\\n\".join(f\"fill {i}\" for i in range(30)))\n1+1",
            ),
            (
                "INPUTS",
                "print(\"\\n\".join(f\"fill {i}\" for i in range(30)))\n1+1",
            ),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [2]: 1+1");
    assert_contains(&output.after, "Out[2]: 2");
    assert_line_contains_all(&output.after, &["INS", "In [3]", "Ctrl-P palette"]);
}

#[test]
fn long_output_transition_to_bottom_pinned_preserves_tail() {
    let Some(output) = run_repro(
        "long-output-transition-bottom",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            (
                "PRE_INPUT",
                "print(\"\\n\".join(f\"line {i}\" for i in range(1,41)))",
            ),
            (
                "INPUTS",
                "print(\"\\n\".join(f\"line {i}\" for i in range(1,41)))",
            ),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_LINES", "160"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "line 39");
    assert_contains(&output.after, "line 40");
    assert_bracketed_runtime_after(&output.after, "line 40");
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
    assert_last_prompt_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
    assert_last_prompt_line_contains_none(
        &output.after,
        &[
            "Connecting to kernel...",
            "Kernel busy. Ctrl-C to interrupt",
        ],
    );
}

#[test]
fn long_output_without_trailing_newline_then_execute_result_stays_clean() {
    let Some(output) = run_repro(
        "long-output-no-newline-bottom",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            (
                "PRE_INPUT",
                "import sys; sys.stdout.write(\"\\n\".join(f\"line {i}\" for i in range(1,41))); sys.stdout.flush(); 0",
            ),
            (
                "INPUTS",
                "import sys; sys.stdout.write(\"\\n\".join(f\"line {i}\" for i in range(1,41))); sys.stdout.flush(); 0",
            ),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_LINES", "180"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "line 39");
    assert_contains(&output.after, "line 40");
    assert_contains(&output.after, "Out[1]: 0");
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
    assert_last_prompt_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
}

#[test]
fn multiline_output_then_short_output_stays_clean() {
    let Some(output) = run_repro(
        "multiline-then-short-output",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            (
                "PRE_INPUT",
                "print(\"\\n\".join(f\"line {i}\" for i in range(1,16)))\nprint(\"short\")",
            ),
            (
                "INPUTS",
                "print(\"\\n\".join(f\"line {i}\" for i in range(1,16)))\nprint(\"short\")",
            ),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_LINES", "120"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "line 15");
    assert_contains(&output.after, "In [2]: print(\"short\")");
    assert_contains(&output.after, "short\n[");
    assert_last_prompt_line_contains_all(&output.after, &["INS", "In [3]", "Ctrl-P palette"]);
    assert_last_prompt_line_contains_none(
        &output.after,
        &[
            "Connecting to kernel...",
            "Kernel busy. Ctrl-C to interrupt",
        ],
    );
}

#[test]
fn bottom_pinned_streaming_output_then_short_output_executes_cleanly() {
    let Some(output) = run_repro(
        "bottom-pinned-streaming-output",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            (
                "PRE_INPUT",
                "print(\"\\n\".join(f\"fill {i}\" for i in range(30)))\nimport sys,time; exec(\"for i in range(8):\\n sys.stdout.write(f\\\"progress {i}\\\" + chr(13)); sys.stdout.flush(); time.sleep(0.02)\\nprint(\\\"done\\\")\")\nprint(\"after\")",
            ),
            (
                "INPUTS",
                "print(\"\\n\".join(f\"fill {i}\" for i in range(30)))\nimport sys,time; exec(\"for i in range(8):\\n sys.stdout.write(f\\\"progress {i}\\\" + chr(13)); sys.stdout.flush(); time.sleep(0.02)\\nprint(\\\"done\\\")\")\nprint(\"after\")",
            ),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_LINES", "160"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "progress 7");
    assert_contains(&output.after, "done");
    assert_contains(&output.after, "In [3]: print(\"after\")");
    assert_contains(&output.after, "after");
    assert_not_contains(&output.after, "1 print(\"after\")");
    assert_last_prompt_line_contains_all(&output.after, &["INS", "In [4]", "Ctrl-P palette"]);
}

#[test]
fn bottom_pinned_transcript_repaint_clears_stale_busy_status() {
    let Some(output) = run_repro(
        "bottom-pinned-stale-busy",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            (
                "PRE_INPUT",
                "print(\"\\n\".join(f\"fill {i}\" for i in range(30)))",
            ),
            (
                "INPUTS",
                "print(\"\\n\".join(f\"fill {i}\" for i in range(30)))",
            ),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_LINES", "80"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "fill 29");
    assert_contains(&output.after, "[");
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
}

#[test]
fn stdin_reply_is_sent_on_enter() {
    let Some(output) = run_repro(
        "stdin-reply",
        "stdin-reply",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
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
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
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
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
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
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
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
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
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
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
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
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
    ) else {
        return;
    };

    assert_contains(
        &output.after,
        "In [1]: import pdb; pdb.set_trace(); print(\"after\")",
    );
    assert!(
        output.after.contains("(Pdb) where") || output.after.contains("ipdb> where"),
        "expected a visible debugger prompt for `where` in output:\n{}",
        output.after
    );
    assert!(
        output.after.contains("(Pdb) c") || output.after.contains("ipdb> c"),
        "expected a visible debugger prompt for `c` in output:\n{}",
        output.after
    );
    assert_contains(&output.after, "after");
    assert_debugger_command_line_count(&output.after, "where", 1);
    assert_debugger_command_line_count(&output.after, "c", 1);
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

    assert!(
        output
            .after
            .contains("1 use edtui::{EditorState, EditorTheme, EditorView};")
            || output
                .after
                .contains("In [1]: use edtui::{EditorState, EditorTheme, EditorView};"),
        "expected first pasted line in output:\n{}",
        output.after
    );
    assert_contains(&output.after, "use ratatui::widgets::Widget;");
    assert_contains(&output.after, "let mut state = EditorState::default();");
    assert_contains(&output.after, ".tab_width(2)");
    assert_contains(&output.after, ".render(area, buf)");
    assert_ordered_contains(
        &output.after,
        &[
            "use edtui::{EditorState, EditorTheme, EditorView};",
            "use ratatui::widgets::Widget;",
            "let mut state = EditorState::default();",
            "EditorView::new(&mut state)",
            ".theme(EditorTheme::default())",
            ".wrap(true)",
            ".syntax_highlighter(None)",
            ".tab_width(2)",
            ".render(area, buf)",
        ],
    );
}

#[test]
fn can_compose_while_kernel_is_busy() {
    let Some(output) = run_repro(
        "compose-while-busy",
        "compose-while-busy",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: import time; time.sleep(2); 42");
    assert!(
        output.after.contains("1 1+1") || output.after.contains("In [2]: 1+1"),
        "expected typed input while busy in output:\n{}",
        output.after
    );
    assert_line_contains_all(
        &output.after,
        &[
            "INS",
            "In [2]",
            "Kernel busy. Ctrl-C to interrupt",
            "Ctrl-P palette",
        ],
    );
}

#[test]
fn shift_enter_keeps_typing_on_second_editor_line() {
    let Some(output) = run_repro(
        "shift-enter-second-line",
        "shift-enter-type-second-line",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
    ) else {
        return;
    };

    assert!(
        output.after.contains("1 abc") || output.after.contains("In [1]: abc"),
        "expected first editor line in output:\n{}",
        output.after
    );
    assert!(
        output.after.contains("2 def") || output.after.contains("        def"),
        "expected second editor line in output:\n{}",
        output.after
    );
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "Out[1]:");
}

#[test]
fn ctrl_c_after_multiline_resets_prompt_spacing() {
    let Some(output) = run_repro(
        "ctrl-c-multiline",
        "ctrl-c-multiline",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
    ) else {
        return;
    };

    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
    assert_no_line_starts_with(&output.after, "2 ");
    assert_not_contains(&output.after, "abc");
    assert_not_contains(&output.after, "def");
}

#[test]
fn ctrl_c_after_multiline_leaves_gap_below_prompt() {
    let Some(output) = run_repro(
        "ctrl-c-multiline-bottom",
        "ctrl-c-multiline-bottom",
        &[
            ("TMUX_SIZE", "120x20"),
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_line_contains_all(&output.after, &["1"]);
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
    assert_blank_line_after_contains(&output.after, "Ctrl-P palette");
    assert_not_contains(&output.after, "abc");
    assert_not_contains(&output.after, "def");
}

#[test]
fn vim_open_below_grows_on_first_try() {
    let Some(output) = run_repro(
        "vim-open-below",
        "vim-open-below",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
    ) else {
        return;
    };

    assert_line_contains_all(&output.after, &["1", "abc"]);
    assert_line_contains_all(&output.after, &["2"]);
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
}

#[test]
fn repeated_resize_preserves_transcript_and_keeps_inline_prompt_clean() {
    let Some(output) = run_repro(
        "resize-cycle",
        "resize-cycle",
        &[
            ("TMUX_SIZE", "120x30"),
            ("PRE_INPUT", "1+1\n2+2"),
            ("INPUTS", "1+1\n2+2"),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_LINES", "120"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
    assert_contains(&output.after, "In [2]: 2+2");
    assert_contains(&output.after, "Out[2]: 4");
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_line_contains_all(&output.after, &["resize_check"]);
    assert_line_contains_all(&output.after, &["INS", "In [3]", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
}

#[test]
fn resize_long_editor_full_resets_projection() {
    let long_text = format!("{:?}", "a".repeat(500));
    let Some(output) = run_repro(
        "resize-long-editor",
        "resize-long-editor",
        &[
            ("TMUX_SIZE", "120x30"),
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("LONG_EDITOR_TEXT", &long_text),
            ("VISUAL_SENTINEL", "resize-anchor-sentinel"),
            ("CAPTURE_VISIBLE_ONLY", "1"),
            ("EXIT_WAIT", "0.10"),
        ],
    ) else {
        return;
    };

    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
    assert_contains(&output.after, "aaaaaaaaaaaaaaaaaaaaaaaa");
    assert_contains(&output.after, "Out[1]:");
    assert_not_contains(&output.after, "resize-anchor-sentinel");
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
    assert!(
        output.meta_value("fpy_row") <= 2,
        "fpy should redraw a clean top-anchored frame after resize clears scrollback; meta:\n{}\nafter:\n{}",
        output.after_meta,
        output.after
    );
    assert!(
        output.meta_value("editor_row") > output.meta_value("fpy_row"),
        "stale reflowed editor rows should not remain above the fpy frame after resize; meta:\n{}\nafter:\n{}",
        output.after_meta,
        output.after
    );
    assert!(
        output.meta_value("prompt_row") > output.meta_value("editor_row"),
        "prompt should remain below the reflowed long editor after resize; meta:\n{}\nafter:\n{}",
        output.after_meta,
        output.after
    );
}

#[test]
fn repeated_resize_with_palette_open_stays_clean() {
    let Some(output) = run_repro(
        "resize-palette-cycle",
        "resize-palette-cycle",
        &[
            ("TMUX_SIZE", "120x30"),
            ("PRE_INPUT", "1+1"),
            ("INPUTS", "1+1"),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_LINES", "120"),
        ],
    ) else {
        return;
    };

    assert_line_count(&output.after, "Command Palette", 1);
    assert_line_count(&output.after, "Interrupt Kernel", 1);
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
}

#[test]
fn history_up_reruns_previous_cell() {
    let Some(output) = run_repro(
        "history-up",
        "history-up",
        &[
            ("PRE_INPUT", "1+1\n2+2"),
            ("INPUTS", "1+1\n2+2"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [2]: 2+2");
    assert_contains(&output.after, "Out[2]: 4");
    assert_contains(&output.after, "In [3]: 2+2");
    assert_contains(&output.after, "Out[3]: 4");
}

#[test]
fn ctrl_k_reruns_previous_history_cell() {
    let Some(output) = run_repro(
        "history-ctrl-k",
        "history-ctrl-k",
        &[
            ("PRE_INPUT", "1+1\n2+2"),
            ("INPUTS", "1+1\n2+2"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [2]: 2+2");
    assert_contains(&output.after, "Out[2]: 4");
    assert_contains(&output.after, "In [3]: 2+2");
    assert_contains(&output.after, "Out[3]: 4");
}

#[test]
fn ctrl_j_moves_back_down_from_history_to_blank_input() {
    let Some(output) = run_repro(
        "history-ctrl-k-ctrl-j",
        "history-ctrl-k-ctrl-j",
        &[
            ("PRE_INPUT", "1+1\n2+2"),
            ("INPUTS", "1+1\n2+2"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [3]: 3+3");
    assert_contains(&output.after, "Out[3]: 6");
    assert_not_contains(&output.after, "In [3]: 2+2");
}

#[test]
fn bottom_print_preserves_input_cell_after_scrolling_output() {
    let Some(output) = run_repro(
        "bottom-print-preserves-input",
        "none",
        &[
            ("TMUX_SIZE", "80x12"),
            (
                "INPUTS",
                "print('\\n'.join(str(i) for i in range(20)))\nprint('\\n'.join(str(i) for i in range(10)))",
            ),
            ("CAPTURE_LINES", "120"),
            ("EXIT_WAIT", "0.2"),
        ],
    ) else {
        return;
    };

    assert_contains(
        &output.after,
        "In [2]: print('\\n'.join(str(i) for i in range(10)))",
    );
    for line in 0..=9 {
        assert_contains(&output.after, &format!("\n{line}\n"));
    }
}

#[test]
fn bottom_started_stdout_preserves_all_lines() {
    let Some(output) = run_repro(
        "bottom-started-stdout",
        "none",
        &[
            ("TMUX_SIZE", "80x12"),
            ("PRE_LAUNCH_FILL_LINES", "11"),
            ("PRE_INPUT", "print('\\n'.join(str(i) for i in range(8)))"),
            ("INPUTS", "print('\\n'.join(str(i) for i in range(8)))"),
            ("CAPTURE_LINES", "80"),
            ("EXIT_WAIT", "0.2"),
        ],
    ) else {
        return;
    };

    assert_contains(
        &output.after,
        "In [1]: print('\\n'.join(str(i) for i in range(8)))",
    );
    for line in 0..=7 {
        assert_contains(&output.after, &format!("\n{line}\n"));
    }
}

#[test]
fn history_search_shows_multiple_results_and_multiline_preview() {
    let history_dir = TempDir::new().expect("history dir");
    write_history_record(
        history_dir.path(),
        "import torch\ntorch.cuda.is_available()",
        Some(800_000_000),
        None,
    );
    write_history_record(
        history_dir.path(),
        "import time\ntime.sleep(1)\n42",
        Some(1_000_000_000),
        None,
    );
    write_history_record(
        history_dir.path(),
        "import os\nos.getcwd()",
        Some(5_000_000),
        None,
    );

    let Some(output) = run_repro(
        "history-search-open",
        "history-search-open",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.05"),
            (
                "FPY_HISTORY_DIR",
                history_dir.path().to_str().expect("utf8 path"),
            ),
            ("SEARCH_QUERY", "import"),
            ("CAPTURE_LINES", "80"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "History Search");
    assert_contains(&output.after, "query: import");
    assert_contains(&output.after, "import os …");
    assert_contains(&output.after, "import time …");
    assert_contains(&output.after, "import torch …");
    assert_contains(&output.after, "preview");
    assert_contains(&output.after, "os.getcwd()");
    assert_contains(&output.after_ansi, "> \u{1b}[38;2;");
    assert_contains(&output.after_ansi, "\u{1b}[39m");
    assert_contains(&output.after_ansi, "getcwd");
}

#[test]
fn bottom_started_history_search_shows_results_not_just_preview() {
    let history_dir = TempDir::new().expect("history dir");
    write_history_record(
        history_dir.path(),
        "import os\nos.getcwd()",
        Some(5_000_000),
        None,
    );
    write_history_record(
        history_dir.path(),
        "import time\ntime.sleep(1)\n42",
        Some(1_000_000_000),
        None,
    );

    let Some(output) = run_repro(
        "bottom-started-history-search",
        "history-search-open",
        &[
            ("TMUX_SIZE", "80x12"),
            ("PRE_LAUNCH_FILL_LINES", "11"),
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.05"),
            (
                "FPY_HISTORY_DIR",
                history_dir.path().to_str().expect("utf8 path"),
            ),
            ("SEARCH_QUERY", "import"),
            ("CAPTURE_LINES", "80"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "History Search");
    assert_contains(&output.after, "query: import");
    assert_contains(&output.after, "> import time …");
    assert_contains(&output.after, "  import os …");
    assert_contains(&output.after, "preview");
}

#[test]
fn bottom_started_history_search_close_then_print_preserves_transcript() {
    let history_dir = TempDir::new().expect("history dir");
    write_history_record(
        history_dir.path(),
        "import os\nos.getcwd()",
        Some(5_000_000),
        None,
    );
    write_history_record(
        history_dir.path(),
        "import time\ntime.sleep(1)\n42",
        Some(1_000_000_000),
        None,
    );

    let Some(output) = run_repro(
        "bottom-started-history-close-print",
        "history-search-close-then-print",
        &[
            ("TMUX_SIZE", "80x12"),
            ("PRE_LAUNCH_FILL_LINES", "11"),
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.2"),
            (
                "FPY_HISTORY_DIR",
                history_dir.path().to_str().expect("utf8 path"),
            ),
            ("SEARCH_QUERY", "import"),
            ("CAPTURE_LINES", "120"),
        ],
    ) else {
        return;
    };

    assert_contains(
        &output.after,
        "In [1]: print('\\n'.join(str(i) for i in range(10)))",
    );
    for line in 0..=9 {
        assert_contains(&output.after, &format!("\n{line}\n"));
    }
    let output_lines = output.after.lines().collect::<Vec<_>>();
    let output_start = output_lines
        .iter()
        .position(|line| *line == "0")
        .expect("output starts with 0");
    assert_eq!(
        &output_lines[output_start..output_start + 10],
        ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
    );
    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_not_contains(&output.after, "History Search");
}

#[test]
fn history_search_matches_multiline_cells_and_loads_previewed_code() {
    let history_dir = TempDir::new().expect("history dir");
    write_history_record(
        history_dir.path(),
        "import torch\ntorch.cuda.is_available()",
        Some(800_000_000),
        None,
    );

    let Some(output) = run_repro(
        "history-search-load",
        "history-search-load",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.05"),
            (
                "FPY_HISTORY_DIR",
                history_dir.path().to_str().expect("utf8 path"),
            ),
            ("SEARCH_QUERY", "cuda"),
        ],
    ) else {
        return;
    };

    assert_line_contains_all(&output.after, &["1", "import torch"]);
    assert_line_contains_all(&output.after, &["2", "torch.cuda.is_available()"]);
    assert_line_contains_all(&output.after, &["INS", "In [1]", "[1/1]", "Ctrl-P palette"]);
}

#[test]
fn history_search_load_seeds_ctrl_j_back_to_loaded_multiline_cell() {
    let history_dir = TempDir::new().expect("history dir");
    write_history_record(
        history_dir.path(),
        "alpha = 1\nalpha",
        Some(1_000_000),
        None,
    );
    write_history_record(history_dir.path(), "beta = 2\nbeta", Some(2_000_000), None);

    let Some(output) = run_repro(
        "history-search-load-ctrl-k-ctrl-j",
        "history-search-load-ctrl-k-ctrl-j",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.05"),
            (
                "FPY_HISTORY_DIR",
                history_dir.path().to_str().expect("utf8 path"),
            ),
            ("SEARCH_QUERY", "beta"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "In [1]: beta = 2");
    assert_contains(&output.after, "Out[1]: 2");
}

#[test]
fn history_search_query_relayout_does_not_mix_stale_rows() {
    let history_dir = TempDir::new().expect("history dir");
    write_history_record(
        history_dir.path(),
        "def fibonacci(n):\n    a, b = 0, 1\n    for _ in range(n):\n        a, b = b, a + b\n    return a",
        Some(10_300_000),
        None,
    );
    write_history_record(
        history_dir.path(),
        "def fibonacci(n):\n    if n < 2:\n        return n\n    return fibonacci(n - 1) + fibonacci(n - 2)",
        Some(9_420_000),
        None,
    );
    for duration_ns in [
        20_100_000_000,
        220_100_000_000,
        720_100_000_000,
        20_000_000_000,
    ] {
        write_history_record(
            history_dir.path(),
            "import pdb; pdb.set_trace(); print(\"after\")",
            Some(duration_ns),
            None,
        );
    }

    let Some(output) = run_repro(
        "history-search-relayout-clean",
        "history-search-open",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.05"),
            (
                "FPY_HISTORY_DIR",
                history_dir.path().to_str().expect("utf8 path"),
            ),
            ("SEARCH_QUERY", "def fib"),
            ("TMUX_SIZE", "120x25"),
            ("CAPTURE_LINES", "80"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "History Search");
    assert_contains(&output.after, "query: def fib");
    assert_contains(&output.after, "> def fibonacci(n): …");
    assert_contains(&output.after, "  def fibonacci(n): …");
    assert_contains(&output.after, "preview");
    assert!(
        output.after.contains("    a, b = 0, 1") || output.after.contains("    if n < 2:"),
        "expected a clean multiline preview in output:\n{}",
        output.after
    );
    assert_not_contains(&output.after, "previewt");
    assert_not_contains(&output.after, "defofibonacci");
}

#[test]
fn persistent_history_is_available_in_new_sessions() {
    let history_dir = TempDir::new().expect("history dir");
    let history_dir_path = history_dir.path().display().to_string();

    let Some(first) = run_repro(
        "persistent-history-write",
        "ctrl-d",
        &[
            ("PRE_INPUT", "40+2"),
            ("INPUTS", "40+2"),
            ("EXIT_WAIT", "0.05"),
            ("FPY_HISTORY_DIR", history_dir_path.as_str()),
        ],
    ) else {
        return;
    };
    assert_contains(&first.after, "Out[1]: 42");

    let Some(second) = run_repro(
        "persistent-history-read",
        "history-up",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("EXIT_WAIT", "0.05"),
            ("FPY_HISTORY_DIR", history_dir_path.as_str()),
        ],
    ) else {
        return;
    };

    assert_contains(&second.after, "In [1]: 40+2");
    assert_contains(&second.after, "Out[1]: 42");
}

#[test]
fn ctrl_l_clears_visible_screen_to_single_prompt() {
    let Some(output) = run_repro(
        "ctrl-l-visible-clear",
        "ctrl-l",
        &[
            ("TMUX_SIZE", "120x20"),
            (
                "PRE_INPUT",
                "print('alpha')\nprint('beta')\nprint('gamma')\nprint('delta')\n1+1",
            ),
            (
                "INPUTS",
                "print('alpha')\nprint('beta')\nprint('gamma')\nprint('delta')\n1+1",
            ),
            ("EXIT_WAIT", "0.05"),
            ("CAPTURE_VISIBLE_ONLY", "1"),
        ],
    ) else {
        return;
    };

    assert_line_count(&output.after, "Ctrl-P palette", 1);
    assert_last_prompt_line_contains_all(&output.after, &["INS", "In [6]", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "Out[5]: 2");
    assert_not_contains(&output.after, "In [5]: 1+1");
    assert_not_contains(&output.after, "alpha");
    assert_not_contains(&output.after, "print('alpha')");
}

#[test]
fn palette_move_close_reopen_does_not_mix_with_status_row() {
    let Some(output) = run_repro(
        "palette-move-cycle",
        "palette-move-cycle",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "0.05")],
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

#[test]
fn palette_interrupt_busy_cell_does_not_leave_stale_output() {
    let Some(output) = run_repro(
        "palette-interrupt-busy",
        "palette-interrupt-busy",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("CAPTURE_VISIBLE_ONLY", "1"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "^C");
    assert_line_contains_all(&output.after, &["INS", "In [2]", "Ctrl-P palette"]);
    assert_not_contains(&output.after, "Out[1]:");
    assert_not_contains(&output.after, "Restart Kernel");
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
    assert_not_contains(&output.after, "Command Palette");
}

#[test]
fn palette_restart_busy_cell_does_not_leave_stale_output() {
    let Some(output) = run_repro(
        "palette-restart-busy",
        "palette-restart-busy",
        &[
            ("PRE_INPUT", ""),
            ("INPUTS", ""),
            ("CAPTURE_VISIBLE_ONLY", "1"),
            ("EXIT_WAIT", "0.05"),
        ],
    ) else {
        return;
    };

    assert_contains(&output.after, "kernel restarted");
    assert_line_contains_all(&output.after, &["INS", "In [1]", "Ctrl-P palette"]);
    assert_line_count(&output.after, "kernel restarted", 1);
    assert_not_contains(&output.after, "Restart Kernel");
    assert_not_contains(&output.after, "Kernel busy. Ctrl-C to interrupt");
    assert_not_contains(&output.after, "Command Palette");
}

struct ReproOutput {
    #[allow(dead_code)]
    before: String,
    after: String,
    after_ansi: String,
    after_meta: String,
}

impl ReproOutput {
    fn meta_value(&self, key: &str) -> usize {
        self.after_meta
            .lines()
            .find_map(|line| {
                let (line_key, value) = line.split_once('=')?;
                (line_key == key)
                    .then(|| value.parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing numeric meta value {key:?} in:\n{}",
                    self.after_meta
                )
            })
    }
}

fn run_repro(name: &str, action: &str, extra_env: &[(&str, &str)]) -> Option<ReproOutput> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(reason) = missing_prerequisites(&repo_root) {
        panic!("tmux e2e prerequisites missing: {reason}");
    }

    let unique = unique_id();
    let target_dir = repo_root.join("target");
    let before_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.before.log"));
    let after_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.after.log"));
    let after_ansi_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.after.ansi.log"));
    let after_meta_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.after.meta"));
    let session = format!("fpy-e2e-{name}-{unique}");
    let fpy_bin = option_env!("CARGO_BIN_EXE_fpy")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .unwrap_or_else(|| repo_root.join("target/debug/fpy"));

    let default_history_dir = (!extra_env.iter().any(|(key, _)| *key == "FPY_HISTORY_DIR"))
        .then(|| TempDir::new().expect("test history dir"));

    let mut command = Command::new(repo_root.join("scripts/fpy-tmux-repro.sh"));
    command.current_dir(&repo_root).arg(action);
    command.env("SESSION", &session);
    command.env("BEFORE_LOG", &before_log);
    command.env("AFTER_LOG", &after_log);
    command.env("AFTER_ANSI_LOG", &after_ansi_log);
    command.env("AFTER_META_LOG", &after_meta_log);
    if let Some(history_dir) = &default_history_dir {
        command.env("FPY_HISTORY_DIR", history_dir.path());
    }
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
        after_ansi: read_log(&after_ansi_log),
        after_meta: read_log(&after_meta_log),
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
    let count = haystack
        .lines()
        .filter(|line| line.contains(needle))
        .count();
    assert_eq!(
        count, expected,
        "expected {needle:?} to appear {expected} time(s) in output:\n{haystack}"
    );
}

fn assert_ordered_contains(haystack: &str, needles: &[&str]) {
    let mut offset = 0;
    for needle in needles {
        let remaining = &haystack[offset..];
        let found = remaining.find(needle).unwrap_or_else(|| {
            panic!("expected to find {needle:?} after byte offset {offset} in:\n{haystack}")
        });
        offset += found + needle.len();
    }
}

fn assert_debugger_command_line_count(haystack: &str, command: &str, expected: usize) {
    let pdb = format!("(Pdb) {command}");
    let ipdb = format!("ipdb> {command}");
    let count = haystack
        .lines()
        .filter(|line| line.contains(&pdb) || line.contains(&ipdb))
        .count();
    assert_eq!(
        count, expected,
        "expected debugger command {command:?} to appear {expected} time(s) in output:\n{haystack}"
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
    let lines = haystack.split('\n').collect::<Vec<_>>();
    let index = lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "expected to find line containing {:?} in output:\n{}",
                needle, haystack
            )
        });
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
        .unwrap_or_else(|| {
            panic!(
                "expected to find line containing {:?} in output:\n{}",
                needle, haystack
            )
        });
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

fn assert_last_prompt_line_contains_all(haystack: &str, needles: &[&str]) {
    let prompt_line = haystack
        .lines()
        .rev()
        .find(|line| line.contains("Ctrl-P palette"))
        .unwrap_or_else(|| panic!("expected a prompt line in output:\n{haystack}"));
    assert!(
        needles.iter().all(|needle| prompt_line.contains(needle)),
        "expected last prompt line to contain all of {:?}, got {:?}\n{}",
        needles,
        prompt_line,
        haystack
    );
}

fn assert_last_prompt_line_contains_none(haystack: &str, needles: &[&str]) {
    let prompt_line = haystack
        .lines()
        .rev()
        .find(|line| line.contains("Ctrl-P palette"))
        .unwrap_or_else(|| panic!("expected a prompt line in output:\n{haystack}"));
    assert!(
        needles.iter().all(|needle| !prompt_line.contains(needle)),
        "expected last prompt line to contain none of {:?}, got {:?}\n{}",
        needles,
        prompt_line,
        haystack
    );
}

fn assert_shell_prompt_immediately_after_last_prompt(haystack: &str) {
    let lines = haystack.lines().collect::<Vec<_>>();
    let prompt_index = lines
        .iter()
        .rposition(|line| line.contains("Ctrl-P palette"))
        .unwrap_or_else(|| panic!("expected fpy prompt/status line in output:\n{haystack}"));
    let next = lines
        .get(prompt_index + 1)
        .unwrap_or_else(|| panic!("expected shell prompt after fpy prompt:\n{haystack}"));
    assert!(
        !next.trim().is_empty(),
        "expected non-blank shell prompt immediately after fpy prompt, got {:?}\n{}",
        next,
        haystack
    );
}

fn write_history_record(root: &Path, code: &str, duration_ns: Option<u64>, outcome: Option<&str>) {
    let host = "test-host";
    let host_dir = root.join(host);
    fs::create_dir_all(&host_dir).expect("create host history dir");
    let session_id = Uuid::now_v7();
    let path = host_dir.join(format!("{session_id}-123.jsonl"));

    let mut contents = format!(
        "{{\"v\":1,\"type\":\"cell\",\"session_id\":\"{}\",\"entry_seq\":1,\"ts_unix_ns\":1,\"host\":\"{}\",\"pid\":123,\"code\":{}}}\n",
        session_id,
        host,
        serde_json::to_string(code).expect("serialize code"),
    );
    if let Some(duration_ns) = duration_ns {
        let outcome = outcome.unwrap_or("ok");
        contents.push_str(&format!(
            "{{\"v\":1,\"type\":\"cell_done\",\"session_id\":\"{}\",\"entry_seq\":1,\"ts_unix_ns\":2,\"duration_ns\":{},\"outcome\":\"{}\"}}\n",
            session_id,
            duration_ns,
            outcome,
        ));
    }
    fs::write(path, contents).expect("write history record");
}

fn wait_for_submit_ready(session: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let output = Command::new("tmux")
            .args(["capture-pane", "-pt", session, "-S", "-40"])
            .output()
            .expect("capture tmux pane");
        let pane = String::from_utf8_lossy(&output.stdout);
        let prompt_line = pane
            .lines()
            .rev()
            .find(|line| line.contains("Ctrl-P palette"));
        let ready = prompt_line.is_some_and(|line| {
            !line.contains("Connecting to kernel...")
                && !line.contains("Kernel busy. Ctrl-C to interrupt")
        });
        if ready {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for submit-ready pane:\n{pane}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_stable_file_size(path: &Path, stable_for: Duration, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut previous = file_len(path);
    let mut stable_since = Instant::now();

    loop {
        thread::sleep(Duration::from_millis(50));
        let current = file_len(path);
        if current == previous {
            if Instant::now().duration_since(stable_since) >= stable_for {
                return;
            }
        } else {
            previous = current;
            stable_since = Instant::now();
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for stable ansi log at {}",
            path.display()
        );
    }
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn missing_prerequisites(repo_root: &Path) -> Option<String> {
    static MISSING_PREREQUISITES: OnceLock<Option<String>> = OnceLock::new();

    MISSING_PREREQUISITES
        .get_or_init(|| detect_missing_prerequisites(repo_root))
        .clone()
}

fn detect_missing_prerequisites(repo_root: &Path) -> Option<String> {
    let repro = repo_root.join("scripts/fpy-tmux-repro.sh");
    if !repro.exists() {
        return Some(format!("missing {}", repro.display()));
    }

    let python = repo_root.join(".venv/bin/python");
    let python_ok = python.exists()
        || matches!(
            Command::new("python3")
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
            Ok(status) if status.success()
        );
    if !python_ok {
        return Some("python3 is not available".to_string());
    }

    let tmux = Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
