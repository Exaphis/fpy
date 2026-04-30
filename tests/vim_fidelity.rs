//! Optional golden tests for edtui/fpy Vim-mode fidelity.
//!
//! These tests run the same keystroke script through edtui and through a real
//! Vim binary, then compare the final buffer. They are ignored by default
//! because they require `vim` on PATH and spawn external processes.
//!
//! Run with:
//!   FPY_VIM=vim cargo test --test vim_fidelity -- --ignored --nocapture

use std::{fs, process::Command};

use edtui::{events::KeyInput, EditorEventHandler, EditorState, Lines};
use tempfile::tempdir;

#[derive(Clone, Copy, Debug)]
struct Case {
    name: &'static str,
    initial: &'static str,
    keys: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "delete_word",
        initial: "one two three",
        keys: "wdw",
    },
];

#[test]
#[ignore = "requires a real Vim binary; run with FPY_VIM=vim cargo test --test vim_fidelity -- --ignored"]
fn edtui_matches_real_vim_for_golden_cases() {
    let vim = vim_binary();

    for case in CASES {
        let edtui = run_edtui(case.initial, case.keys);
        let vim = run_vim(&vim, case.initial, case.keys);
        assert_eq!(
            edtui, vim,
            "case {:?} with keys {:?}",
            case.name, case.keys
        );
    }
}

#[test]
#[ignore = "requires a real Vim binary; run with FPY_VIM=vim FPY_VIM_FUZZ_ITERS=100 cargo test --test vim_fidelity -- --ignored"]
fn fuzz_supported_vim_normal_mode_sequences_against_real_vim() {
    let vim = vim_binary();
    let iterations = std::env::var("FPY_VIM_FUZZ_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100);
    let mut rng = Lcg::new(
        std::env::var("FPY_VIM_FUZZ_SEED")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0xf9_f1_de_11_7e),
    );

    // Keep this corpus to commands we intentionally support and have not already
    // documented as divergences. Add commands here as edtui's Vim fidelity grows.
    let single_line_atoms = [
        "h", "j", "k", "l", "w", "b", "e", "W", "B", "E", "0", "_", "$", "gg", "G", "x",
        "dd", "dw", "D", "J", "iX<Esc>", "IX<Esc>", "aX<Esc>", "AX<Esc>", "OX<Esc>",
        "iX<Enter>Y<Esc>", "aX<Enter>Y<Esc>",
        "2dd", "2h", "2l", "2w", "2b", "2e", "2W", "2B", "2E", "2x", "3w", "3b", "3e",
        "u", "<C-r>",
    ];
    let multiline_atoms = [
        "h", "j", "k", "l", "w", "b", "e", "W", "B", "E", "0", "_", "$", "gg", "G", "x",
        "dd", "dw", "D", "J", "iX<Esc>", "IX<Esc>", "aX<Esc>", "AX<Esc>", "OX<Esc>",
        "iX<Enter>Y<Esc>", "aX<Enter>Y<Esc>",
        "2dd", "2h", "2l", "2w", "2b", "2e", "2W", "2B", "2E", "2x", "3w", "3b", "3e", "2j", "2k",
        "u", "<C-r>",
    ];
    let initials = [
        "one two three",
        "alpha beta\ngamma delta",
        "abc def\nxyz",
        "a b c d e",
    ];

    for iteration in 0..iterations {
        let initial = initials[rng.usize(initials.len())];
        let atoms = if initial.contains('\n') {
            &multiline_atoms[..]
        } else {
            &single_line_atoms[..]
        };
        let mut keys = String::new();
        let mut steps = Vec::new();
        let atom_count = 1 + rng.usize(12);
        for _ in 0..atom_count {
            let atom = atoms[rng.usize(atoms.len())];
            keys.push_str(atom);
            steps.push(atom);
        }

        let edtui = run_edtui(initial, &keys);
        let vim_snapshot = run_vim_steps(&vim, initial, &steps);
        if edtui != vim_snapshot {
            panic!(
                "fuzz iteration {iteration}, initial {initial:?}, keys {keys:?}\nedtui: {edtui:?}\nvim:   {vim_snapshot:?}\nedtui trace: {:?}\nvim trace:   {:?}",
                trace_edtui(initial, &steps),
                trace_vim_steps(&vim, initial, &steps),
            );
        }
    }
}

fn vim_binary() -> String {
    let vim = std::env::var("FPY_VIM").unwrap_or_else(|_| "vim".to_string());
    assert!(
        Command::new(&vim).arg("--version").output().is_ok(),
        "Vim binary not found: {vim:?}"
    );
    vim
}

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn usize(&mut self, upper: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.0 >> 32) as usize) % upper
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Snapshot {
    text: String,
    cursor: (usize, usize),
}

fn run_edtui(initial: &str, keys: &str) -> Snapshot {
    let mut state = EditorState::new(Lines::from(initial));
    let mut handler = EditorEventHandler::vim_mode();
    for key in parse_keys(keys) {
        handler.on_key_event(key, &mut state);
    }
    snapshot(&state)
}

fn trace_edtui(initial: &str, steps: &[&str]) -> Vec<Snapshot> {
    let mut state = EditorState::new(Lines::from(initial));
    let mut handler = EditorEventHandler::vim_mode();
    let mut snapshots = Vec::new();
    for step in steps {
        for key in parse_keys(step) {
            handler.on_key_event(key, &mut state);
        }
        snapshots.push(snapshot(&state));
    }
    snapshots
}

fn snapshot(state: &EditorState) -> Snapshot {
    Snapshot {
        text: editor_text(&state.lines),
        cursor: (state.cursor.row, state.cursor.col),
    }
}

fn editor_text(lines: &Lines) -> String {
    lines
        .iter_row()
        .map(|line| line.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_vim(vim: &str, initial: &str, keys: &str) -> Snapshot {
    run_vim_steps(vim, initial, &[keys])
}

fn run_vim_steps(vim: &str, initial: &str, steps: &[&str]) -> Snapshot {
    run_vim_steps_with_trace(vim, initial, steps)
        .pop()
        .expect("vim should produce at least one snapshot")
}

fn trace_vim_steps(vim: &str, initial: &str, steps: &[&str]) -> Vec<Snapshot> {
    run_vim_steps_with_trace(vim, initial, steps)
}

fn run_vim_steps_with_trace(vim: &str, initial: &str, steps: &[&str]) -> Vec<Snapshot> {
    let dir = tempdir().expect("tempdir");
    let file = dir.path().join("buffer.txt");
    let cursor_file = dir.path().join("cursor.txt");
    let trace_file = dir.path().join("trace.txt");
    let script = dir.path().join("script.vim");
    fs::write(&file, initial).expect("write initial buffer");

    let mut normal_commands = String::new();
    let trace_file_arg = vim_single_quoted_path(&trace_file);
    for step in steps {
        normal_commands.push_str("let before_text = join(getline(1, '$'), '\\n')\n");
        normal_commands.push_str("execute \"normal! ");
        normal_commands.push_str(&vim_normal_execute_arg(step));
        normal_commands.push_str("\"\n");
        normal_commands.push_str(&format!(
            "call writefile([line('.') . ':' . col('.') . ':' . join(getline(1, '$'), '\\n')], {trace_file_arg}, 'a')\n"
        ));
        // Ex commands are otherwise coalesced into a single undo block in Vim's
        // batch mode. Resetting the option to itself forces a new undo block so
        // `u` behaves like it does when these atoms are typed interactively, but
        // only after real text changes; motion/no-op atoms should not consume undo.
        normal_commands.push_str("if before_text !=# join(getline(1, '$'), '\\n') | let &undolevels = &undolevels | endif\n");
    }
    let file_arg = vim_single_quoted_path(&file);
    let cursor_file_arg = vim_single_quoted_path(&cursor_file);
    fs::write(
        &script,
        format!(
            "set nomore\nset nofixendofline\nexecute 'edit ' . {file_arg}\nnormal! gg0\n{normal_commands}call writefile([line('.') . ':' . col('.')], {cursor_file_arg})\nwrite!\nqall!\n"
        ),
    )
    .expect("write vim script");

    let output = Command::new(vim)
        .arg("-Nu")
        .arg("NONE")
        .arg("-n")
        .arg("-es")
        .arg("-S")
        .arg(&script)
        .output()
        .expect("run vim");

    assert!(
        output.status.success(),
        "vim failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let text = fs::read_to_string(file).expect("read vim output");
    let cursor = fs::read_to_string(cursor_file).expect("read vim cursor");
    let (row, col) = cursor
        .trim()
        .split_once(':')
        .expect("vim cursor should be row:col");
    let final_snapshot = Snapshot {
        text,
        // Vim reports 1-based line and byte column. These tests intentionally use ASCII only,
        // so byte column and edtui's character column are equivalent.
        cursor: (
            row.parse::<usize>().expect("vim cursor row") - 1,
            col.parse::<usize>().expect("vim cursor col") - 1,
        ),
    };
    let mut snapshots = parse_vim_trace(&trace_file);
    if snapshots.is_empty() {
        snapshots.push(final_snapshot);
    } else if let Some(last) = snapshots.last_mut() {
        *last = final_snapshot;
    }
    snapshots
}

fn parse_vim_trace(path: &std::path::Path) -> Vec<Snapshot> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let mut fields = line.splitn(3, ':');
            let row = fields
                .next()
                .expect("trace row")
                .parse::<usize>()
                .expect("trace row")
                - 1;
            let col = fields
                .next()
                .expect("trace col")
                .parse::<usize>()
                .expect("trace col")
                - 1;
            let text = fields.next().unwrap_or_default().replace("\\n", "\n");
            Snapshot { text, cursor: (row, col) }
        })
        .collect()
}

fn vim_single_quoted_path(path: &std::path::Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

fn parse_keys(keys: &str) -> Vec<KeyInput> {
    let mut out = Vec::new();
    let mut chars = keys.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut token = String::new();
            for next in chars.by_ref() {
                if next == '>' {
                    break;
                }
                token.push(next);
            }
            match token.as_str() {
                "Esc" => out.push(KeyInput::new(crossterm::event::KeyCode::Esc)),
                "Enter" | "CR" => out.push(KeyInput::new(crossterm::event::KeyCode::Enter)),
                "BS" => out.push(KeyInput::new(crossterm::event::KeyCode::Backspace)),
                "Tab" => out.push(KeyInput::new(crossterm::event::KeyCode::Tab)),
                "C-r" => out.push(KeyInput::ctrl('r')),
                other => panic!("unsupported key token <{other}>")
            }
        } else if ch.is_ascii_uppercase() {
            out.push(KeyInput::shift(ch));
        } else {
            out.push(KeyInput::new(ch));
        }
    }
    out
}

fn vim_normal_execute_arg(keys: &str) -> String {
    let mut out = String::new();
    let mut chars = keys.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '<' => {
                let mut token = String::new();
                for next in chars.by_ref() {
                    if next == '>' {
                        break;
                    }
                    token.push(next);
                }
                match token.as_str() {
                    "Esc" => out.push_str("\\<Esc>"),
                    "Enter" | "CR" => out.push_str("\\<CR>"),
                    "BS" => out.push_str("\\<BS>"),
                    "Tab" => out.push_str("\\<Tab>"),
                    "C-r" => out.push_str("\\<C-r>"),
                    other => panic!("unsupported key token <{other}>")
                }
            }
            _ => out.push(ch),
        }
    }
    out
}
