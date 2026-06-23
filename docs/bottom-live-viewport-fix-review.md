# Bottom live viewport fix review

## Context

A backend fix was implemented to address terminal corruption when `fpy` runs near the bottom of the normal terminal. The intended invariant is:

> Before drawing live UI, there must be enough physical terminal rows below `origin_y` to draw the current live UI rows, and committed transcript rows must already be safely represented in scrollback.

The implemented direction is broadly aligned with that invariant: it adds live-row accounting, dynamic viewport reservation, append/recovery state handling, and tmux regressions for bottom-start and bottom-print cases.

However, further manual reproduction shows the behavior is still incorrect after opening and closing history search at the bottom.

## Manual reproduction: history search then print at bottom

Steps:

1. Start `fpy` near the bottom line of a small tmux pane, e.g. `80x12`.
2. Press `Ctrl-R` to open history search.
3. Press `Esc` to close history search.
4. Run:

```python
print('\n'.join(str(i) for i in range(10)))
```

Expected transcript:

```text
fpy 0.1.0
In [1]: print('\n'.join(str(i) for i in range(10)))
0
1
2
3
4
5
6
7
8
9
[...]
In [2]:
 INS  In [2] Ctrl-P palette
```

Observed by user:

```text
fpy 0.1.0
In [1]: print('\n'.join(str(i) for i in range(10)))
0
1
2
3
4
5
6
7
8
9
[37.6ms]
In [2]:
 INS  In [2] Ctrl-P palette
2
3
4
5
6
7
8
9
```

That shows stale/duplicated output rows after the live footer.

Observed in a local tmux reproduction:

```text
fpy 0.1.0
1
2
3
4
5
6
7
8
9
[26.9ms]
In [2]:
 INS  In [2] Ctrl-P palette
```

In that run, the committed input line and output `0` were lost entirely.

Capture files from the local reproduction:

- `target/bottom-history-esc-print.log`
- `target/bottom-history-esc-print.ansi.log`

## Why this still fails

Opening history search near the bottom triggers live viewport reservation for a tall overlay. The backend scrolls blank rows to create enough physical space for the overlay.

Closing history search then replaces the tall overlay with the much smaller editor/footer live UI. The terminal now contains real scrollback rows that were created only to reserve temporary live UI space for the overlay.

On the next transcript append, the backend attempts to:

1. clear previous live rows,
2. append committed transcript rows,
3. reserve/repaint live rows.

But the backend state does not distinguish between:

- scrollback rows that are committed transcript content, and
- scrollback rows introduced only as temporary live viewport reservation for an overlay.

As a result, the append path miscalculates what terminal rows are safe transcript rows versus stale live/reserve rows. Depending on timing and exact terminal state, committed rows can be:

- overwritten by live repaint,
- omitted from scrollback,
- duplicated after the footer,
- or mixed with stale output below the live UI.

The key issue is that **live viewport reservation is allowed to become real scrollback without being tracked as non-transcript temporary space**. Once history search has forced bottom scrolling, later transcript appends treat the terminal as if it is in a clean appendable state, but it is not.

## Code review findings

### 1. Documentation is stale

`docs/normal-terminal-bottom-scrollback-issue.md` still describes the fix as proposed/partial and says a bottom-print test should remain failing. After the current implementation, some bottom tests pass, but the history-search-then-print case still fails.

That document should be updated after the final approach is chosen.

### 2. Duplicate scrollback purge after full reset

The implemented backend calls `Clear(ClearType::Purge)` inside `full_projection_reset()`. The new draw path also queues another purge after full reset:

```rust
if full_projection_reset {
    self.clear_rows_below_visible_frame(origin_y)?;
    queue!(self.writer, Clear(ClearType::Purge))?;
}
```

The second purge appears redundant and potentially too aggressive. It should be removed unless a specific repro requires it.

### 3. Subtle behavior needs comments

The new backend logic is difficult to reason about and needs explanatory comments around:

- `previous_live_viewport_touches_bottom()`
- `live_reserve_scroll_rows(required_live_rows)` and why it subtracts one row
- `protect_bottom_live_viewport`
- `VisibleDrawMode::LiveUiOnly`
- `recovery_pending_frame()`

Without comments, future changes are likely to reintroduce scrollback corruption.

### 4. Current fix partially matches the invariant

Good parts:

- Startup magic reserve was removed.
- Live viewport size is based on actual live row count.
- Some append/recovery state no longer advances committed rows prematurely.
- Existing bottom tmux tests pass.

Remaining problem:

- Temporary live viewport reservation, especially for overlays, is not accounted for as non-transcript scrollback. This makes later transcript appends unsafe after an overlay was opened near the bottom.

## Proposed direction for a corrected fix

The backend needs an explicit model for whether the normal terminal is currently in a clean appendable state.

### Option A: Overlay reservation invalidates append safety

When `ensure_live_viewport()` scrolls rows for live UI only, especially for overlays, mark the terminal projection as requiring recovery before the next committed append.

Possible policy:

- If live viewport reservation scrolls while there is no transcript append in progress, set `pending_scrollback_recovery = true` or a separate `pending_live_reservation_recovery = true`.
- The next transcript append must not use the simple append path.
- Instead, it must perform an explicit recovery/reset path that reconstructs committed transcript rows from the canonical model.

This is conservative and likely correct, but may clear scrollback more often after overlay-at-bottom interactions.

### Option B: Track reserved live-scroll rows separately

Maintain a count of rows scrolled only to make room for live UI:

```rust
reserved_live_scroll_rows: usize
```

Then, before a transcript append, account for or consume those rows explicitly so they are not confused with transcript rows.

This is more precise but more complex. It requires careful handling across:

- overlay open/close,
- live UI shrinking,
- resize,
- recovery reset,
- shutdown cursor placement,
- transcript append after pending recovery.

### Option C: For overlays at bottom, prefer full projection recovery

Since overlays are temporary and can be tall, a simpler policy may be:

- If an overlay requires live viewport scrolling, do not leave the backend in append-safe mode.
- When the overlay closes, force a full projection reset/recovery before accepting transcript append.

This may be the most pragmatic approach for now.

## Tests to add

Add a tmux regression for the new failure:

```text
bottom_started_history_search_close_then_print_preserves_transcript
```

Suggested scenario:

1. `TMUX_SIZE=80x12`
2. `PRE_LAUNCH_FILL_LINES=11`
3. start `fpy`
4. `Ctrl-R`
5. `Esc`
6. submit `print('\n'.join(str(i) for i in range(10)))`
7. assert:
   - `In [1]: print('\n'.join(str(i) for i in range(10)))` appears
   - output lines `0` through `9` appear exactly in transcript order
   - no duplicate output lines appear after the footer

Also keep the existing bottom tests:

- `bottom_started_stdout_preserves_all_lines`
- `bottom_started_history_search_shows_results_not_just_preview`
- `bottom_print_preserves_input_cell_after_scrolling_output`
- `long_output_transition_to_bottom_pinned_preserves_tail`
- `bottom_pinned_streaming_output_then_short_output_executes_cleanly`
- `bottom_pinned_transcript_repaint_clears_stale_busy_status`

## Recommendation

Do not continue adding small compensating scroll calculations to the append path. That has already produced duplicate tails and missing input rows in adjacent cases.

Instead, make live viewport reservation part of the backend state model:

- distinguish transcript scrollback from temporary live-reservation scrollback, or
- conservatively mark append as unsafe after live-only reservation and recover before the next transcript append.

The second approach is simpler and probably safer for the current architecture.
