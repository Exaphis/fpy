# fpy

`fpy` is a Rust terminal frontend for Python `ipykernel`.

It is intentionally closer to a lightweight terminal REPL than a notebook UI:

- transcript output stays in the normal terminal instead of an alternate screen
- the current input cell is rendered inline with `ratatui`
- kernel transport uses the Jupyter messaging protocol over ZeroMQ
- the input editor uses `edtui` with Vim-style editing and syntax highlighting

## Current Status

This is an early but usable terminal-first prototype.

Implemented:

- `fpy run` to launch a local `ipykernel`
- `fpy attach --connection-file ...` to attach to an existing kernel
- execute input, plain-text results, stdout/stderr, tracebacks, and `input()`
- interrupt, restart, and connection info via a command palette
- syntax-highlighted input plus syntax-highlighted echoed `In [...]` cells
- Vim-style editing via `edtui`, including multiline editing and bracketed paste
- persistent command history across sessions with per-session append-only log files
- inline prompt UI that tries to preserve normal terminal scrollback and shell behavior
- tmux-driven end-to-end regression coverage for prompt and terminal behavior

Out of scope today:

- rich notebook output such as images, HTML, widgets, or LaTeX rendering
- completion, inspection, or a full IPython parity feature set

## Usage

Run a local kernel:

```bash
cargo run -- run
```

Attach to an existing kernel:

```bash
cargo run -- attach --connection-file /path/to/kernel-connection.json
```

The default Python executable for `run` is `python3`.

## Interaction Model

Important bindings:

- `Enter`: submit the current cell
- `Shift-Enter`: insert a newline
- `Ctrl-C`: clear input when idle, interrupt the kernel when busy
- `Ctrl-D`: exit when the current input is empty
- `Ctrl-L`: clear the visible screen
- `Ctrl-P`: open the command palette
- `Ctrl-R`: fuzzy-search persistent history and load a previous cell into the editor
- `Up` / `Down`: persistent history recall when editing a single-line cell

The editor is powered by `edtui`, so normal/visual/insert mode behavior is Vim-like rather than shell-like. The input area uses line numbers on the left, a mode/status row below the editor, and grows inline until it reaches the bottom of the terminal, after which the editor scrolls internally.

## Development

Useful commands:

```bash
cargo test
cargo clippy --all-targets --all-features
cargo run -- run
```

For real terminal behavior, use tmux-based testing. It is much more reliable than judging layout from a captured non-interactive PTY.

Low-level repro harness:

- [`scripts/fpy-tmux-repro.sh`](scripts/fpy-tmux-repro.sh)

Rust integration tests:

```bash
cargo test --test tmux_e2e -- --nocapture
```

Startup benchmark:

```bash
scripts/benchmark-startup.sh
```

That benchmark uses `tmux` to build `fpy` in release mode and measure:

- time to usable input
- time to safe submission
- time to the first successful `1+1` result

for both `target/release/fpy run --python ...` and plain `ipython`.

Examples:

```bash
scripts/fpy-tmux-repro.sh ctrl-d
scripts/fpy-tmux-repro.sh vim-open-below
PRE_INPUT='' INPUTS='' PASTE_TEXT='x = 1
y = 2' scripts/fpy-tmux-repro.sh paste
scripts/fpy-tmux-visual-repro.sh startup-anchor
scripts/fpy-tmux-visual-repro.sh footer-styling
```

The `tmux` e2e suite currently covers:

- transcript preservation on `Ctrl-D`
- shell recovery after `exit()`
- multiline prompt growth at the bottom of the terminal
- bottom-of-screen result visibility
- multiline paste rendering
- `Shift-Enter` multiline editing
- first `Esc-o` growth in Vim normal mode
- history recall with `Up`

## Architecture

High-level module layout:

- [`src/app.rs`](src/app.rs): top-level async event loop and UI/kernel coordination
- [`src/kernel/`](src/kernel): kernel lifecycle, transport runtime, message decoding, diagnostics
- [`src/ui/display.rs`](src/ui/display.rs): canonical display/transcript model, frame renderer, cursor state, display fixtures
- [`src/ui/components/`](src/ui/components): component renderers for transcript, editor, footer, and overlays
- [`src/ui/backend/`](src/ui/backend): recording backend and differential normal-terminal crossterm backend
- [`src/ui/session.rs`](src/ui/session.rs): raw mode, bracketed paste, keyboard protocol setup, and exit cleanup
- [`src/ui/`](src/ui): UI state machine, input handling, editor integration, and transcript formatting
- [`src/jupyter.rs`](src/jupyter.rs): Jupyter wire-message encoding, signing, and decoding

The most important design constraint is that `fpy` does not want a fullscreen alternate-screen TUI. The canonical transcript/display state lives in `fpy`, while the terminal remains a normal main-screen render target with semantically faithful scrollback and shell recovery on exit.
