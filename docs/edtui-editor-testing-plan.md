# edtui Editor Testing Plan

## Purpose

This plan defines editor-specific testing for the vendored `edtui` crate. The goal is to make Vim behavior changes safe without relying primarily on fpy tmux end-to-end tests.

Use this alongside:

- `docs/edtui-vim-refactor-plan.md`
- `AGENTS.md`
- `tests/tmux_e2e.rs`

The principle is:

> Test editor semantics inside `vendor/edtui`; test terminal layout/rendering integration in `fpy` tmux tests.

## Test Layers

### 1. Pure editor semantic tests

Location:

```text
vendor/edtui/src/vim/*
vendor/edtui/src/actions/*
vendor/edtui/src/events/key.rs
```

These should cover Vim command behavior without Ratatui rendering or fpy.

Use these for:

- word motions
- WORD motions
- counts
- operator-pending mode
- text objects
- cursor placement after edits
- undo grouping
- visual selection semantics
- paste/delete/yank/change behavior

Run with:

```bash
cargo test --manifest-path vendor/edtui/Cargo.toml --lib
```

### 2. edtui rendering tests

Location:

```text
vendor/edtui/src/view/*
```

Use these for:

- selection rendering
- empty-line visual selection rendering
- cursor screen coordinate calculation
- wrapping behavior
- line number rendering if we use edtui-native line numbers later
- viewport scrolling

These should render to a `ratatui_core::buffer::Buffer` and assert cells/spans/styles directly.

Run with:

```bash
cargo test --manifest-path vendor/edtui/Cargo.toml --lib view
```

### 3. fpy unit tests

Location:

```text
src/ui/*
src/custom_terminal.rs
src/insert_history/*
```

Use these for fpy-owned behavior:

- prompt gutter formatting
- status labels
- custom terminal diffing
- transcript insertion
- pane geometry
- fpy-specific editor setup

Run with:

```bash
cargo test --lib
```

### 4. fpy tmux e2e tests

Location:

```text
tests/tmux_e2e.rs
scripts/fpy-tmux-repro.sh
```

Use these only when behavior depends on the terminal, fpy prompt integration, or real event flow.

Examples:

- prompt growth near bottom of screen
- stale cells after redraw
- paste through terminal bracketed paste
- Ctrl-D cleanup
- editor behavior that depends on fpy key routing
- visual-mode bugs only visible through actual terminal rendering

Run with:

```bash
cargo test --test tmux_e2e -- --nocapture
```

## Golden Behavioral Reference

Use `prompt_toolkit` as the reference for IPython-like Vi behavior.

Reference files:

```text
/tmp/python-prompt-toolkit/src/prompt_toolkit/key_binding/vi_state.py
/tmp/python-prompt-toolkit/src/prompt_toolkit/key_binding/bindings/vi.py
/tmp/python-prompt-toolkit/src/prompt_toolkit/document.py
```

Especially important reference behavior:

```python
_FIND_WORD_RE = r"([a-zA-Z0-9_]+|[^a-zA-Z0-9_\s]+)"
_FIND_BIG_WORD_RE = r"([^\s]+)"
```

Do not require full Vim parity when prompt_toolkit differs or omits behavior. The target is practical IPython-style Vi mode.

## Test Harness Helpers

Add editor test helpers in vendored `edtui`, preferably under:

```text
vendor/edtui/src/vim/test_utils.rs
```

or inside `#[cfg(test)]` modules if keeping helpers private.

Recommended helpers:

```rust
fn state(text: &str) -> EditorState;
fn normal_state(text: &str, row: usize, col: usize) -> EditorState;
fn insert_state(text: &str, row: usize, col: usize) -> EditorState;
fn visual_state(text: &str, anchor: Index2, cursor: Index2) -> EditorState;
fn press(state: &mut EditorState, keys: &str);
fn press_inputs(state: &mut EditorState, inputs: &[KeyInput]);
fn assert_cursor(state: &EditorState, row: usize, col: usize);
fn assert_text(state: &EditorState, text: &str);
fn assert_mode(state: &EditorState, mode: EditorMode);
```

For key strings, support a small readable syntax if useful:

```text
"w"
"3w"
"daw"
"<Esc>"
"<Enter>"
"<C-r>"
```

If that parser becomes distracting, use `KeyInput` arrays instead.

## Core Test Matrix

### Word classification

Test lowercase word behavior:

```text
test_1234
foo.bar
foo/bar
foo   bar
hello_world.next
```

Cases:

- `b` treats `_` as word char
- `w` treats `_` as word char
- `e` treats `_` as word char
- punctuation groups remain separate for lowercase `w/b/e`
- whitespace is skipped correctly

Example expectations:

```text
text: test_1234
cursor at 8, press b -> cursor at 0
cursor at 0, press e -> cursor at 8
```

### WORD motions

Test uppercase WORD behavior:

```text
foo.bar baz/qux
```

Cases:

- `W` jumps to next non-whitespace group
- `B` jumps to previous non-whitespace group
- `E` jumps to end of current/next non-whitespace group
- punctuation does not split WORDs

### Counts

Test counts for motions:

```text
3w
2b
2e
5h
5l
3j
2k
10G
5gg
2$
```

Test counts for edits:

```text
4x
3dd
```

Important edge cases:

- `0` by itself moves to beginning of line
- `10G` goes to line 10
- counts clamp at buffer boundaries
- invalid count command clears pending count

### Operators

Test operator + motion:

```text
dw
db
de
d$
d0
dG
dgg
cw
c$
yw
y$
```

Test count multiplication:

```text
d2w
2dw
2d3w
```

For each operator test, assert:

- resulting text
- cursor position
- mode
- clipboard/yank content where relevant
- undo restores expected state

### Linewise operators

Test:

```text
dd
3dd
dj
dk
dG
dgg
yy
2yy
cc
```

Edge cases:

- deleting first line
- deleting middle lines
- deleting last line
- deleting all lines leaves one empty row
- cursor row and column are clamped

### Text objects

Test inner/a word objects:

```text
ciw
diw
yiw
caw
daw
yaw
ciW
daW
```

Test quote/bracket objects:

```text
ci"
da"
yi"
ci'
da'
ci(
da(
ci[
da[
ci{
da{
```

Cases:

- cursor inside object
- cursor on delimiter
- nested delimiters if supported
- missing delimiter should be no-op or clear pending state predictably
- whitespace inclusion for `a*` objects

### Visual mode

Test visual character mode:

```text
vwd
vwc
vwy
```

Test visual line mode:

```text
Vd
Vc
Vy
```

Edge cases:

- reversed selections
- selecting empty lines
- deleting selection at end of buffer
- cursor clamp after visual delete/change
- selection cleared after operation

### Insert/normal mode interactions

Test:

```text
i
I
a
A
o
O
<Esc>
```

Cases:

- `o` in empty buffer creates/open line correctly
- `o` below current line
- `O` above current line
- cursor placement after leaving insert mode
- insert mode accepts normal printable characters

### Paste and clipboard

Test:

- paste into empty buffer
- paste linewise yanks
- paste characterwise yanks
- paste over visual selection
- paste with multiline text
- single-line mode paste replaces newlines with spaces

### Undo/redo

Undo/redo tests should verify command-level grouping.

Rules to test:

1. Pure motions create no undo entries.
2. Yank creates no undo entry.
3. A complete mutating Vim command creates exactly one undo entry.
4. Repeated edits capture once for the whole repeat.
5. Operator commands capture once for the whole operator command.
6. Visual delete/change capture once for the whole visual operation.
7. Undo/redo never leaves the cursor out of bounds.

Motion cases that should not affect undo history:

```text
3w then u should not change text
10G then u should not change text
$ then u should not change text
```

Yank cases that should not affect undo history:

```text
yw then u should not change text
yy then u should not change text
```

Mutating commands that should undo in one step:

```text
4x then u restores all 4 chars
3dd then u restores all 3 lines
dw then u restores original text
d2w then u restores original text
2dw then u restores original text
dG then u restores original text
paste then u restores original text
visual delete then u restores original text
```

Redo:

```text
u then <C-r>
```

Implementation guidance for tests:

- If possible, assert that one `u` fully restores the pre-command text for repeated/operator edits.
- Avoid relying only on final text after many undos; that can hide over-capturing.
- Add cursor validity assertions after both undo and redo.

Exact Vim insert-session grouping for change commands can be refined later. For example, ideal Vim-like behavior is:

```text
cwhello<Esc>u restores original text
```

The first operator-pending pass may only guarantee that the delete/change-start portion of `cw`/`ciw` is one undo step. If so, keep that limitation explicit in tests and docs.

## Snapshot-Style Tests

For complex command behavior, prefer compact table tests.

Example shape:

```rust
struct Case {
    name: &'static str,
    input: &'static str,
    cursor: Index2,
    keys: &'static str,
    expected_text: &'static str,
    expected_cursor: Index2,
    expected_mode: EditorMode,
}
```

This makes it easy to add many Vim behavior cases without writing one verbose test per command.

## Property / Invariant Tests

Consider adding lightweight invariant checks after command execution:

- cursor row is always `< lines.len()`
- cursor col is valid for current mode
- empty buffer is represented as one blank row
- selection endpoints are in-bounds or clampable
- undo/redo never leaves invalid cursor

These can be normal tests over a list of scripted key sequences. Full fuzzing is optional.

Example scripted sequences:

```text
"o<Esc>dd"
"3o<Esc>3dd"
"vjjd"
"dG"
"ggdG"
"ciwabc<Esc>u"
```

## When to Add fpy tmux Tests

Add a tmux e2e test when:

- the bug only reproduces in a terminal
- the bug involves prompt height or bottom-pinned layout
- the bug involves stale screen cells
- the bug involves bracketed paste from terminal events
- the bug involves fpy-specific key interception
- the bug involves shell cleanup/exit behavior

Do not add tmux tests for pure editor semantics like `b` over `test_1234`; those belong in vendored `edtui` unit tests.

## Required Commands Before Commit

For changes inside vendored `edtui` only:

```bash
cargo test --manifest-path vendor/edtui/Cargo.toml --lib
cargo test
```

For rendering/layout/editor integration changes:

```bash
cargo test
cargo test --test tmux_e2e -- --nocapture
```

For terminal behavior changes, also run the relevant repro:

```bash
scripts/fpy-tmux-repro.sh vim-open-below
scripts/fpy-tmux-repro.sh paste
scripts/fpy-tmux-repro.sh ctrl-d
```

## First Testing Improvements to Implement

1. Add vendored edtui test helpers for `EditorState` + key input.
2. Add word motion tests based on prompt_toolkit regex behavior.
3. Add cursor invariant assertions to delete/change/visual tests.
4. Add count tests before refactoring count handling.
5. Add operator/text-object table tests before implementing operator-pending mode.

## Success Criteria

The testing setup is successful when:

- most Vim behavior bugs can be reproduced without tmux
- prompt_toolkit comparison cases are encoded as edtui tests
- every vendored Vim change includes focused unit tests
- fpy tmux tests are reserved for integration/terminal behavior
- cursor/range/selection invariants catch regressions early
