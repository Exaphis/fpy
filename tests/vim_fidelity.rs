//! Optional golden tests for edtui/fpy Vim-mode fidelity.
//!
//! These tests run the same keystroke script through edtui and through a real
//! Neovim binary, then compare the final buffer/cursor. They are ignored by
//! default because they require `nvim` on PATH and spawn external processes.
//!
//! Run with:
//!   FPY_NVIM=nvim cargo test --test vim_fidelity -- --ignored --nocapture

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use edtui::{
    EditorEventHandler, EditorState, Lines, clipboard::InternalClipboard, events::KeyInput,
};
use tempfile::{TempDir, tempdir};

#[derive(Clone, Copy, Debug)]
struct Case {
    name: &'static str,
    initial: &'static str,
    keys: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct StepCase {
    name: &'static str,
    initial: &'static str,
    steps: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        name: "delete_word",
        initial: "one two three",
        keys: "wdw",
    },
    Case {
        name: "linewise_delete_moves_to_indent",
        initial: "def test_1234(x: int):\n    foobar\ntrailer",
        keys: "dd",
    },
    Case {
        name: "counted_linewise_delete_moves_to_indent",
        initial: "first\nsecond\n    foobar\ntrailer",
        keys: "2dd",
    },
    Case {
        name: "linewise_delete_whole_buffer_leaves_empty_line",
        initial: "only line",
        keys: "dd",
    },
    Case {
        name: "delete_char_atom",
        initial: "abc",
        keys: "x",
    },
    Case {
        name: "delete_char_count_atom",
        initial: "abcde",
        keys: "2x",
    },
    Case {
        name: "delete_to_end_atom",
        initial: "abc def",
        keys: "wD",
    },
    Case {
        name: "find_char_forward",
        initial: "abc abc",
        keys: "fa",
    },
    Case {
        name: "till_char_forward",
        initial: "abc abc",
        keys: "ta",
    },
    Case {
        name: "find_char_backward",
        initial: "abc abc",
        keys: "$Fa",
    },
    Case {
        name: "till_char_backward",
        initial: "abc abc",
        keys: "$Ta",
    },
    Case {
        name: "big_word_forward_counts_empty_line",
        initial: "X\n\nYone two three",
        keys: "2W",
    },
    Case {
        name: "line_end_sets_vertical_preferred_column",
        initial: "abc def\nXxyz",
        keys: "j$2k",
    },
    Case {
        name: "substitute_start",
        initial: "abc def",
        keys: "sX<Esc>",
    },
    Case {
        name: "substitute_middle",
        initial: "abc def",
        keys: "wsX<Esc>",
    },
    Case {
        name: "substitute_end",
        initial: "abc def",
        keys: "$sX<Esc>",
    },
    Case {
        name: "substitute_count",
        initial: "abc def",
        keys: "w2sX<Esc>",
    },
    Case {
        name: "change_to_end",
        initial: "abc def",
        keys: "wCX<Esc>",
    },
    Case {
        name: "paste_after_char_delete",
        initial: "abc",
        keys: "xp",
    },
    Case {
        name: "delete_till_p_does_not_paste",
        initial: "abc pqr",
        keys: "dtp",
    },
    Case {
        name: "change_till_p_does_not_paste",
        initial: "abc pqr",
        keys: "ctX<Esc>",
    },
    Case {
        name: "paste_after_line_delete",
        initial: "abc\ndef",
        keys: "ddp",
    },
    Case {
        name: "paste_after_word_delete",
        initial: "abc def",
        keys: "dwp",
    },
    Case {
        name: "paste_undo_after_char_delete",
        initial: "abc",
        keys: "xpu",
    },
    Case {
        name: "paste_undo_after_line_delete",
        initial: "abc\ndef",
        keys: "ddpu",
    },
    Case {
        name: "delete_inner_word",
        initial: "abc def ghi",
        keys: "wdiw",
    },
    Case {
        name: "delete_around_word",
        initial: "abc def ghi",
        keys: "wdaw",
    },
    Case {
        name: "delete_inner_big_word",
        initial: "abc-def ghi",
        keys: "diW",
    },
    Case {
        name: "delete_around_big_word",
        initial: "abc-def ghi",
        keys: "daW",
    },
    Case {
        name: "change_inner_word",
        initial: "abc def ghi",
        keys: "wciwX<Esc>",
    },
    Case {
        name: "change_around_word",
        initial: "abc def ghi",
        keys: "wcawX<Esc>",
    },
    Case {
        name: "change_inner_big_word",
        initial: "abc-def ghi",
        keys: "ciWX<Esc>",
    },
    Case {
        name: "change_around_big_word",
        initial: "abc-def ghi",
        keys: "caWX<Esc>",
    },
    Case {
        name: "delete_inner_parens",
        initial: "foo(bar baz) qux",
        keys: "fbdi(",
    },
    Case {
        name: "delete_around_parens",
        initial: "foo(bar baz) qux",
        keys: "fbda(",
    },
    Case {
        name: "delete_inner_double_quotes",
        initial: "foo \"bar baz\" qux",
        keys: "fbdi\"",
    },
    Case {
        name: "delete_around_double_quotes",
        initial: "foo \"bar baz\" qux",
        keys: "fbda\"",
    },
    Case {
        name: "delete_inner_single_quotes",
        initial: "foo 'bar baz' qux",
        keys: "fbdi'",
    },
    Case {
        name: "delete_around_single_quotes",
        initial: "foo 'bar baz' qux",
        keys: "fbda'",
    },
    Case {
        name: "delete_inner_brackets",
        initial: "foo[bar baz] qux",
        keys: "fbdi[",
    },
    Case {
        name: "delete_around_brackets",
        initial: "foo[bar baz] qux",
        keys: "fbda[",
    },
    Case {
        name: "change_inner_parens",
        initial: "foo(bar baz) qux",
        keys: "fbci(X<Esc>",
    },
    Case {
        name: "change_around_parens",
        initial: "foo(bar baz) qux",
        keys: "fbca(X<Esc>",
    },
    Case {
        name: "change_inner_double_quotes",
        initial: "foo \"bar baz\" qux",
        keys: "fbci\"X<Esc>",
    },
    Case {
        name: "change_around_double_quotes",
        initial: "foo \"bar baz\" qux",
        keys: "fbca\"X<Esc>",
    },
    Case {
        name: "change_inner_single_quotes",
        initial: "foo 'bar baz' qux",
        keys: "fbci'X<Esc>",
    },
    Case {
        name: "change_around_single_quotes",
        initial: "foo 'bar baz' qux",
        keys: "fbca'X<Esc>",
    },
    Case {
        name: "change_inner_brackets",
        initial: "foo[bar baz] qux",
        keys: "fbci[X<Esc>",
    },
    Case {
        name: "change_around_brackets",
        initial: "foo[bar baz] qux",
        keys: "fbca[X<Esc>",
    },
    Case {
        name: "indent_current_line",
        initial: "abc\ndef",
        keys: ">>",
    },
    Case {
        name: "outdent_current_line",
        initial: "    abc\ndef",
        keys: "<lt><lt>",
    },
    Case {
        name: "indent_down_motion",
        initial: "abc\ndef\nghi",
        keys: ">j",
    },
    Case {
        name: "outdent_down_motion",
        initial: "    abc\n    def\nghi",
        keys: "<lt>j",
    },
    Case {
        name: "gv_reselects_previous_visual_selection",
        initial: "abc def ghi",
        keys: "ve<Esc>gvd",
    },
    Case {
        name: "gv_reselects_previous_visual_line_selection",
        initial: "abc\ndef\nghi",
        keys: "Vj<Esc>gvd",
    },
    Case {
        name: "visual_indent_char_selection",
        initial: "abc\ndef\nghi",
        keys: "vj>",
    },
    Case {
        name: "visual_outdent_char_selection",
        initial: "    abc\n    def\nghi",
        keys: "vj<lt>",
    },
    Case {
        name: "visual_indent_line_selection",
        initial: "abc\ndef\nghi",
        keys: "Vj>",
    },
    Case {
        name: "visual_outdent_line_selection",
        initial: "    abc\n    def\nghi",
        keys: "Vj<lt>",
    },
];

const STEP_CASES: &[StepCase] = &[
    StepCase {
        name: "dynamic_multiline_construction",
        initial: "a b c d e",
        steps: &["idef test_1234(x: int):<Esc>", "o    foobar<Esc>"],
    },
    StepCase {
        name: "dynamic_multiline_construction_undo_open_line",
        initial: "a b c d e",
        steps: &["idef test_1234(x: int):<Esc>", "o    foobar<Esc>", "u"],
    },
    StepCase {
        name: "python_autoindent_open_below_copies_current_indent",
        initial: "    x = 1",
        steps: &["oX<Esc>"],
    },
    StepCase {
        name: "python_autoindent_open_below_after_colon",
        initial: "def foo():",
        steps: &["obar<Esc>"],
    },
    StepCase {
        name: "dynamic_multiline_construction_then_delete_line",
        initial: "a b c d e",
        steps: &["idef test_1234(x: int):<Esc>", "o    foobar<Esc>", "dd"],
    },
    StepCase {
        name: "undo_line_delete_after_char_paste_restores_vim_cursor",
        initial: "a b c d e",
        steps: &[
            "2x", "xp", "dd", "u", "sX<Esc>", "3w", "2dd", "IX<Esc>", "2W",
        ],
    },
    StepCase {
        name: "undo_substitute_after_line_delete_restores_empty_buffer",
        initial: "a b c d e",
        steps: &["G", "Fa", "2sX<Esc>", "<C-r>", "dd", "sX<Esc>", "u"],
    },
    StepCase {
        name: "redo_cursor_after_caw_then_join_noop",
        initial: "a b c d e",
        steps: &[
            "2sX<Esc>",
            "2B",
            "l",
            "sX<Esc>",
            "cawX<Esc>",
            "u",
            "<C-r>",
            "J",
        ],
    },
    StepCase {
        name: "undo_insert_at_start_after_change_restores_cursor",
        initial: "one two three",
        steps: &[
            "aX<Esc>", "W", "E", "E", "diw", "AX<Esc>", "2W", "CX<Esc>", "IX<Esc>", "u",
        ],
    },
    StepCase {
        name: "undo_insert_at_start_after_redo_restores_cursor",
        initial: "one two three",
        steps: &[
            "cawX<Esc>",
            "AX<Esc>",
            "u",
            "IX<Esc>",
            "l",
            "2w",
            "<C-r>",
            "fa",
            "B",
            "u",
        ],
    },
];

#[test]
#[ignore = "requires a real Neovim binary; run with FPY_NVIM=nvim cargo test --test vim_fidelity -- --ignored"]
fn edtui_matches_real_neovim_for_golden_cases() {
    let nvim = nvim_binary();
    let mut oracle = NeovimOracle::start(&nvim);

    for case in CASES {
        let edtui = run_edtui_steps(case.initial, &[case.keys]);
        let neovim = oracle.run(case.initial, &[case.keys]);
        assert_eq!(
            edtui, neovim,
            "case {:?} with keys {:?}",
            case.name, case.keys
        );
    }

    for case in STEP_CASES {
        let edtui = run_edtui_steps(case.initial, case.steps);
        let neovim = oracle.run(case.initial, case.steps);
        assert_eq!(
            edtui, neovim,
            "step case {:?} with steps {:?}",
            case.name, case.steps
        );
    }
}

#[test]
#[ignore = "requires a real Neovim binary; run with FPY_NVIM=nvim FPY_VIM_FUZZ_ITERS=1000 cargo test --test vim_fidelity -- --ignored"]
fn fuzz_supported_vim_normal_mode_sequences_against_real_neovim() {
    let nvim = nvim_binary();
    let mut oracle = NeovimOracle::start(&nvim);
    let iterations = std::env::var("FPY_VIM_FUZZ_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1000);
    let mut rng = Lcg::new(
        std::env::var("FPY_VIM_FUZZ_SEED")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0xf9_f1_de_11_7e),
    );

    // Keep this corpus to commands we intentionally support and have not already
    // documented as divergences. Add commands here as edtui's Vim fidelity grows.
    let single_line_atoms = [
        "h",
        "j",
        "k",
        "l",
        "w",
        "b",
        "e",
        "W",
        "B",
        "E",
        "0",
        "_",
        "$",
        "gg",
        "G",
        "x",
        "dd",
        "dw",
        "D",
        "sX<Esc>",
        "CX<Esc>",
        "J",
        "fa",
        "ta",
        "Fa",
        "Ta",
        "iX<Esc>",
        "IX<Esc>",
        "aX<Esc>",
        "AX<Esc>",
        "OX<Esc>",
        "idef test_1234(x: int):<Esc>o    foobar<Esc>",
        "iX<Enter>Y<Esc>",
        "aX<Enter>Y<Esc>",
        "2dd",
        "2h",
        "2l",
        "2w",
        "2b",
        "2e",
        "2W",
        "2B",
        "2E",
        "2x",
        "2sX<Esc>",
        "xp",
        "diw",
        "daw",
        "ciwX<Esc>",
        "cawX<Esc>",
        "3w",
        "3b",
        "3e",
        "u",
        "<C-r>",
        "dtp",
    ];
    let multiline_atoms = [
        "h",
        "j",
        "k",
        "l",
        "w",
        "b",
        "e",
        "W",
        "B",
        "E",
        "0",
        "_",
        "$",
        "gg",
        "G",
        "x",
        "dd",
        "dw",
        "D",
        "sX<Esc>",
        "CX<Esc>",
        "J",
        "fa",
        "ta",
        "Fa",
        "Ta",
        "iX<Esc>",
        "IX<Esc>",
        "aX<Esc>",
        "AX<Esc>",
        "OX<Esc>",
        "idef test_1234(x: int):<Esc>o    foobar<Esc>",
        "iX<Enter>Y<Esc>",
        "aX<Enter>Y<Esc>",
        "2dd",
        "2h",
        "2l",
        "2w",
        "2b",
        "2e",
        "2W",
        "2B",
        "2E",
        "2x",
        "2sX<Esc>",
        "xp",
        "diw",
        "daw",
        "ciwX<Esc>",
        "cawX<Esc>",
        "3w",
        "3b",
        "3e",
        "2j",
        "2k",
        "u",
        "<C-r>",
        "dtp",
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

        let edtui = run_edtui_steps(initial, &steps);
        let nvim_snapshot = oracle.run(initial, &steps);
        if edtui != nvim_snapshot {
            panic!(
                "fuzz iteration {iteration}, initial {initial:?}, keys {keys:?}\nedtui: {edtui:?}\nneovim: {nvim_snapshot:?}\nedtui trace: {:?}\nneovim trace:   {:?}",
                trace_edtui(initial, &steps),
                oracle.trace(initial, &steps),
            );
        }
    }
}

fn nvim_binary() -> String {
    let nvim = std::env::var("FPY_NVIM").unwrap_or_else(|_| "nvim".to_string());
    assert!(
        Command::new(&nvim).arg("--version").output().is_ok(),
        "Neovim binary not found: {nvim:?}"
    );
    nvim
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

fn run_edtui_steps(initial: &str, steps: &[&str]) -> Snapshot {
    let mut state = EditorState::new(Lines::from(initial));
    state.set_clipboard(InternalClipboard::default());
    let mut handler = EditorEventHandler::vim_mode();
    for step in steps {
        state.begin_undo_transaction();
        for key in parse_keys(step) {
            handler.on_key_event(key, &mut state);
        }
        state.end_undo_transaction();
    }
    snapshot(&state)
}

fn trace_edtui(initial: &str, steps: &[&str]) -> Vec<Snapshot> {
    let mut state = EditorState::new(Lines::from(initial));
    state.set_clipboard(InternalClipboard::default());
    let mut handler = EditorEventHandler::vim_mode();
    let mut snapshots = Vec::new();
    for step in steps {
        state.begin_undo_transaction();
        for key in parse_keys(step) {
            handler.on_key_event(key, &mut state);
        }
        state.end_undo_transaction();
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

struct NeovimOracle {
    _dir: TempDir,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl NeovimOracle {
    fn start(nvim: &str) -> Self {
        let dir = tempdir().expect("neovim oracle tempdir");
        let script = dir.path().join("oracle.lua");
        fs::write(&script, NEOVIM_ORACLE_LUA).expect("write neovim oracle lua");
        let mut child = Command::new(nvim)
            .arg("--headless")
            .arg("-u")
            .arg("NONE")
            .arg("-n")
            .arg("-l")
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start neovim oracle");
        let stdin = child.stdin.take().expect("neovim oracle stdin");
        let stdout = BufReader::new(child.stdout.take().expect("neovim oracle stdout"));
        Self {
            _dir: dir,
            child,
            stdin,
            stdout,
        }
    }

    fn run(&mut self, initial: &str, steps: &[&str]) -> Snapshot {
        self.request(false, initial, steps)
            .pop()
            .expect("neovim oracle should produce a snapshot")
    }

    fn trace(&mut self, initial: &str, steps: &[&str]) -> Vec<Snapshot> {
        self.request(true, initial, steps)
    }

    fn request(&mut self, trace: bool, initial: &str, steps: &[&str]) -> Vec<Snapshot> {
        write!(
            self.stdin,
            "RUN\t{}\t{}",
            usize::from(trace),
            hex_encode(initial)
        )
        .expect("write oracle request");
        for step in steps {
            write!(
                self.stdin,
                "\t{}",
                hex_encode(&neovim_normal_execute_arg(step))
            )
            .expect("write oracle step");
        }
        writeln!(self.stdin).expect("finish oracle request");
        self.stdin.flush().expect("flush oracle request");

        let mut snapshots = Vec::new();
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .expect("read oracle response");
            assert!(n != 0, "neovim oracle exited unexpectedly");
            let line = line.trim_end();
            if line == "END" {
                break;
            }
            let mut parts = line.splitn(3, '\t');
            let row = parts
                .next()
                .expect("oracle row")
                .parse::<usize>()
                .expect("oracle row")
                - 1;
            let col = parts
                .next()
                .expect("oracle col")
                .parse::<usize>()
                .expect("oracle col")
                - 1;
            let text = hex_decode(parts.next().unwrap_or_default());
            snapshots.push(Snapshot {
                text,
                cursor: (row, col),
            });
        }
        snapshots
    }
}

impl Drop for NeovimOracle {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "QUIT");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const NEOVIM_ORACLE_LUA: &str = r#"
local function hex_decode(s)
  return (s:gsub('..', function(cc) return string.char(tonumber(cc, 16)) end))
end
local function hex_encode(s)
  return (s:gsub('.', function(c) return string.format('%02x', string.byte(c)) end))
end
local function split_lines(s)
  local lines = {}
  local start = 1
  while true do
    local i = string.find(s, '\n', start, true)
    if not i then
      table.insert(lines, string.sub(s, start))
      break
    end
    table.insert(lines, string.sub(s, start, i - 1))
    start = i + 1
  end
  if #lines == 0 then lines = {''} end
  return lines
end
local function snapshot()
  local pos = vim.api.nvim_win_get_cursor(0)
  local text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, true), '\n')
  io.stdout:write(pos[1] .. '\t' .. pos[2] + 1 .. '\t' .. hex_encode(text) .. '\n')
  io.stdout:flush()
end
vim.o.more = false
vim.o.fixendofline = false
vim.o.expandtab = true
vim.o.shiftwidth = 4
vim.o.tabstop = 4
vim.o.softtabstop = 4
vim.o.autoindent = true
vim.o.smartindent = false
vim.o.cindent = false
vim.o.indentexpr = ''
vim.cmd('filetype plugin on')
vim.cmd('filetype indent on')
for line in io.lines() do
  if line == 'QUIT' then break end
  local fields = {}
  for field in string.gmatch(line, '[^\t]+') do table.insert(fields, field) end
  if fields[1] == 'RUN' then
    local trace = fields[2] == '1'
    vim.cmd('enew!')
    vim.bo.filetype = 'python'
    local old_undolevels = vim.o.undolevels
    vim.o.undolevels = -1
    vim.api.nvim_buf_set_lines(0, 0, -1, true, split_lines(hex_decode(fields[3] or '')))
    vim.cmd('silent! undojoin')
    vim.o.undolevels = old_undolevels
    vim.cmd('normal! gg0')
    for i = 4, #fields do
      local before = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, true), '\n')
      vim.cmd('execute "normal! ' .. hex_decode(fields[i]) .. '"')
      if trace then snapshot() end
      local after = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, true), '\n')
      if before ~= after then vim.cmd('let &undolevels = &undolevels') end
    end
    if not trace then snapshot() end
    io.stdout:write('END\n')
    io.stdout:flush()
  end
end
"#;

fn hex_encode(s: &str) -> String {
    s.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let hex = std::str::from_utf8(chunk).expect("hex utf8");
        out.push(u8::from_str_radix(hex, 16).expect("hex byte"));
    }
    String::from_utf8(out).expect("hex decoded utf8")
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
                "A-r" => out.push(KeyInput::alt('r')),
                "lt" => out.push(KeyInput::new('<')),
                other => panic!("unsupported key token <{other}>"),
            }
        } else if ch.is_ascii_uppercase() {
            out.push(KeyInput::shift(ch));
        } else {
            out.push(KeyInput::new(ch));
        }
    }
    out
}

fn neovim_normal_execute_arg(keys: &str) -> String {
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
                    "A-r" => out.push_str("\\<C-r>"),
                    "lt" => out.push('<'),
                    other => panic!("unsupported key token <{other}>"),
                }
            }
            _ => out.push(ch),
        }
    }
    out
}
