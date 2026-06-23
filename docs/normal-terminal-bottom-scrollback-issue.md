# Normal-terminal bottom scrollback issue

## Summary

`fpy` can lose or visually mangle committed transcript rows when the live UI is rendered near the bottom of the terminal. This is not specific to history search. History search makes the problem obvious because the overlay is tall, but ordinary stdout can also lose committed input/output rows.

The underlying issue is in the normal-screen projection/backend policy: committed transcript append and live UI repaint can contend for the same physical terminal rows when there is not enough space below the current live UI origin.

## Reproduced symptoms

### Startup near the last line

When `fpy` starts near the last line of a small tmux pane, history search may show only the bottom of the overlay, e.g. only:

```text
preview
import os
os.getcwd()
 INS  In [1] Ctrl-P palette
```

Missing rows include:

- `History Search`
- `query: ...`
- result list rows

The same bottom-start condition can corrupt stdout. For example:

```python
print('\n'.join(str(i) for i in range(8)))
```

Expected:

```text
0
1
2
3
4
5
6
7
```

Observed during reproduction:

```text
0
1
2
3
4
7
```

Rows `5` and `6` were missing from the terminal projection.

### Printing while already near the bottom

The issue is not only startup anchoring. If a prior cell scrolls the prompt/live UI to the bottom, a later print can lose the input cell itself.

Reproduction pattern:

```python
print('\n'.join(str(i) for i in range(20)))
print('\n'.join(str(i) for i in range(10)))
```

Observed failure: the second cell's committed input line:

```text
In [2]: print('\n'.join(str(i) for i in range(10)))
```

can be missing, even though its output and runtime line appear.

## Current uncommitted status

There is partial uncommitted work that improves startup-near-bottom behavior by reserving more rows at startup. However, that is not a complete fix.

Known problems with the partial approach:

- It uses a magic startup reserve, currently `height.min(12)`.
- It addresses only startup anchoring, not later transcript appends that push the live UI to the bottom.
- Experimental backend attempts to add extra scrolling during append introduced more magic numbers and caused duplicate/missing rows in other bottom-pinned cases.
- The tmux regression `bottom_print_preserves_input_cell_after_scrolling_output` currently captures the broader print-at-bottom failure and should remain as a failing test until the backend invariant is fixed.

## Root cause

`CrosstermMainScreenBackend` maintains an `origin_y` for live UI drawing. It draws visible rows by slicing the canonical frame to the number of physical rows available below `origin_y`:

```rust
available_height = frame.size.height - origin_y;
start = frame.full_rows.len().saturating_sub(available_height);
visible_rows = frame.full_rows[start..];
```

When `origin_y` is near the bottom, `available_height` is small. For live overlays this clips the top of the overlay. For transcript appends, it is worse: committed rows are appended to normal terminal scrollback, then live rows are repainted starting at `origin_y`. If those physical rows overlap, the live repaint can clear or overwrite rows that should have become committed scrollback.

The display model/canonical transcript can still be correct. The bug is in projecting that model onto the normal terminal without first guaranteeing enough physical space for live UI.

## Desired invariant

The backend should maintain this invariant before drawing live UI:

> There must be enough physical terminal rows below `origin_y` to draw the current live UI rows, and committed transcript rows must already be safely represented in scrollback.

This invariant must be enforced:

- at startup
- after every transcript append
- after resize/reflow/recovery paths
- before drawing overlays such as history search

## Proposed fix

Remove fixed startup-only reserves and replace them with a backend-level `ensure_live_viewport` step.

### 1. Split frame rows

For each frame, derive:

```rust
committed_rows = rows where kind == RowKind::CommittedTranscript
live_rows = rows where kind == RowKind::LiveUi
```

The required live viewport height is:

```rust
required_live_rows = live_rows.len().clamp(1, terminal_height)
```

For wrapped terminal rows, use the already-rendered row count; each `TerminalRow` is one physical row.

### 2. Ensure enough live viewport space

Before `draw_visible_rows`, guarantee:

```rust
available_rows = terminal_height - origin_y
available_rows >= required_live_rows
```

If not:

```rust
deficit = required_live_rows - available_rows
print "\r\n" deficit times
origin_y = origin_y.saturating_sub(deficit)
```

This scrolls only as much as needed. No fixed `12` row startup reserve.

### 3. Apply the same logic after transcript append

On append-safe transcript growth:

1. Append newly committed rows normally.
2. Adjust `origin_y` by the number of physical rows scrolled by the append.
3. Run `ensure_live_viewport(required_live_rows)` before repainting live UI.

The key is that the backend must reason about where the terminal cursor/scrollback is after appending committed rows, then reserve live UI space before clearing/repainting live rows.

### 4. Do not mark unsafe appends as committed

If an update is classified as `TranscriptAppend` while the previous frame was not append-safe, do not advance `previous_committed_rows` as if the rows are safely present in scrollback. Keep recovery pending until a frame can either:

- perform an explicit full projection reset/recovery, or
- append/reconstruct committed rows safely under the live viewport invariant.

Otherwise future frames can believe rows are already in scrollback and those rows may disappear permanently from the terminal projection.

### 5. Startup becomes a special case of the same invariant

Startup should not have a separate magic reserve. It should initialize `origin_y` from the cursor position, build the first frame, compute live rows, and call the same `ensure_live_viewport` logic before drawing.

If early startup needs a pre-frame reserve, it should be parameterized by the first frame's live row count, not by a constant.

## Test coverage to keep/add

### Existing/new tmux regressions

Keep or add these tmux tests:

- `bottom_started_stdout_preserves_all_lines`
  - Start `fpy` near the last line.
  - Run `print('\n'.join(str(i) for i in range(8)))`.
  - Assert input and all output lines appear.

- `bottom_started_history_search_shows_results_not_just_preview`
  - Start `fpy` near the last line.
  - Open history search.
  - Assert header, query, result list, preview, and footer appear.

- `bottom_print_preserves_input_cell_after_scrolling_output`
  - In a small pane, run a long print to push the prompt/live UI to the bottom.
  - Run a second print.
  - Assert the second input cell line and all output rows appear.
  - This is the important non-startup regression.

### Existing bottom-pinned tests that must keep passing

Any fix should also pass current bottom/scrollback coverage, including:

- `multiline_growth_bottom_pinned`
- `bottom_of_screen_result_still_visible`
- `long_output_transition_to_bottom_pinned_preserves_tail`
- `bottom_pinned_streaming_output_then_short_output_executes_cleanly`
- `bottom_pinned_transcript_repaint_clears_stale_busy_status`

These tests catch duplicate-tail and stale-live-row regressions introduced by naive extra scrolling.

## Caution

Do not fix this by only shrinking history search or increasing a startup reserve. The stdout/input-loss reproductions prove the issue is a general backend projection invariant problem.

Also avoid unconditional full-screen scrolls. They are visually disruptive and unnecessary. The backend should scroll only the exact deficit required for the current live UI viewport.
