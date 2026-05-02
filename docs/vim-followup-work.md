# Vim Follow-up Work

This document tracks Vim-fidelity work intentionally left out of the first refactor pass.

## Recommended Ordering

### Phase A: Stabilize Change Commands

Do this first because `C` and `s` are common Vim editing primitives and closely related to the operator/change architecture.

#### A1. `s` / counted `s`

Golden cases first:

```text
sX<Esc>       at start/middle/end of line
2sX<Esc>
sX<Esc>       on an empty line
```

Then fuzz atoms:

```rust
"sX<Esc>"
"2sX<Esc>"
```

#### A2. `C`

Golden cases first:

```text
CX<Esc>
wCX<Esc>
$CX<Esc>
0CX<Esc>
C<Enter>X<Esc>    later, if newline/change semantics are stable
```

Then fuzz atom:

```rust
"CX<Esc>"
```

### Phase B: Backward Char Search

Finish char-search symmetry after forward `f` / `t` are stable.

Golden cases first:

```text
$Fa
$Ta
Fzu
Tzu
dFa
dTa
```

Then fuzz atoms:

```rust
"Fa"
"Ta"
```

Reason: adding `F` / `T` exposed no-op/undo interactions, so this should come after change semantics are stable.

### Phase C: Undo/Redo Command Ownership

Move `u` and `<C-r>` out of generic keybindings into Vim command handling.

Golden cases:

```text
xu
x<C-r>
ddu
dd<C-r>
x_u
D_xDu0gg     or a reduced form of this known shape
```

Reason: undo/redo are already in fuzz, but they are still generic actions. Once `s`, `C`, `F`, and `T` add more undo pressure, centralize undo/redo command ownership.

### Phase D: Minimal Paste/Register Behavior

Implement only the minimal register behavior needed for REPL editing.

Golden cases:

```text
xp
ddp
dwp
xpu
ddpu
```

Then fuzz atoms:

```rust
"p"
"ddp"
"xpu"
```

Reason: paste depends on delete/yank text shape and undo, so it should follow undo stabilization.

### Phase E: Dynamic Multiline Construction

Revisit dynamically building Python-like multiline text:

```rust
"idef test_1234(x: int):<Esc>o    foobar<Esc>"
```

Avoid testing Vim batch `normal!` undo quirks. Prefer step-based interactive oracle support:

```text
idef test_1234(x: int):<Esc>
o    foobar<Esc>
u
```

The expected final state should reflect interactive Vim behavior, not one batch `normal!` undo block.

After that is stable, add the construction atom to fuzz without trailing `u`.

Known blocker from attempting to add the construction atom to random fuzz: it changes corpus ordering enough to expose existing undo cursor fidelity gaps around character deletes after prior insert/change sessions. A reduced failing shape is:

```text
initial: "abc def\nxyz"
steps:
  IX<Esc>
  k
  gg
  sX<Esc>
  3e
  W
  2x
  2k
  u

edtui cursor after undo: (1, 2)
vim cursor after undo:   (0, 0)
```

A naive rule that captures character deletes on later rows with cursor `(0, 0)` fixes this shape but breaks dynamic multiline undo after deleting near the opened line. Treat this as an undo metadata/modeling task, not a one-off cursor clamp.

### Phase F: Text Objects

Golden cases:

```text
diw
daw
ciw
caw
diW
daW
di(
da(
di"
da"
```

Then add fuzz atoms gradually, starting with delete-only objects:

```rust
"diw"
"daw"
```

Only add change text objects after `s` / `C` / change semantics are solid.

### Phase G: Search Commands

Optional. Only do this if fpy wants real Vim search behavior in the inline editor.

Golden cases:

```text
/foo<Enter>
n
N
?foo<Enter>
```

This may require expanding the harness to compare search-driven cursor movement rather than highlights.

### Phase H: Terminal Navigation Keys

Decide policy first:

1. Treat arrow/Home/End/Delete as Vim aliases and oracle-test them.
2. Treat them as editor convenience keys and leave them generic.

The current recommendation is option 2 unless users report problems.

### Phase I: Dot-repeat, Macros, and Register Completeness

Defer. Dot-repeat probably requires storing replayable semantic `VimCommand`s, not raw key sequences.

### Near-term Sequence

Recommended immediate sequence:

```text
A1: fix/fuzz s
A2: fix/fuzz C
B:  fix/fuzz F/T
C:  move u/<C-r> into Vim command ownership
D:  minimal p/register behavior
E:  dynamic multiline construction with step-based oracle
F:  text objects
```

This order minimizes churn: change semantics first, then search-motion edge cases, then undo ownership, then paste/registers.

The current supported corpus is passing against real Vim, and common normal-mode motions/deletes now route mostly through `vendor/edtui/src/vim/*`. The items below should be treated as separate oracle-driven increments: add targeted real-Vim cases first, then add fuzz atoms only after semantics are stable.

## 1. Change Commands

### `C`

Current status:
- Still handled by the generic `ChangeToEndOfLine` action path.
- A naive migration to the same Vim operator path as `D` produced cursor/text differences.

Known mismatch from attempted migration:

```text
initial: "abc def"
keys:    "wCX<Esc>"
edtui:   "abcX "
vim:     "abc X"
```

Work needed:
- Model `C` as Vim `c$`, including correct insertion point and normal-mode cursor after `<Esc>`.
- Add targeted golden cases before adding to fuzz.
- Decide whether `C` should use a special range/cursor rule rather than exactly reusing `D`'s range.

### `s` / counted `s`

Current status:
- `s` is handled by existing Vim command code, but it is not in the random fuzz corpus.
- Adding `sX<Esc>` exposed cursor/replacement differences.

Known mismatch from attempted fuzz expansion:

```text
initial: "abc def\nxyz"
keys:    "02esX<Esc>..."
edtui:   "abc dXe\nxyz"
vim:     "abc deX\nxyz"
```

Work needed:
- Clarify Vim replacement range for `s` at end-of-line and after word-end motions.
- Add golden cases for:
  - `sX<Esc>` at start/middle/end of line
  - `2sX<Esc>`
  - `s` on empty line
- Add to fuzz only after targeted cases pass.

## 2. Backward Char Search Motions

### `F` / `T`

Current status:
- Targeted golden cases exist for `$Fa` and `$Ta`.
- `F` / `T` are not in the random fuzz corpus.
- Adding them exposed no-op/undo interactions.

Known failure shape:

```text
keys: "TaddkggggtaDddefaggu"
edtui final: ""
vim final:   "one two three"
```

Work needed:
- Understand whether no-op backward char searches should participate in undo in the current harness model.
- Add smaller golden cases around failed `F`/`T` searches and subsequent `u`.
- Add `Fa` / `Ta` to fuzz only after no-op behavior is stable.

## 3. Paste and Registers

### `p`

Current status:
- Still handled by generic action/keybinding path.
- Not in the real-Vim fuzz corpus.

Work needed:
- Decide minimal register model for REPL editing.
- Verify delete/yank text shape for:
  - characterwise delete then `p`
  - linewise delete then `p`
  - paste after empty-line deletes
- Add golden cases before fuzzing.

Potential atoms:

```rust
"p"
"ddp"
"xpu"
"dwp"
```

## 4. Undo/Redo Command Ownership

### `u` / `<C-r>`

Current status:
- Included in fuzz and passing.
- Still routed via generic keybinding actions.
- Undo transaction ownership has moved toward the Vim executor, but undo/redo commands themselves are not yet semantic `VimCommand`s.

Work needed:
- Move `u` and `<C-r>` dispatch into Vim command handling.
- Preserve current behavior for no-op deletes and undo cursor restoration.
- Re-run high iteration fuzz after any change.

## 5. Search Commands

### `/`, `?`, `n`, `N`

Current status:
- Still handled by generic search actions.
- Not compared against real Vim.

Work needed:
- Decide whether fpy wants Vim-like search fidelity in the inline REPL editor.
- If yes, add a separate oracle group because search state/highlighting may not be comparable with the current final-buffer snapshot alone.
- Consider comparing only final cursor position and buffer text for search motions.

Potential golden cases:

```text
/word<Enter>
n
N
?word<Enter>
```

## 6. Terminal Navigation Keys

### Arrow keys, Home, End, Delete

Current status:
- Normal-mode arrow/Home/End/Delete keys still use generic keybindings.
- The fuzz harness primarily exercises Vim character commands, not terminal-key aliases.

Work needed:
- Decide whether these are Vim commands or editor convenience keys.
- If they should be Vim-compatible, add parser tokens for them and compare against Vim.
- Otherwise leave them as generic terminal-editor conveniences.

Potential test tokens already supported by parser:

```text
<Left>
<Right>
<Up>
<Down>
<Home>
<End>
<Del>
```

Note: `parse_keys` may need additional token support for some of these.

## 7. Dynamic Multiline Construction Atom

Current status:
- The dynamic Python-like construction case is intentionally not in the random fuzz corpus.
- The old `...<Esc>o...<Esc>u` golden was removed because it tested Vim batch `normal!` undo grouping, not interactive behavior.

Candidate atom:

```rust
"idef test_1234(x: int):<Esc>o    foobar<Esc>"
```

Work needed:
- Add step-based interactive oracle support if we want to test undo after this sequence accurately.
- Reintroduce as a fuzz atom only after related insert/open-line undo semantics survive high iteration fuzz.

## 8. Text Objects and Broader Operators

Current status:
- Some text-object infrastructure exists in `vendor/edtui/src/vim/text_object.rs`.
- The main fuzz corpus does not aggressively exercise text objects.

Work needed:
- Add golden cases for:
  - `diw`, `daw`
  - `ciw`, `caw`
  - `diW`, `daW`
  - delimiter objects such as `di(`, `da(`, `di"`
- Add fuzz atoms gradually after targeted cases pass.

## 9. Dot-repeat and Registers

Current status:
- Out of scope for the first refactor.

Work needed:
- Only consider after command execution is fully semantic.
- Dot-repeat probably requires storing a replayable `VimCommand`, not raw keys.

## Suggested Workflow for Each Item

1. Add one or more ignored real-Vim golden cases in `tests/vim_fidelity.rs`.
2. Run:

   ```bash
   FPY_VIM_FUZZ_ITERS=1000 cargo test --test vim_fidelity -- --ignored --nocapture
   ```

3. Fix targeted behavior without adding the atom to random fuzz yet.
4. Add the atom to the fuzz corpus.
5. Re-run 1000+ iterations.
6. If a failure appears, prefer fixing semantics over excluding the atom.
7. Commit each stable increment separately.

## Current Good Baseline

At the time this document was written, these pass:

```bash
FPY_VIM_FUZZ_ITERS=1000 cargo test --test vim_fidelity -- --ignored --nocapture
cargo test --test vim_fidelity --no-run
cargo test
cargo clippy --all-targets --all-features
```

### Known 5000-iteration cursor-only divergence: `2B` across blank indented line

After fixing `s` on an empty buffer after `dd`, a longer 5000-iteration oracle run exposed a remaining cursor-only mismatch:

```text
initial: "alpha beta\ngamma delta"
keys: "fa_ciwX<Esc>02lidef test_1234(x: int):<Esc>o    foobar<Esc>CX<Esc>u$dawE2B"

edtui: Snapshot { text: "X def test_1234(x: int):beta\n    \ngamma delta", cursor: (1, 0) }
vim:   Snapshot { text: "X def test_1234(x: int):beta\n    \ngamma delta", cursor: (0, 19) }
```

The text matches; only the final cursor differs. The reduced shape appears to involve `daw` leaving an indented blank line, followed by `E2B`. Naive attempts to special-case big-word backward across blank/indented lines caused earlier fuzz regressions, so leave this as follow-up motion fidelity work rather than a targeted cursor clamp.
