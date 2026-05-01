# Vim Integration Refactor Plan

## Motivation

The real-Vim fidelity harness has found repeated divergences that are not isolated bugs. They mostly come from the current architecture:

```text
KeyInput stream
  -> keybinding lookup
  -> generic Action / Composed action chain
  -> scattered capture, clamp, motion, delete, and mode side effects
```

That model is convenient for simple keybindings, but Vim semantics are command-oriented. Undo boundaries, cursor placement, operator/motion behavior, insert sessions, and no-op edits all depend on the Vim command being executed, not just on individual low-level actions.

The goal is not full Vim. The goal is a maintainable Vim subset that feels correct for editing Python snippets in `fpy`, with real Vim used as the oracle for supported behavior.

## Scope

Target common REPL-editing Vim behavior:

- normal-mode motions: `h`, `j`, `k`, `l`, `w`, `b`, `e`, `W`, `B`, `E`, `0`, `_`, `$`, `gg`, `G`
- common edits: `x`, `dd`, `dw`, `D`, `J`
- counts for supported motions/edits
- insert/append/open-line commands: `i`, `I`, `a`, `A`, `o`, `O`
- insert-mode newline inside insert sessions
- undo/redo: `u`, `<C-r>`
- predictable cursor placement after linewise and characterwise edits
- deterministic behavior for empty lines and trailing empty rows

Explicit non-goals for now:

- full Ex commands
- macros
- registers beyond minimal yank/delete storage
- dot-repeat fidelity
- marks/jumps
- visual/block mode polish
- full text-object coverage
- plugin compatibility

## Key Design Change

Introduce a Vim-specific command layer between key input and editor mutation:

```text
KeyInput stream
  -> Vim parser / command builder
  -> VimCommand
  -> VimCommandExecutor
  -> buffer mutation + cursor result + undo transaction
```

The current generic actions can remain as low-level primitives, but Vim behavior should not depend on incidental side effects of generic action chains.

## Proposed Modules

Add or consolidate under `vendor/edtui/src/vim/`:

```text
vendor/edtui/src/vim/
  command.rs       // semantic VimCommand enum
  executor.rs      // applies VimCommand to EditorState
  parser.rs        // turns KeyInput into complete/pending commands
  transaction.rs   // undo transaction helpers / command boundaries
  motion.rs        // motion resolution to positions/ranges
  operator.rs      // delete/change/yank range application
  text_object.rs   // future text objects
  word.rs          // shared word/big-word classification helpers
```

Existing `vim/motion.rs`, `vim/operator.rs`, and `vim/range.rs` can be migrated rather than replaced immediately.

## Core Types

### `VimCommand`

Example shape:

```rust
enum VimCommand {
    Motion(MotionCommand),
    InsertSession(InsertSession),
    OpenLine { direction: OpenLineDirection, text: Vec<KeyInput> },
    OperatorMotion { operator: Operator, motion: MotionCommand, count: usize },
    LinewiseDelete { count: usize },
    DeleteToEndOfLine,
    DeleteChar { count: usize },
    JoinLines { count: usize },
    Undo,
    Redo,
    Noop,
}
```

Important: commands should represent Vim semantics, not UI keybinding implementation details.

### `CommandResult`

Each command should explicitly report cursor intent:

```rust
struct CommandResult {
    cursor: CursorResult,
    changed_text: bool,
    undo_boundary: UndoBoundary,
}

enum CursorResult {
    Keep,
    MoveTo(Index2),
    FirstNonWhitespaceOnRow(usize),
    ClampNormalMode,
    ClampInsertMode,
}

enum UndoBoundary {
    None,
    SingleCommand,
    InsertSession,
}
```

This avoids scattered `clamp_cursor()` calls determining final behavior by accident.

### `UndoTransaction`

Undo grouping should be owned by the Vim executor. The public `EditorState::begin_undo_transaction()` / `end_undo_transaction()` added for the harness can become the primitive used internally by the executor.

Rules to model:

- one supported fuzz atom should correspond to one Vim undo block when it changes text
- one insert session should undo as one edit
- `o` / `O` plus inserted text should undo as one edit
- no-op motions should not consume undo
- no-op edits should match Vim case-by-case where relevant
- redo history should clear on new text changes

## Parser Strategy

Do not try to parse all Vim upfront. Build an incremental parser for the supported subset.

Initial parser states:

```rust
enum ParserState {
    Normal,
    PendingCount { digits: String },
    PendingOperator { operator: Operator, count: Option<usize> },
    PendingG { count: Option<usize> },
    Insert { origin: InsertOrigin, keys: Vec<KeyInput> },
}
```

The parser emits:

```rust
enum ParseResult {
    Pending,
    Command(VimCommand),
    Unhandled,
}
```

Unhandled keys can initially fall back to existing keybindings, but the goal should be to migrate supported Vim behavior away from fallback paths.

## Refactor Phases

### Phase 1: Put Transactions Behind the Vim Executor

- Add `vim/executor.rs`.
- Move harness transaction calls into normal Vim command execution where possible.
- Keep the harness step transaction as a test-only oracle alignment tool until runtime executor coverage is complete.
- Ensure insert sessions and `o` / `O` use one transaction.

Acceptance:

```bash
FPY_VIM_FUZZ_ITERS=1000 cargo test --test vim_fidelity -- --ignored --nocapture
cargo test
cargo clippy --all-targets --all-features
```

### Phase 2: Migrate Insert/Open-Line Commands

Move these commands out of generic action-chain semantics:

- `i...<Esc>`
- `I...<Esc>`
- `a...<Esc>`
- `A...<Esc>`
- `o...<Esc>`
- `O...<Esc>`
- insert-mode `<Enter>`

Executor responsibilities:

- capture undo at the command start
- apply text insertions/newlines
- place cursor according to Vim normal-mode exit rules
- preserve indentation/text exactly as typed

Then add the dynamic Python case to the random atom corpus only after it survives fuzzing:

```rust
"idef test_1234(x: int):<Esc>o    foobar<Esc>"
```

### Phase 3: Migrate Linewise Edits

Move linewise edit behavior into Vim operator execution:

- `dd`
- counted `dd`
- linewise portions of `d` motions
- cursor placement after deleting lines
- undo cursor restoration for whole-buffer vs partial-buffer deletes

Acceptance cases should include indented replacement lines, e.g. deleting a line above:

```text
def test_1234(x: int):
    foobar
```

Vim usually places the cursor at the first non-whitespace character on the replacement line after `dd`.

### Phase 4: Migrate Characterwise Operators

Move these into the Vim executor/operator layer:

- `x`
- `dw`
- `D`
- `d{motion}` for supported motions

Special cases to preserve:

- `D` on the last non-empty line vs empty line
- `dw` at end of line
- `dw` across empty lines
- deleting the last word while preserving/removing trailing whitespace like Vim

### Phase 5: Consolidate Motions

Unify motion logic so the same motion implementation is used by:

- standalone motion commands
- operator-pending motions
- counted motions

Focus on known fragile areas:

- `w` crossing line boundaries
- `W`/`E` around empty lines
- leading whitespace on indented lines
- trailing empty rows
- preferred column for `j`/`k`

### Phase 6: Remove Vim Semantics from Generic Actions

After executor migration, generic actions should become low-level editor primitives. They should not decide Vim-specific behavior such as:

- undo grouping
- normal-mode cursor clamping
- first-non-whitespace placement
- linewise delete semantics

This reduces regressions where fixing one Vim command changes another unrelated generic action path.

## Testing Strategy

Keep the existing ignored real-Vim integration tests:

```bash
FPY_VIM=vim FPY_VIM_FUZZ_ITERS=1000 cargo test --test vim_fidelity -- --ignored --nocapture
```

Use them as the primary oracle for supported atoms.

For every newly supported atom:

1. Add one or more targeted golden cases.
2. Add the atom to the fuzz corpus.
3. Run at least 1000 deterministic fuzz iterations.
4. If it fails, prefer fixing semantics over excluding the atom.
5. Only exclude if the exposed behavior is outside the current scope and document why.

Useful environment knobs:

```bash
FPY_VIM=vim
FPY_VIM_FUZZ_ITERS=1000
FPY_VIM_FUZZ_SEED=...
```

Always run after changes:

```bash
cargo test --test vim_fidelity --no-run
cargo test
cargo clippy --all-targets --all-features
```

For prompt/editor UI regressions, also run tmux e2e tests from `AGENTS.md` as appropriate.

## Current Known Pressure Points

These should be treated as high-priority migration targets:

- dynamically built multiline buffers using `o` / `O`
- undo after composite insert/open-line sequences
- `dd` cursor placement onto indented lines
- `E` / `W` around empty lines
- `D` and `dw` near empty/trailing lines
- interaction of redo with subsequent insert/open-line edits

## Success Criteria

The refactor is succeeding when:

- supported Vim behavior is implemented primarily in `vim/*`, not scattered through generic actions
- adding a fuzz atom usually requires local changes in parser/executor/motion/operator modules
- undo grouping is command-based and no longer patched per action
- the dynamic Python construction atom can live in the random fuzz corpus
- the full test suite and 1000+ Vim-fidelity fuzz iterations pass consistently

## Practical Rule

When a fuzz failure appears, ask:

> Is this a generic editor primitive bug, or is this Vim command semantics leaking into the wrong layer?

Prefer moving Vim-specific behavior into the Vim command executor over adding another local workaround in `actions/*`.
