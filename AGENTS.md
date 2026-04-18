# AGENTS

This file is for future Codex instances working in this repo.

## Project Summary

`fpy` is a Rust terminal frontend for `ipykernel`.

Key design goals:

- keep transcript output in the normal terminal scrollback
- use `ratatui` only for the inline prompt/editor area
- avoid alternate-screen behavior
- feel closer to IPython than to a fullscreen TUI app

## Important Files

- [`src/app.rs`](src/app.rs)
  Top-level async loop. Owns bootstrapping, UI redraw cadence, kernel event handling, and shutdown.

- [`src/kernel/mod.rs`](src/kernel/mod.rs)
  Public kernel/session API and local child lifecycle.

- [`src/kernel/runtime.rs`](src/kernel/runtime.rs)
  ZeroMQ socket setup and async recv/send loops.

- [`src/kernel/messages.rs`](src/kernel/messages.rs)
  Maps decoded Jupyter messages into `KernelEvent`s.

- [`src/kernel/diagnostics.rs`](src/kernel/diagnostics.rs)
  Startup and crash diagnostics for local kernels.

- [`src/ui/mod.rs`](src/ui/mod.rs)
  `AppUi` state machine and terminal lifecycle.

- [`src/ui/editor.rs`](src/ui/editor.rs)
  `edtui` integration, gutter rendering, editor setup, and small Vim-specific glue.

- [`src/ui/render.rs`](src/ui/render.rs)
  Pane geometry, inline viewport sizing, throbber/status helpers.

- [`src/ui/transcript.rs`](src/ui/transcript.rs)
  ANSI-aware transcript formatting and syntax-highlighted `In [...]` echo rendering.

- [`src/insert_history/mod.rs`](src/insert_history/mod.rs)
  Writes transcript above the pane, including the bottom-pinned scroll-region path.

- [`src/custom_terminal.rs`](src/custom_terminal.rs)
  Custom terminal wrapper used instead of stock `ratatui::Terminal`. If terminal behavior looks wrong, inspect this before blaming `ratatui`.

## Testing Workflow

Start here:

```bash
cargo test
cargo clippy --all-targets --all-features
```

For anything involving prompt layout, scrollback, exit cleanup, paste, or editor behavior, use tmux:

```bash
scripts/fpy-tmux-repro.sh ctrl-d
scripts/fpy-tmux-repro.sh vim-open-below
scripts/fpy-tmux-repro.sh paste
```

For startup latency comparisons against plain IPython, use:

```bash
scripts/benchmark-startup.sh
```

That benchmark uses `tmux`, builds `fpy` in release mode, and reports:

- time to usable input
- time to safe submission
- time to the first successful `1+1` result

for both `fpy` and `ipython`.

There is also a Rust integration suite in [`tests/tmux_e2e.rs`](tests/tmux_e2e.rs):

```bash
cargo test --test tmux_e2e -- --nocapture
```

Current covered regressions:

- `ctrl_d_preserves_transcript`
- `kernel_exit_returns_shell`
- `multiline_growth_bottom_pinned`
- `bottom_of_screen_result_still_visible`
- `multiline_paste_preserves_all_lines`
- `shift_enter_creates_multiline_editor`
- `vim_open_below_grows_on_first_try`
- `history_up_reruns_previous_cell`

The Rust tests currently drive the shell repro harness rather than talking to `tmux` directly.

If a new end-to-end TUI regression is reported, add a reproducing test to
[`tests/tmux_e2e.rs`](tests/tmux_e2e.rs) before fixing the bug. Treat that as the default workflow
for prompt-layout, paste, scrollback, exit-cleanup, and multiline-editor regressions.

The script writes captures to:

- `target/fpy-tmux-repro.before.log`
- `target/fpy-tmux-repro.after.log`

Do not trust non-interactive PTY behavior for terminal bugs unless tmux shows the same thing.

## Known Gotchas

- `edtui` treats `Lines::from("")` as a zero-row buffer.
  `fpy` works around that in [`src/ui/editor.rs`](src/ui/editor.rs) by forcing one empty row for empty input buffers. Do not remove that unless you also fix the underlying editor behavior.

- Bracketed paste is enabled in [`src/ui/mod.rs`](src/ui/mod.rs). Pasted text is normalized from `\r\n` / `\r` to `\n` before being handed to `edtui`.

- Pane geometry changes must invalidate the custom terminal viewport or stale screen cells remain visible.

- The biggest remaining architectural tension is between `fpy` wanting shell-like inline behavior and `edtui` being a generic `ratatui` editor widget. If more Vim fidelity is needed, a fork of `edtui` may be cleaner than stacking more local workarounds.

- Empty-line visual selection in `edtui` is effectively invisible because selections restyle existing spans, and empty lines have no spans. If that matters, fix it in the editor layer, not with transcript hacks.

- Completions are intentionally deferred for now. The current recommendation is to implement a first pass in `fpy` itself
  (Jupyter completion requests + `fpy`-owned suggestion UI/state) before deciding to fork or vendor `edtui`.
  A fork becomes more attractive if completions need to feel editor-native or if more `edtui`-level fixes pile up.

If `edtui` is forked or vendored later, the current likely candidates are:

- General Vim count prefixes beyond the current local `nG` shim.
- Empty-line visual selection rendering.
- Empty-buffer semantics so `""` behaves like a one-row blank buffer.
- Editor-native completion popup positioning relative to the cursor/viewport.
- Completion insertion/acceptance integrated with editor cursor, selection, and undo semantics.
- Completion navigation semantics in insert mode.
- Inline ghost text / suggestion preview.
- Better extension points for custom overlays, completion sources, or editor-side rendering.

## Practical Guidance

- Prefer fixing terminal behavior with the smallest possible change in `ui/`, `insert_history/`, or `custom_terminal.rs`.
- If a bug only appears when the prompt is near the bottom of the screen, check the bottom-pinned insertion path first.
- If a bug only appears during editing, check whether it is an `edtui` behavior before adding `fpy`-specific glue.
- If you change prompt sizing or viewport logic, rerun tmux repros immediately.

## Current Direction

The codebase was recently refactored to split the large single-file modules into:

- `src/kernel/`
- `src/ui/`
- `src/insert_history/`

Keep moving in that direction. Avoid growing `src/ui/mod.rs` or `src/kernel/mod.rs` back into giant mixed-responsibility files.
