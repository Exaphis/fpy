# AGENTS

This file is for future Codex instances working in this repo.

## Project Summary

`fpy` is a Rust terminal frontend for `ipykernel`.

Key design goals:

- own canonical transcript/display state in `fpy` and render it into the normal terminal
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
  `AppUi` state machine. It coordinates state transitions and frame redraws.

- [`src/ui/display.rs`](src/ui/display.rs)
  Canonical display/transcript model, frame renderer, cursor state, and display fixtures.

- [`src/ui/components/`](src/ui/components)
  Component-style renderers for transcript, editor, footer, and overlays.

- [`src/ui/backend/`](src/ui/backend)
  Recording backend and Pi-style normal-terminal backend.

- [`src/ui/session.rs`](src/ui/session.rs)
  Raw mode, bracketed paste, keyboard protocol setup, and exit cleanup.

- [`src/ui/editor.rs`](src/ui/editor.rs)
  `edtui` integration and editor setup helpers.

- [`vendor/edtui`](vendor/edtui)
  Vendored `edtui` dependency. Put editor-core fixes and Vim-emulation fixes here instead of stacking local shims in `src/ui/`.

- [`src/ui/render.rs`](src/ui/render.rs)
  Small status/render helpers shared by `AppUi`.

- [`src/ui/transcript.rs`](src/ui/transcript.rs)
  ANSI-aware transcript formatting and syntax-highlighted `In [...]` echo rendering.

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

For visual UI/UX checks that need ANSI captures or launch-position metadata, use:

```bash
scripts/fpy-tmux-visual-repro.sh startup-anchor
scripts/fpy-tmux-visual-repro.sh footer-styling
scripts/fpy-tmux-visual-repro.sh bottom-prompt
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
- `target/fpy-tmux-repro.after.ansi.log`
- `target/fpy-tmux-repro.after.meta`

Do not trust non-interactive PTY behavior for terminal bugs unless tmux shows the same thing.

## Known Gotchas

- The vendored `edtui` should treat empty editor buffers as one blank row via `EditorState::new`. Do not reintroduce an fpy-side workaround for `Lines::from("")` unless the vendored behavior changes.

- Bracketed paste is enabled in [`src/ui/mod.rs`](src/ui/mod.rs). Pasted text is normalized from `\r\n` / `\r` to `\n` before being handed to `edtui`.

- Frame backend changes must keep committed transcript rows distinct from live UI rows. Transcript growth may append; live UI edits, resize, and recovery should repaint without duplicating committed transcript content.
- The active normal-terminal direction is Pi-style projection rendering. `DisplayModel` / `TerminalFrame.full_rows` are the canonical logical buffer; the backend is only a terminal projection cache. Do not add transcript-vs-live append bookkeeping to the Pi backend.

- The Pi-style backend should not track startup origin or physical addressable rows. Preserve shell context on first render by writing without clearing. After that, use logical line-buffer diffing, viewport-top tracking, and relative movement. If a visible short projection grows in place, create room by moving to the old projection bottom, emitting the required `\r\n` rows, and repainting; do not purge scrollback for ordinary visible growth.

- Unsafe Pi-style recovery is allowed to clear screen and scrollback (`CSI 2J`, home, `CSI 3J`) and redraw from canonical state, but it should be reserved for truly unsafe diffs such as changed rows above the previous viewport, viewport movement upward, resize/reflow, or non-tail growth that moves the viewport down.

- Tail appends must scroll by viewport delta: `new_viewport_top.saturating_sub(previous_viewport_top)`. Do not special-case tail append as a single newline; multi-row appends must keep physical and logical viewports aligned.

- Pi-style differential writes should build a complete body before entering synchronized output. Fallible movement/repaint computation must happen before `CSI ? 2026 h` is added, or the disable sequence must otherwise be guaranteed.

- `TerminalFrame.full_rows` are expected to be width-shaped by components/display rendering. The Pi backend enforces this with ANSI-aware visible-width checks because unexpected terminal wrapping invalidates logical cursor-row tracking. Fix over-wide rows in `src/ui/components/`, `src/ui/display.rs`, or width-sensitive model construction rather than relaxing the backend contract.

- When building width-sensitive UI state in `AppUi`, refresh terminal size first so component models and backend frame size use the same width.

- The biggest remaining architectural tension is between `fpy` wanting shell-like inline behavior and `edtui` being a generic `ratatui` editor widget. Since `edtui` is vendored, prefer making editor-core/Vim-fidelity changes in `vendor/edtui` rather than adding more local workarounds.

- Empty-line visual selection in `edtui` is effectively invisible because selections restyle existing spans, and empty lines have no spans. If that matters, fix it in the editor layer, not with transcript hacks.

- Completions are intentionally deferred for now. The current recommendation is to implement a first pass in `fpy` itself
  (Jupyter completion requests + `fpy`-owned suggestion UI/state) before deciding to fork or vendor `edtui`.
  A fork becomes more attractive if completions need to feel editor-native or if more `edtui`-level fixes pile up.

Current likely candidates for future vendored `edtui` work:

- General Vim count prefixes beyond the current vendored `nG` support.
- Empty-line visual selection rendering.
- Empty-buffer semantics so `""` behaves like a one-row blank buffer.
- Editor-native completion popup positioning relative to the cursor/viewport.
- Completion insertion/acceptance integrated with editor cursor, selection, and undo semantics.
- Completion navigation semantics in insert mode.
- Inline ghost text / suggestion preview.
- Better extension points for custom overlays, completion sources, or editor-side rendering.

## Practical Guidance

- Prefer fixing terminal behavior with the smallest possible change in `src/ui/display.rs`, `src/ui/components/`, `src/ui/backend/`, or `src/ui/session.rs`.
- If a bug only appears when the prompt is near the bottom of the screen, check Pi-style viewport math, visible-growth room creation, tail append viewport-delta scrolling, and exit cursor placement in `src/ui/backend/` first.
- If a bug only appears during editing, check whether it is an `edtui` behavior. Prefer fixing such behavior in `vendor/edtui` before adding `fpy`-specific glue.
- If you change prompt sizing or viewport logic, rerun tmux repros immediately.

## Current Direction

The codebase was recently refactored to split the large single-file modules into:

- `src/kernel/`
- `src/ui/`

Keep moving in that direction. Avoid growing `src/ui/mod.rs` or `src/kernel/mod.rs` back into giant mixed-responsibility files.
