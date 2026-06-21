# Force repaint cleanup proposal

## Background

`CrosstermMainScreenBackend` currently has two overlapping ways to bypass normal row diffing:

- `force_repaint: bool`
- `needs_full_projection_reset: bool`

Before resize switched to pi-style projection reset, `force_repaint` was useful for repainting rows after terminal size changes without relying on `previous_visible_rows`. After the resize implementation changed to clear screen + clear scrollback + redraw from canonical state, resize no longer needs a separate forced-diff path.

Current resize path:

1. `refresh_size()` detects size change.
2. It sets `needs_full_projection_reset = true`.
3. `draw_frame()` calls `full_projection_reset()`.
4. `full_projection_reset()` clears the terminal projection, resets backend caches, and currently sets `force_repaint = true`.
5. `draw_visible_rows()` sees `force_repaint`, clears rows above `origin_y`, ignores row equality, then resets `force_repaint = false`.

Because `full_projection_reset()` already clears the visible screen, resets `origin_y` to 0, clears `previous_visible_rows`, and resets `previous_origin_y`, the additional `force_repaint` behavior is redundant for the resize path.

## Problem

`force_repaint` now adds mental overhead and creates ambiguous rendering modes:

- `needs_full_projection_reset` means terminal projection is invalid and should be rebuilt.
- `force_repaint` means previous row cache should be ignored for one draw.

Those are distinct concepts in theory, but the current production use is coupled: full projection reset sets force repaint. This makes it less clear which invariant each flag protects.

It also keeps old resize-era logic in `draw_visible_rows()`:

- clear rows above `origin_y` when `force_repaint` is true;
- skip row equality checks only when `force_repaint` is false;
- clear `force_repaint` after drawing.

With resize now top-anchored after full reset (`origin_y = 0`), clearing rows above `origin_y` is a no-op in the main resize path.

## Proposal

Remove `force_repaint` from `CrosstermMainScreenBackend`.

Use one explicit reset path:

```rust
needs_full_projection_reset: bool
```

`full_projection_reset()` should:

- hide cursor;
- move to `(0, 0)`;
- clear visible screen;
- clear scrollback;
- move to `(0, 0)`;
- set `origin_y = 0`;
- clear `previous_origin_y`;
- set `last_visible_row_count = 0`;
- clear `previous_committed_rows`;
- clear `previous_visible_rows`;
- clear `previous_size`;
- clear `needs_full_projection_reset`.

Then `draw_visible_rows()` can return to a simple diff renderer:

```rust
for (row, text) in visible_rows.iter().enumerate() {
    if self.previous_visible_rows.get(row) == Some(text) {
        continue;
    }
    queue!(MoveTo(...), Clear(CurrentLine), Print(text))?;
}

for row in visible_rows.len()..self.previous_visible_rows.len() {
    queue!(MoveTo(...), Clear(CurrentLine))?;
}
```

After a full projection reset, `previous_visible_rows` is empty, so all current visible rows naturally draw without a separate flag.

## Optional follow-up: make reset an update kind

Today resize is still represented as `FrameUpdateKind::ResizeOrReflow`, and production separately computes:

```rust
let full_projection_reset =
    self.needs_full_projection_reset || update_kind == FrameUpdateKind::ResizeOrReflow;
```

A later cleanup could rename or split this to make production behavior obvious:

```rust
enum FrameUpdateKind {
    Initial,
    TranscriptAppend,
    LiveUiOnly,
    FullProjectionReset,
    Recovery,
}
```

or keep classification semantic and add a production-only render operation:

```rust
enum RenderOperation {
    Diff,
    AppendTranscript,
    FullProjectionReset,
}
```

This is not required to remove `force_repaint`, but it would reduce confusion around `ResizeOrReflow` now meaning "clear scrollback and redraw" in production.

## Test plan

Update existing backend tests:

- `main_screen_backend_full_resets_without_appending_on_resize` should continue to assert `CSI 2J` and `CSI 3J` are emitted.
- It should assert subsequent transcript append does not emit another `CSI 3J`.
- It should assert post-reset `previous_visible_rows` contains the newly drawn rows.

Add/keep tmux e2e coverage:

- repeated long-editor resize clears sentinel/shell scrollback;
- one prompt/footer remains;
- no stale old-width editor rows remain above the current frame;
- submitting after resize still works.

## Non-goals

- Change resize product behavior.
- Reintroduce preserve-scrollback resize repair.
- Change `RecordingBackend` semantics beyond any naming/documentation cleanup.
