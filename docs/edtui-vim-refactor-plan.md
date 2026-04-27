# edtui Vim Refactor Plan

## Goal

Improve the vendored `edtui` Vim behavior enough for `fpy` to feel close to IPython / `prompt_toolkit` Vi mode, without rewriting the rendering, viewport, syntax highlighting, clipboard, or buffer infrastructure from scratch.

This is not a plan to implement full Vim. The target is a practical REPL editor subset:

- accurate common word motions
- general counts for common motions/actions
- operator-pending mode for `d`, `c`, and `y`
- common text objects like `iw`, `aw`, `iW`, `aW`, quotes, and brackets
- predictable cursor placement after edits
- undo grouping that treats one Vim command as one edit

Use `prompt_toolkit` as the behavioral reference where possible, because IPython uses `prompt_toolkit` Vi mode.

Relevant reference files in a local prompt_toolkit checkout:

- `/tmp/python-prompt-toolkit/src/prompt_toolkit/key_binding/vi_state.py`
- `/tmp/python-prompt-toolkit/src/prompt_toolkit/key_binding/bindings/vi.py`
- `/tmp/python-prompt-toolkit/src/prompt_toolkit/document.py`

## Non-goals

Do not attempt full Vim parity initially. Defer:

- Ex command language
- macros
- marks/jumps
- full register semantics
- digraphs
- block visual mode polish
- full dot-repeat fidelity
- plugin-style extensibility

These can be revisited after the common prompt-editing subset is solid.

## Reuse From edtui

Keep and build on:

- `EditorState`
- `EditorView`
- `EditorTheme`
- `Lines`
- rendering / wrapping / viewport code
- syntax highlighting integration
- paste handling
- clipboard abstraction
- undo/redo storage
- insert-mode primitives
- basic buffer mutation helpers

Likely reusable files:

- `vendor/edtui/src/state.rs`
- `vendor/edtui/src/state/*`
- `vendor/edtui/src/view.rs`
- `vendor/edtui/src/view/*`
- `vendor/edtui/src/clipboard.rs`
- `vendor/edtui/src/events/key/input.rs`
- `vendor/edtui/src/events/paste.rs`
- `vendor/edtui/src/actions/insert.rs`
- `vendor/edtui/src/actions/cpaste.rs`
- parts of `vendor/edtui/src/actions/delete.rs`
- parts of `vendor/edtui/src/actions/search.rs`

Likely to refactor heavily:

- `vendor/edtui/src/events/key.rs`
- `vendor/edtui/src/actions/motion.rs`
- `vendor/edtui/src/actions/select.rs`
- parts of `vendor/edtui/src/actions/delete.rs`

## Proposed Module Structure

Add a dedicated internal Vim module under vendored `edtui`:

```text
vendor/edtui/src/vim/
  mod.rs
  state.rs
  command.rs
  parser.rs
  motion.rs
  operator.rs
  text_object.rs
  range.rs
  word.rs
```

Suggested responsibilities:

### `vim/state.rs`

Own pending Vim command state:

```rust
enum PendingOperator {
    Delete,
    Change,
    Yank,
}

struct VimCommandState {
    pending_count: String,
    operator: Option<PendingOperator>,
    operator_count: Option<usize>,
    // Later: register, pending `g`, pending text-object prefix, etc.
}
```

This should replace ad hoc fields like `pending_normal_count`.

### `vim/range.rs`

Define operator/motion result types:

```rust
enum RangeKind {
    Exclusive,
    Inclusive,
    Linewise,
}

struct TextRange {
    start: Index2,
    end: Index2,
    kind: RangeKind,
}
```

This mirrors prompt_toolkit's `TextObject` / `TextObjectType` idea.

### `vim/word.rs`

Centralize word classification and scanning.

Match prompt_toolkit's practical definitions:

```python
_FIND_WORD_RE = r"([a-zA-Z0-9_]+|[^a-zA-Z0-9_\s]+)"
_FIND_BIG_WORD_RE = r"([^\s]+)"
```

Rust behavior:

- word char: ASCII alphanumeric or `_`
- punctuation group: non-word, non-whitespace chars
- WORD: any run of non-whitespace chars

This fixes cases like:

```text
test_1234
```

where `b` should jump to `t`, not `_`.

### `vim/motion.rs`

Implement pure-ish motion computations that return destinations/ranges without directly mutating editor state unless explicitly asked.

Examples:

```rust
fn word_forward(state: &EditorState, count: usize) -> Motion;
fn word_backward(state: &EditorState, count: usize) -> Motion;
fn word_end(state: &EditorState, count: usize) -> Motion;
fn big_word_forward(state: &EditorState, count: usize) -> Motion;
fn big_word_backward(state: &EditorState, count: usize) -> Motion;
fn big_word_end(state: &EditorState, count: usize) -> Motion;
fn line_end(state: &EditorState, count: usize) -> Motion;
fn goto_line(state: &EditorState, line: usize) -> Motion;
```

Existing motion actions can delegate to these helpers.

### `vim/operator.rs`

Implement operator application:

```rust
fn apply_operator(
    state: &mut EditorState,
    operator: PendingOperator,
    range: TextRange,
);
```

Important behavior:

- high-level mutating Vim commands call `state.capture()` exactly once
- low-level range mutation helpers do not call `state.capture()`
- delete and change update clipboard
- change switches to insert mode
- yank does not mutate buffer and does not create an undo entry
- pure motions do not create undo entries
- cursor is clamped after mutation
- linewise ranges delete/yank whole lines

### `vim/text_object.rs`

Implement text objects:

- `iw`
- `aw`
- `iW`
- `aW`
- `i(`/`a(` and `i)`/`a)`
- `i[`/`a[` and `i]`/`a]`
- `i{`/`a{` and `i}`/`a}`
- `i"`/`a"`
- `i'`/`a'`

Reuse existing select/change-inner logic where reasonable, but prefer returning `TextRange` so all operators can share it.

### `vim/parser.rs` or `vim/command.rs`

Translate key input into one of:

```rust
enum VimCommand {
    Pending,
    Motion(MotionCommand),
    Operator(PendingOperator),
    OperatorMotion(PendingOperator, MotionCommand),
    OperatorTextObject(PendingOperator, TextObjectCommand),
    Edit(EditCommand),
    ModeSwitch(EditorMode),
    Unhandled,
}
```

This is where counts are parsed and multiplied.

Prompt_toolkit behavior to copy conceptually:

```python
event._arg = str((vi_state.operator_arg or 1) * (event.arg or 1))
```

So commands like these behave consistently:

```text
2dw
 d2w
2d3w
```

## Implementation Phases

### Phase 0: Baseline Tests

Before large refactors, add focused tests in vendored `edtui` for current and desired Vim behavior.

Test file can remain in existing modules initially, or new tests can live near `vim/*`.

Baseline cases:

```text
b on test_1234 goes to t
w on test_1234 treats it as one word
e on test_1234 goes to 4
w on foo.bar stops at punctuation boundary
W on foo.bar jumps as one WORD
3w repeats motion
2b repeats motion
10G goes to line 10
5gg goes to line 5
3dd deletes three lines
4x deletes four chars
dw deletes word
cw changes word and enters insert mode
yw yanks word without mutation
ciw changes inner word
daw deletes a word plus trailing whitespace
de/d$/dG line/range semantics
cursor clamps after deleting final lines
undo after d2w restores in one step
```

Run:

```bash
cargo test --manifest-path vendor/edtui/Cargo.toml --lib
cargo test
```

For fpy-facing regressions, add or reuse tmux e2e tests only when behavior affects prompt layout, rendering, or terminal cleanup.

### Phase 1: Word Semantics

Fix word classification first.

Tasks:

- treat `_` as a word character
- add WORD scanning support
- add tests for `test_1234`, `foo.bar`, whitespace-separated WORDs
- ensure existing `w`, `b`, `e` still behave reasonably

Expected files:

- `vendor/edtui/src/actions/motion.rs` initially
- later move to `vendor/edtui/src/vim/word.rs`

This phase is low risk and high value.

### Phase 2: General Count Handling

Replace special-case `nG` handling with a general normal/visual count parser.

Support counts for:

```text
h j k l
w b e
W B E
0 ^ _ $
gg G
x dd
```

Behavior notes:

- `0` by itself remains beginning-of-line.
- nonzero digits start a count.
- once a count has started, `0` extends it.
- count applies to the next motion/edit.
- invalid command clears pending count.

Examples:

```text
3w
20l
5j
10G
5gg
4x
3dd
```

This can be implemented before operator-pending mode.

### Phase 3: Motion Result Abstraction

Introduce `Motion` / `TextRange` helpers and refactor existing motions to delegate to them.

Goals:

- navigation can move the cursor using the same motion logic operators will use
- visual mode can update selection from the same motion results
- operator mode can convert motion results to delete/change/yank ranges

Do this incrementally. Do not rewrite every action at once.

Start with:

- `w`
- `b`
- `e`
- `$`
- `gg`
- `G`

Then add:

- `W`
- `B`
- `E`
- vertical motions

### Phase 4: Operator-Pending Mode

Add pending operator state and support:

```text
d
c
y
```

Initial operator + motion targets:

```text
w b e
W B E
0 ^ _ $
gg G
j k
```

Expected commands:

```text
dw
db
de
d$
dG
dgg
cw
c$
yw
y$
d2w
2dw
2d3w
```

Also preserve direct/common commands:

```text
dd
cc
yy
D
C
```

Implementation details:

- `d` enters operator-pending state.
- next motion produces a `TextRange`.
- operator applies to that range.
- `c` deletes range and switches to insert mode.
- `y` copies range and remains in normal mode.
- `Esc` clears pending operator/count state.

### Phase 5: Text Objects

Implement text objects as `TextRange` producers.

Initial set:

```text
iw aw
iW aW
i( a(
i[ a[
i{ a{
i" a"
i' a'
```

Expected commands:

```text
ciw
daw
yi"
ca(
di[
ya{
```

Visual mode should eventually be able to use text objects too, but normal operator support is the first target.

### Phase 6: Cursor Placement and Undo Semantics

Audit and fix cursor placement after:

- `dw` at end of line
- `d$`
- `dd` on last line
- `dG`
- `cw`
- visual delete/change
- deleting all text
- deleting final rows

Undo expectations:

- one complete mutating Vim command = one undo step
- pure motions create no undo entries
- yank creates no undo entry because it does not mutate the buffer
- repeated edits like `4x` and `3dd` capture once for the whole repeat, not once per primitive edit
- operator commands like `dw`, `d2w`, `2dw`, `ciw`, and visual delete/change capture once for the whole operator command
- internal cursor movement used for computing ranges should not create extra undo entries
- low-level buffer mutation helpers should not call `state.capture()`; the high-level command/operator layer owns capture timing
- `cw`/`ciw` insert-session grouping can be refined later, but the delete/change-start portion should still be one undo step

### Phase 7: Visual Mode Unification

Make visual operations reuse the same range/operator machinery where possible.

Targets:

- visual `d`
- visual `c`
- visual `y`
- linewise visual `V`
- cursor clamp after visual operations
- empty-line selection rendering if needed later

This phase should reduce duplicated deletion/yank/change behavior.

### Phase 8: fpy Integration Cleanup

After the vendored Vim layer owns the behavior:

- remove any remaining fpy-side Vim shims
- keep `src/ui/editor.rs` focused on fpy prompt/editor setup
- keep `src/ui/mod.rs` free of editor-core behavior
- update `AGENTS.md` if new conventions emerge

## Compatibility Target

Prefer `prompt_toolkit` over full Vim where behavior differs or where full Vim is too complex.

Important prompt_toolkit concepts to mirror:

- `_` is a word character
- WORD is non-whitespace
- operators and text objects are separate concepts
- operator count and motion count multiply
- text objects have exclusive/inclusive/linewise range kind
- selection type matters for linewise operations

## Risk Areas

### Range Inclusivity

Vim's inclusive/exclusive behavior is subtle. Keep it explicit with `RangeKind` and test every operator/motion combination added.

### Linewise Edits

Linewise deletion/yank/change are common sources of off-by-one errors. Test deleting first, middle, last, and all lines.

### Cursor Clamping

After every buffer mutation, clamp both row and column. We already fixed one cursor-past-buffer bug after deleting selected lines.

### Undo Grouping

Vim commands should be grouped at the user-command level, not at the primitive-action level.

Rules:

1. Pure motions do not call `state.capture()`.
2. Yank does not call `state.capture()`.
3. A complete mutating Vim command calls `state.capture()` exactly once.
4. Repeated edits like `4x` and `3dd` capture once for the whole repeat.
5. Operator commands like `dw`, `d2w`, `2dw`, `ciw`, and visual delete/change capture once for the whole operator command.
6. Low-level helpers such as `delete_range_without_capture`, `insert_text_without_capture`, or `yank_range` should not capture.
7. Insert-session grouping after change commands, e.g. `cwhello<Esc>` undoing as one unit, is desirable but can be implemented after the first operator-pending pass.

Avoid chaining existing actions that each call `state.capture()` when implementing one high-level Vim command. Instead, expose no-capture primitives and let the Vim command/operator layer own undo capture timing.

### Existing fpy Behavior

Run fpy tests frequently. In particular:

```bash
cargo test
cargo test --test tmux_e2e -- --nocapture
```

Use tmux repros for prompt layout/rendering regressions:

```bash
scripts/fpy-tmux-repro.sh vim-open-below
scripts/fpy-tmux-repro.sh paste
scripts/fpy-tmux-repro.sh ctrl-d
```

## Suggested First PR / Commit Stack

1. `Fix Vim word classification in vendored edtui`
   - `_` as word char
   - tests for `test_1234`

2. `Add WORD motions to vendored edtui`
   - `W`, `B`, `E`
   - tests matching prompt_toolkit regex behavior

3. `Generalize Vim count handling`
   - counts for motions and simple edit repeats
   - replace special `nG` code

4. `Introduce Vim motion ranges`
   - `TextRange`, `RangeKind`
   - refactor `w/b/e/$/G/gg` to use it

5. `Add operator-pending delete/change/yank`
   - `dw`, `cw`, `yw`, `d$`, etc.

6. `Add common Vim text objects`
   - `iw`, `aw`, `iW`, `aW`, quote/bracket objects

7. `Unify visual operations with Vim ranges`
   - visual delete/change/yank use the same operator code

## Success Criteria

The refactor is successful when:

- common IPython/prompt_toolkit Vi-mode editing feels familiar
- `src/ui/` contains no editor-core Vim shims
- vendored `edtui` has focused tests for motions/operators/text objects
- fpy tmux e2e tests continue to pass
- adding a new Vim command does not require duplicating parser, range, and mutation logic
