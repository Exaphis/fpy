# Resize full-redraw plan

## Decision

On terminal resize, `fpy` should treat the terminal projection as invalid and perform a pi-style full redraw:

1. clear the visible screen;
2. clear terminal scrollback;
3. redraw the current frame from canonical `fpy` display state.

This intentionally makes `fpy` state, not terminal scrollback, the source of truth after resize.

## Rationale

Terminal resize is not a reliable incremental operation for `fpy`'s normal-screen UI. Terminals and tmux may reflow existing physical rows into different visible/scrollback rows before `fpy` receives the resize event. Once live UI rows such as editor text, footer/status rows, palettes, or old-width wrapped fragments have entered terminal scrollback, `fpy` cannot reliably remove them without clearing scrollback.

The current backend tries to preserve and patch terminal contents across resize using an origin row plus differential row repainting. That model is fragile because it assumes terminal rows remain a stable render target across size changes. They do not.

Pi's TUI avoids this class of bugs by treating width and most height changes as full-redraw events. It clears the terminal projection and renders again from owned app state. `fpy`'s refactored display model now supports the same product assumption: canonical transcript/display state is owned by `fpy`; terminal scrollback is only a projection.

## Product semantics

This changes what resize promises:

- Canonical transcript and display state must remain intact in `fpy`.
- The visible UI after resize must be correct and rendered from canonical state.
- Terminal scrollback is allowed to be cleared on resize.
- Byte-for-byte historical terminal layout is not preserved across resize.
- Live UI rows must not leak into post-resize scrollback.
- Committed transcript content must not be corrupted in canonical state.

This is consistent with the current display direction in `docs/pi-style-tui-refactor-plan.md`: terminal scrollback is not the canonical transcript source of truth.

## Proposed behavior

On `Event::Resize`:

1. Refresh terminal size.
2. Mark the frame backend as requiring resize recovery.
3. On the next redraw, emit a full terminal reset for the projection:
   - synchronized output begin if supported/desired;
   - hide cursor;
   - clear visible screen (`CSI 2J`);
   - home cursor (`CSI H`);
   - clear scrollback (`CSI 3J`);
   - render the current `TerminalFrame` from `DisplayModel`;
   - position cursor;
   - synchronized output end;
   - flush.
4. Replace backend caches (`previous_committed_rows`, `previous_visible_rows`, `previous_size`, origin/viewport bookkeeping) with the freshly rendered state.

The redraw should render only the current frame/viewport, not replay the entire historical transcript into terminal scrollback. Historical transcript remains available from canonical `DisplayModel` and future resume/export/history features.

## Backend implications

`CrosstermMainScreenBackend` should stop trying to preserve normal-screen row identity across resize. Resize should bypass append/diff classifications and force a full projection redraw.

Suggested backend state:

```rust
enum RedrawMode {
    Differential,
    FullProjectionReset,
}
```

or a simple flag:

```rust
needs_full_projection_reset: bool
```

`refresh_size()` can set this flag when size changes. `draw_frame()` consumes it.

Frame classification can still exist for non-resize updates:

- `Initial`
- `TranscriptAppend`
- `LiveUiOnly`
- `Recovery`

But resize should no longer use `ResizeOrReflow` to drive a differential repaint. It should drive `FullProjectionReset`.

## Tests to add/update

### tmux e2e

Add/strengthen a repeated resize test with a long live editor input, e.g. `"a" * 500`:

- type 500 `a`s into the editor;
- resize through several widths/heights;
- capture visible pane and scrollback;
- assert only one live prompt/footer exists after resize;
- assert no stale old-width wrapped editor rows remain above the current frame;
- assert no live UI duplicates exist in scrollback;
- submit after resize and assert execution still works;
- assert committed canonical transcript after submit is correct.

Because scrollback is intentionally cleared on resize, tests should not expect pre-resize shell/fpy launch lines or previous terminal transcript rows to remain in tmux capture.

### backend/unit fixtures

Add backend tests for resize full reset:

- size change causes full projection reset output, not transcript append;
- reset clears backend row caches before installing the new frame;
- live UI rows are not appended as committed transcript rows;
- subsequent transcript append after resize behaves normally.

### regression expectations

The desired post-resize capture should look like a clean current fpy frame, not a preserved terminal history. This is different from previous tests that asserted shell sentinel lines remained above the frame.

## Open questions

- Should scrollback clearing on resize be unconditional, or behind an escape hatch such as `FPY_RESIZE_STRATEGY=clear|preserve`?
- Should `Ctrl-L` use the same full projection reset path?
- Should a future transcript browser/export command make the loss of terminal-native scrollback after resize less surprising?
- Should startup/exit behavior print a small notice when a resize clears terminal scrollback? Probably not by default; it would add noise.

## Non-goals

- Preserve exact terminal scrollback layout across resize.
- Repair terminal/tmux reflow artifacts without clearing scrollback.
- Replay the full transcript into terminal scrollback on every resize.
- Treat terminal scrollback as the authoritative transcript.
