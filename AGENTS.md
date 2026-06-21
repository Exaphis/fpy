# AGENTS

This file is for future Codex instances working in this repo.

## Project Summary

`fpy` is a Rust terminal frontend for `ipykernel`.

Key design goals:

- own canonical transcript/display state in `fpy` and render it into the normal terminal
- keep normal terminal scrollback semantically faithful to committed cell inputs/outputs
- do not treat terminal scrollback as the canonical transcript source of truth
- avoid alternate-screen behavior
- feel closer to IPython than to a fullscreen TUI app

Display/scrollback policy:

- Byte-for-byte historical screen layout is not a goal. Resize, reflow, and redraw may change wrapping, styling, or visible layout.
- Scrollback fidelity means committed transcript-content fidelity: no duplicated, missing, stale, reordered, or mangled committed cell inputs/outputs.
- Live UI rows such as the editor, footer, status, palettes, and history search should not leak into scrollback as transcript content.
- Full redraw/recovery paths may be pi-like, but be careful with terminal scrollback clears (`CSI 3J`): clearing all scrollback is acceptable only as an explicit recovery/strategy choice, not as an accidental default.

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
  Recording backend and differential normal-terminal crossterm backend.

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
- If a bug only appears when the prompt is near the bottom of the screen, check frame classification and visible-row repainting in `src/ui/backend/` first.
- If a bug only appears during editing, check whether it is an `edtui` behavior. Prefer fixing such behavior in `vendor/edtui` before adding `fpy`-specific glue.
- If you change prompt sizing or viewport logic, rerun tmux repros immediately.

## Current Direction

The codebase was recently refactored to split the large single-file modules into:

- `src/kernel/`
- `src/ui/`

Keep moving in that direction. Avoid growing `src/ui/mod.rs` or `src/kernel/mod.rs` back into giant mixed-responsibility files.
