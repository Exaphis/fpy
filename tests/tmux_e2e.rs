use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn ctrl_d_preserves_transcript() {
    let output = run_repro("ctrl-d-preserves", "ctrl-d", &[]);
    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
    assert_contains(&output.after, "kevin@mango-pro");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn kernel_exit_returns_shell() {
    let output = run_repro("kernel-exit", "exitpy", &[]);
    assert_contains(&output.after, "In [1]: 1+1");
    assert_contains(&output.after, "Out[1]: 2");
    assert_contains(&output.after, "kevin@mango-pro");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn multiline_growth_bottom_pinned() {
    let output = run_repro(
        "multiline-growth-bottom",
        "paste",
        &[
            ("TMUX_SIZE", "120x40"),
            ("PRE_INPUT", "1+1\n!ls -lah\n!ls -lah"),
            ("INPUTS", "1+1\n!ls -lah\n!ls -lah"),
            ("PASTE_TEXT", "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl"),
        ],
    );

    assert_contains(&output.after, "In [2]: !ls -lah");
    assert_contains(&output.after, "In [3]: !ls -lah");
    assert_contains(&output.after, " 1 a");
    assert_contains(&output.after, "12 l");
    assert_contains(&output.after, "INS  In [4]  Ctrl-P palette");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn bottom_of_screen_result_still_visible() {
    let output = run_repro(
        "bottom-result-visible",
        "none",
        &[
            ("TMUX_SIZE", "120x20"),
            ("PRE_INPUT", "!ls -lah\n!ls -lah\n!ls -lah\n!ls -lah\n1+1"),
            ("INPUTS", "!ls -lah\n!ls -lah\n!ls -lah\n!ls -lah\n1+1"),
            ("EXIT_WAIT", "1"),
        ],
    );

    assert_contains(&output.after, "In [5]: 1+1");
    assert_contains(&output.after, "Out[5]: 2");
    assert_contains(&output.after, "INS  In [6]  Ctrl-P palette");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn multiline_paste_preserves_all_lines() {
    let output = run_repro(
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
    );

    assert_contains(
        &output.after,
        "1 use edtui::{EditorState, EditorTheme, EditorView};",
    );
    assert_contains(&output.after, "2 use ratatui::widgets::Widget;");
    assert_contains(&output.after, "4 let mut state = EditorState::default();");
    assert_contains(&output.after, "9         .render(area, buf);");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn can_compose_while_kernel_is_busy() {
    let output = run_repro(
        "compose-while-busy",
        "compose-while-busy",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    );

    assert_contains(&output.after, "In [1]: import time; time.sleep(3); 42");
    assert_contains(&output.after, "1 1+1");
    assert_contains(&output.after, "INS  In [2]  Ctrl-P palette  Kernel busy. Ctrl-C to interrupt");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn shift_enter_creates_multiline_editor() {
    let output = run_repro(
        "shift-enter",
        "shift-enter",
        &[("PRE_INPUT", ""), ("INPUTS", ""), ("EXIT_WAIT", "1")],
    );

    assert_contains(&output.after, "1 abc");
    assert_contains(&output.after, "2");
    assert_contains(&output.after, "INS  In [1]  Ctrl-P palette");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn vim_open_below_grows_on_first_try() {
    let output = run_repro(
        "vim-open-below",
        "vim-open-below",
        &[("EXIT_WAIT", "1")],
    );

    assert_contains(&output.after, "1");
    assert_contains(&output.after, "2");
    assert_contains(&output.after, "INS  In [2]  Ctrl-P palette");
}

#[test]
#[ignore = "requires tmux, local ipykernel, and a real terminal multiplexer environment"]
fn history_up_reruns_previous_cell() {
    let output = run_repro(
        "history-up",
        "history-up",
        &[("PRE_INPUT", "1+1\n2+2"), ("INPUTS", "1+1\n2+2"), ("EXIT_WAIT", "1")],
    );

    assert_contains(&output.after, "In [2]: 2+2");
    assert_contains(&output.after, "Out[2]: 4");
    assert_contains(&output.after, "In [3]: 2+2");
    assert_contains(&output.after, "Out[3]: 4");
}

struct ReproOutput {
    #[allow(dead_code)]
    before: String,
    after: String,
}

fn run_repro(name: &str, action: &str, extra_env: &[(&str, &str)]) -> ReproOutput {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    require_path(repo_root.join("scripts/fpy-tmux-repro.sh"), "scripts/fpy-tmux-repro.sh");
    require_path(repo_root.join(".venv/bin/python"), ".venv/bin/python");
    require_command("tmux");

    let unique = unique_id();
    let target_dir = repo_root.join("target");
    let before_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.before.log"));
    let after_log = target_dir.join(format!("tmux-e2e-{name}-{unique}.after.log"));
    let session = format!("fpy-e2e-{name}-{unique}");

    let mut command = Command::new(repo_root.join("scripts/fpy-tmux-repro.sh"));
    command.current_dir(&repo_root).arg(action);
    command.env("SESSION", &session);
    command.env("BEFORE_LOG", &before_log);
    command.env("AFTER_LOG", &after_log);
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

    ReproOutput {
        before: read_log(&before_log),
        after: read_log(&after_log),
    }
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

fn require_command(command: &str) {
    let status = Command::new(command).arg("-V").status();
    assert!(
        matches!(status, Ok(status) if status.success()),
        "required command {command:?} is not available"
    );
}

fn require_path(path: PathBuf, display: &str) {
    assert!(path.exists(), "required path {display:?} does not exist");
}

fn unique_id() -> u64 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos() as u64;
    time ^ u64::from(std::process::id()) ^ NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
