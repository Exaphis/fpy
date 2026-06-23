# Pi-style normal-terminal renderer plan

## Summary

Replace `fpy`'s current hybrid normal-terminal backend with a Pi-style renderer:

- render the entire canonical display state into one logical line buffer
- composite overlays into that same buffer
- diff the new logical buffer against the previous one
- scroll only when appending beyond the visible viewport
- fall back to full projection reset/redraw when a differential update cannot be applied safely

This is a deliberate behavior change. The terminal scrollback becomes a projection of `fpy`'s canonical state rather than something the backend tries to maintain as a mostly append-only transcript while separately repainting live UI rows.

`fpy` already owns canonical transcript/display state in `DisplayModel`, so this approach is feasible and should be much more robust than continuing to patch the current live-row/transcript-row hybrid backend.

## Motivation

The current backend tries to preserve committed transcript append semantics while repainting editor/footer/overlay rows in-place below a startup `origin_y`. This creates a hard distinction between:

- committed transcript rows, which are appended to terminal scrollback
- live UI rows, which are repainted in-place

That split is fragile when the live UI is near the bottom of the terminal.

Known failures include:

1. Starting `fpy` near the last line and opening history search may show only the bottom preview/footer, omitting the header/query/result list.
2. Starting near the last line and printing multiple stdout rows can lose committed output rows.
3. Printing while already bottom-pinned can lose the committed input cell line.
4. Opening history search at the bottom, closing it, then printing can produce stale/duplicated output below the footer or lose the input line/output prefix.

The root problem is that live viewport reservation can create real terminal scrollback rows that are not committed transcript rows. Later transcript appends may mistake that terminal state for clean appendable transcript state.

Pi's TUI avoids this class of bugs by treating the UI as one logical line buffer with a viewport and using full redraw when differential rendering is unsafe.

## Reference: what Pi does

Pi's implementation lives in:

```text
/Users/kevin/n/lib/node_modules/@earendil-works/pi-coding-agent/node_modules/@earendil-works/pi-tui/dist/tui.js
```

Relevant behavior:

- `newLines = render(width)` renders the base UI.
- `compositeOverlays(newLines, width, height)` composites overlays into the same logical line buffer.
- `applyLineResets(newLines)` appends resets to every rendered line.
- The renderer tracks:
  - `previousLines`
  - `previousViewportTop`
  - `cursorRow`
  - `hardwareCursorRow`
  - terminal width/height
- If width changes, height changes, or the first changed row is above the previous viewport, it calls `fullRender(true)`.
- `fullRender(true)` clears screen/scrollback and redraws the logical buffer.
- If appending beyond the viewport bottom, it emits exactly enough `\r\n` rows to scroll the terminal viewport.

Important pi safety rule:

```js
if (firstChanged < prevViewportTop) {
  fullRender(true);
  return;
}
```

Differential rendering only touches rows that are actually addressable in the current visible viewport. Otherwise it recovers with a full redraw.

## Desired `fpy` behavior after this change

### Canonical state

`fpy` continues to own canonical state in `DisplayModel` and related transcript structures. The terminal is only a projection/cache.

### Rendering model

Each frame should produce one logical line buffer:

```rust
Vec<RenderedTerminalLine>
```

A line should contain:

- rendered text, including ANSI styling
- optional cursor marker/position metadata, or cursor information resolved separately

Rows can still keep `RowKind` internally if useful for tests or policies, but the backend should not use row kind to maintain separate transcript append vs live UI repaint paths.

### Overlays

Overlays should be composited into the same logical buffer before diffing, not treated as a separate live viewport reservation.

There are two acceptable first-pass approaches:

1. Current simple stacked layout:
   - transcript rows
   - overlay or editor rows
   - footer rows

2. More pi-like overlay compositing:
   - render base transcript/editor/footer
   - overlay draws over a region of the visible viewport

The first pass can keep the current stacked layout if that is simpler. The important behavior change is that the backend gets one final logical buffer and diffs it as a whole. Confirm during implementation that `TerminalFrame.full_rows` already contains the final overlay/editor/footer projection; if any overlay behavior currently lives in the backend, move or bypass it so the pi-style backend sees only final rows.

### Scrollback/projection policy

The normal terminal scrollback is allowed to be rewritten/cleared during full redraw/recovery. This is an explicit behavior change from the previous hybrid design.

Do not try to preserve append-only transcript scrollback semantics in the new backend. Prefer correctness of the projection from canonical state.

## Decisions for the first implementation

These decisions are settled and should guide implementation:

1. **First render preserves shell context.** Use a first full render with `clear = false`, matching pi. If `fpy` is launched near the bottom, writing the initial projection may naturally scroll the terminal; that is acceptable.
2. **Unsafe recovery uses full clear including scrollback.** On unsafe diff/recovery/resize cases, use pi-style full projection reset (`CSI 2J`, home, `CSI 3J`) and rerender from canonical `fpy` state.
3. **No app-level scroll position.** The backend renders the tail/bottom of the canonical logical buffer on each draw. Users may still use terminal-native scrollback, but the backend does not track or preserve a user-scrolled position.
4. **Shrink handling should follow pi, not always full redraw.** Shrinking is extremely common (closing overlays, editor shrinking, footer/status shortening), so safe visible shrinks should be handled differentially by clearing deleted rows. Full redraw is only for unsafe shrink cases, such as deleted/changed rows above the previous viewport.
5. **Clear each repainted row before printing.** Match pi: repaint uses `Clear(CurrentLine)` / `CSI 2K` before writing the new row to avoid stale trailing cells.
6. **Exit leaves projection visible.** On shutdown, do not clear screen/scrollback. Reset styles, show cursor, and move the cursor below the current visible projection so the shell prompt appears after `fpy`.
7. **Rows are already width-shaped; backend owns final line reset.** The backend assumes `TerminalFrame.full_rows` are final terminal rows that fit `frame.size.width`. Components/display rendering own wrapping/truncation. The backend should assert/log on over-wide rows, matching pi's component contract, because unexpected terminal wrapping invalidates logical cursor-row tracking. The backend should always append `CSI 0m` after every printed row, even if renderers also reset, to prevent style bleed during partial repaint and after exit. Store/compare canonical raw row text only; do not store backend-added resets in `previous_lines`.
8. **Use synchronized output.** Wrap render writes with `CSI ? 2026 h` / `CSI ? 2026 l`. Build one full `String`/`Vec<u8>` containing enable + body + disable before writing so the disable sequence is always present in successful writes. Do not stream partial synchronized writes unless the disable sequence is guaranteed. Do fallible computation before adding the sync-enable sequence to the output buffer.
9. **Implement alongside the old backend first.** Add a `PiStyleMainScreenBackend` and switch via env var or temporary selection mechanism before deleting old hybrid logic.
10. **Differential rendering should be close to pi.** Include append scrolling, safe shrink clearing, unsafe-diff full redraw, row clearing, bottom viewport projection, relative cursor movement from tracked logical cursor state, and synchronized output in the first implementation.

## Implementation plan

### 1. Add a new backend or rewrite `CrosstermMainScreenBackend`

Recommended: add a new backend implementation first, e.g.

```rust
PiStyleMainScreenBackend<W: Write>
```

Select it with a temporary environment variable while validating behavior, e.g.:

```text
FPY_RENDERER=pi      # new backend
FPY_RENDERER=hybrid  # old backend
```

The exact name can change, but define one explicit switch and document/log which backend is active. Once tests pass, make the pi-style backend the default and delete the old backend later.

This reduces risk while implementing.

### 2. Backend state

The new backend should track:

```rust
struct PiStyleMainScreenBackend<W: Write> {
    writer: W,
    size: Size,
    previous_lines: Vec<String>,
    previous_width: u16,
    previous_height: u16,
    previous_viewport_top: usize,
    /// Logical row index where the terminal cursor is believed to be after the last render.
    /// This is required for pi-style relative cursor movement after a clear=false first render,
    /// because the physical screen row where the projection started is intentionally unknown.
    hardware_cursor_row: usize,
}
```

Optional/debug fields:

```rust
full_redraw_count: usize,
last_update_kind: Option<PiStyleUpdateKind>,
```

Do not track `origin_y`, `previous_committed_rows`, `previous_live_row_count`, or append-safety state in the new backend. Those are artifacts of the current hybrid model.

### 3. Frame input

The backend can continue accepting `TerminalFrame` initially:

```rust
fn draw_frame(&mut self, frame: TerminalFrame) -> io::Result<()>;
```

But it should ignore `RowKind` for rendering and use:

```rust
let new_lines: Vec<String> = frame.full_rows.iter().map(|row| row.text.clone()).collect();
```

Cursor can use `frame.cursor`.

### 4. Viewport calculation

Use terminal height:

```rust
let height = frame.size.height as usize;
let new_len = new_lines.len();
let viewport_top = new_len.saturating_sub(height);
```

The visible viewport is:

```rust
new_lines[viewport_top..]
```

If terminal height or width is reported as zero, return without writing and without mutating previous render state. Normal terminals should not report zero-sized panes, but the implementation should avoid underflow/panic in tiny-terminal cases.

For the first implementation, maintain a bottom-pinned viewport at all times. Later, if `fpy` wants scrollback navigation or user scroll detection, that can be added separately.

### 5. Full render

Implement:

```rust
fn full_render(&mut self, new_lines: &[String], clear: bool) -> io::Result<()>;
```

Suggested behavior:

- Begin synchronized output: `CSI ? 2026 h`
- If `clear`:
  - hide cursor
  - move to home
  - clear screen
  - clear scrollback (`CSI 3J`) if accepted for this behavior change
- Write the **entire logical buffer** separated by `\r\n`, not only the visible viewport. This reconstructs the terminal projection from canonical `fpy` state. Join rows with `\r\n` **between** rows only; do not emit a trailing `\r\n`, so the terminal cursor ends on the last logical row rather than a blank row after the projection.
- End synchronized output
- Update backend state:
  - `previous_lines = new_lines.to_vec()`
  - `previous_viewport_top = new_lines.len().saturating_sub(height)`
  - `hardware_cursor_row = new_lines.len().saturating_sub(1)` before any explicit cursor positioning; if `draw_cursor` moves the hardware cursor, update this to the logical row corresponding to that position. Maintain a valid `hardware_cursor_row` even when the visible cursor is hidden, so later relative movement and exit cleanup still have a known logical row.

First render is decided: use `full_render(clear = false)` to preserve shell context above `fpy`, matching pi. If `fpy` is launched near the bottom, writing the full initial buffer may naturally scroll the terminal.

Unsafe differential/recovery/resize renders use `full_render(clear = true)`, including screen clear, home, and scrollback clear. This is intentionally destructive to terminal scrollback. Use this recovery sequence inside one synchronized output buffer: hide cursor, home (`CSI H`), clear screen (`CSI 2J`), clear scrollback (`CSI 3J`), render rows, position/show cursor as appropriate, then disable synchronized output. If the full logical buffer is taller than the terminal, writing it from home naturally recreates scrollback from canonical `fpy` state. If the buffer is shorter than the terminal, the projection starts at the top of the screen after recovery; that is acceptable for the first implementation. Even after full clear, subsequent normal differential operations should use the relative logical cursor model, not absolute screen addressing.

### 6. Differential render

Implement the pi-style diff algorithm:

1. Compute width/height changes.
2. If width changed: full render with clear.
3. If height changed: full render with clear.
4. Compute `first_changed` and `last_changed` comparing `previous_lines` to `new_lines` by exact raw string equality, including ANSI escape sequences. ANSI changes are real render changes.
5. If no changes: only reposition cursor.
6. If `first_changed < previous_viewport_top`: full render with clear.
7. If rows were deleted/shrunk, use pi-style safe shrink handling:
   - If the target row/end of new content is above `previous_viewport_top`, full render with clear.
   - Otherwise, move to the end of the new visible content, clear deleted visible rows with `Clear(CurrentLine)` / `CSI 2K`, and move back as needed.
   - If the number of rows to clear exceeds terminal height or otherwise cannot be addressed safely, full render with clear.
8. Otherwise render changed visible rows.

Safe means all rows that need repainting or clearing are inside the previous visible viewport. Be conservative in the first implementation: full clear recovery if `new_viewport_top < previous_viewport_top` or `first_changed < previous_viewport_top`; otherwise clear/repaint only the visible range. If deletion makes the viewport move upward (`new_viewport_top < previous_viewport_top`), rows newly appearing at the top were not previously addressable.

For safe shrink/delete handling, prefer this concrete first implementation:

- compute the old visible bottom and new visible bottom,
- clamp the starting logical row to the visible/addressable range, e.g. `start = first_changed.max(previous_viewport_top).max(new_viewport_top)` as appropriate for the clear/repaint operation,
- clear from `start` through the old visible bottom with `\r`, `Clear(CurrentLine)` / `CSI 2K`, and `CSI 0m`,
- repaint current visible rows from `start` through the new visible bottom,
- route every clear/repaint row through a helper that moves to a logical row relative to the current viewport and updates `hardware_cursor_row`,
- update `previous_lines` and `previous_viewport_top`.

For normal differential repaint, clear each row before printing and repaint from `first_changed` through the affected visible range. Repainting through the end of the current visible viewport is acceptable if simpler.

### 7. Scrolling for appended rows

If the new tail viewport starts below the previous viewport, scroll by the viewport delta:

```rust
let new_viewport_top = new_lines.len().saturating_sub(height);
let scroll = new_viewport_top.saturating_sub(previous_viewport_top);
if scroll > 0 {
    write!(writer, "{}", "\r\n".repeat(scroll));
    previous_viewport_top += scroll;
    hardware_cursor_row = previous_viewport_top + height.saturating_sub(1);
}
```

If `scroll >= height`, all old visible content has scrolled away. The append scroll itself is still physically valid, but differential assumptions about old visible rows are no longer useful; repaint the full new visible viewport after scrolling, or use full clear recovery if simpler.

Use helpers that take an explicit current viewport top and target viewport top so movement never mixes stale and new viewport bases:

```rust
current_screen_row = hardware_cursor_row - current_viewport_top;
target_screen_row = target_row - target_viewport_top;
```

After any required scroll, use pi-style relative cursor movement from `hardware_cursor_row` to the target logical row. After append scrolling, use only the updated viewport base for subsequent movement/repaint calculations; do not mix old and new viewport tops. Do **not** use absolute `MoveTo(0, screen_y)` for normal differential rendering after a `clear=false` first render: the projection may have started in the middle of the terminal, so screen row 0 may belong to preserved shell context.

Compute relative movement using logical rows and viewport tops:

```rust
let current_screen_row = hardware_cursor_row.saturating_sub(previous_viewport_top);
let target_screen_row = target_row.saturating_sub(new_viewport_top);
let line_diff = target_screen_row as isize - current_screen_row as isize;
```

Then emit `CSI n B` for positive movement or `CSI n A` for negative movement, followed by `\r` to move to column 0. Row repaint should use this sequence: move to logical row, `\r`, `CSI 2K`, row text, `CSI 0m`. Update `hardware_cursor_row` to the logical row where the terminal cursor ends after every clear/repaint/cursor-positioning operation.

This viewport-delta scroll is safer than basing scroll solely on `first_changed - 1`, because the viewport movement is the actual amount of terminal scrolling needed to keep the tail bottom-pinned.

### 8. Cursor handling

Continue using `CursorState` from `TerminalFrame`.

After rendering, position the hardware cursor only if `frame.cursor.visible` and `frame.cursor.position` is inside the current viewport. Confirm `CursorState.position.x` is a terminal cell column, not a byte index; if not, convert before cursor movement.

Use the same relative movement model as row repainting. Do not use absolute `MoveTo` after a `clear=false` first render unless a full clear/home has established screen origin.

Pseudo:

```rust
fn position_cursor(cursor: &CursorState, viewport_top: usize, hardware_cursor_row: &mut usize) {
    if !cursor.visible { Hide; return; }
    let Some(pos) = cursor.position else { Hide; return; };
    let target_row = pos.y as usize;
    if target_row < viewport_top || target_row >= viewport_top + height {
        Hide;
        return;
    }

    let current_screen_row = hardware_cursor_row.saturating_sub(previous_viewport_top);
    let target_screen_row = target_row - viewport_top;
    move_relative_rows(target_screen_row as isize - current_screen_row as isize);
    move_to_column(pos.x);
    SetCursorStyle(...);
    Show;
    *hardware_cursor_row = target_row;
}
```

This is similar to current cursor clipping logic but with `viewport_top` and relative movement instead of `origin_y`/`first_full_row` plus absolute screen rows.

`hardware_cursor_row` is a logical row index, not a screen row. It records where the terminal cursor is believed to be in the logical buffer after rendering/cursor positioning. It is required for normal differential rendering because first render preserves shell context and does not establish a known physical screen origin. Absolute `MoveTo` should be reserved for full-clear/home paths or cursor positioning known to be inside the current viewport after a full reset; normal diff repaint should use relative movement like pi.

### 9. Exit cleanup

On shutdown, leave the final projection visible. Do not clear screen or scrollback.

The backend/session cleanup should:

1. reset styles (`CSI 0m` or equivalent),
2. always show the cursor,
3. move to column 0,
4. use `hardware_cursor_row` and `previous_viewport_top` to move down to the bottom of the current visible projection; this must also work when the last render hid the cursor or the projection is shorter than terminal height,
5. emit `\r\n` so the shell prompt appears after `fpy` (including when already on the bottom row),
6. flush output.

This should be a backend projection/cursor-placement method called by shutdown and selected through the same backend/env switch as rendering. It should not replace existing terminal mode cleanup for raw mode, bracketed paste, keyboard protocol, etc.

Add or keep tmux coverage such as `kernel_exit_returns_shell` to verify shell prompt placement.

### 10. Remove old hybrid backend concepts

Once the new backend is used, remove or stop using:

- `origin_y`
- `previous_origin_y`
- `previous_committed_rows`
- `previous_append_safe`
- `pending_scrollback_recovery`
- `needs_full_projection_reset`
- live-row append protection
- `recovery_pending_frame`
- append-safe transcript distinction in the backend

`RecordingBackend` may still keep transcript/scrollback expectations for tests, but those expectations should be revised for the new behavior.

### 11. Tests

Keep tmux coverage as the main acceptance suite.

Critical tests:

```bash
cargo test --test tmux_e2e bottom -- --nocapture
```

Must include/pass:

- `bottom_started_stdout_preserves_all_lines`
- `bottom_started_history_search_shows_results_not_just_preview`
- `bottom_print_preserves_input_cell_after_scrolling_output`
- new `bottom_started_history_search_close_then_print_preserves_transcript`
- `long_output_transition_to_bottom_pinned_preserves_tail`
- `bottom_pinned_streaming_output_then_short_output_executes_cleanly`
- `bottom_pinned_transcript_repaint_clears_stale_busy_status`

Add the new regression:

```text
bottom_started_history_search_close_then_print_preserves_transcript
```

Scenario:

1. `TMUX_SIZE=80x12`
2. `PRE_LAUNCH_FILL_LINES=11`
3. start `fpy`
4. `Ctrl-R`
5. `Esc`
6. submit `print('\n'.join(str(i) for i in range(10)))`
7. assert:
   - input line appears
   - output lines `0..9` appear in order
   - no duplicate output lines appear after footer

Also include/keep exit cleanup coverage, especially `kernel_exit_returns_shell`, because the new backend changes cursor/viewport state management.

Add backend-unit tests for the new pi-style renderer:

- first render preserves shell context: no clear/home/scrollback purge in output,
- unsafe changed row above viewport triggers full clear recovery,
- append scroll emits the viewport delta and repaints expected rows,
- safe logical shrink clears stale visible rows without full clear,
- cursor positioning uses relative moves rather than absolute `MoveTo`,
- zero-sized terminal returns without mutating previous render state.

Inventory `tests/tmux_e2e.rs` before implementation so missing named tests are added rather than assumed to exist.

`RecordingBackend` and snapshot tests should shift away from append-only scrollback expectations. For the Pi-style backend, tests should primarily assert:

- final visible projection,
- canonical transcript/display rows,
- update classification/debug counters where useful,
- safe recovery behavior when diff rows are outside the visible viewport.

Also run:

```bash
cargo test
cargo clippy --all-targets --all-features
```

## Expected behavior changes

- Full redraw/recovery may clear terminal scrollback.
- Historical rows may be reprojected from canonical state.
- Overlay open/close should no longer corrupt later transcript appends.
- Bottom-of-screen stdout should not lose or duplicate rows.
- Resize should be simpler and safer because width/height changes can force full redraw.

## Risks

- Users may notice more aggressive clearing/redrawing than before.
- Some tests that assert scrollback append behavior will need updates or removal.
- Exit cleanup must be checked carefully so the shell prompt appears below the final rendered projection.

## Recommendation

Implement the new backend alongside the old one, switch `AppUi` to it behind a temporary env var, and validate tmux bottom/reflow/exit tests. Once stable, make it the default and delete the old hybrid append/live repaint logic.
