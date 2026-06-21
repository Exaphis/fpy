# Pi TUI research and fpy display refactor plan

This document records what pi's TUI actually does, based on the installed `@earendil-works/pi-tui` docs and implementation, and then proposes which ideas are worth borrowing for `fpy`.

Implementation status: the refactor has moved past the migration phase described below. `fpy` now uses app-owned display state, structured transcript entries, component rendering, display fixtures, and a differential normal-terminal frame backend. The old `custom_terminal.rs` / `insert_history` display path has been removed from the crate.

Relevant pi files researched:

- `/Users/kevin/n/lib/node_modules/@earendil-works/pi-coding-agent/docs/tui.md`
- `/Users/kevin/n/lib/node_modules/@earendil-works/pi-coding-agent/node_modules/@earendil-works/pi-tui/dist/tui.js`
- `/Users/kevin/n/lib/node_modules/@earendil-works/pi-coding-agent/node_modules/@earendil-works/pi-tui/dist/terminal.js`
- `/Users/kevin/n/lib/node_modules/@earendil-works/pi-coding-agent/node_modules/@earendil-works/pi-tui/dist/components/*.js`

## Core product decision

This refactor intentionally changes the display model:

> Old model: terminal scrollback is the committed transcript, and `fpy` maintains a mutable prompt below it.
>
> New model: `fpy` owns the canonical transcript/display state and renders it into the normal terminal. Terminal scrollback remains a semantic transcript of committed cell inputs/outputs, but the canonical source of truth is `fpy` state.

Behavior changes and non-goals:

- `fpy` should still run in the normal terminal main screen, not the alternate screen.
- Normal terminal scrollback must remain semantically faithful to committed transcript content: no duplicated, missing, stale, reordered, or mangled cell inputs/outputs.
- Byte-for-byte historical screen layout is not required. Resize/reflow/redraw may change wrapping, styling, or visible layout.
- Scrollback fidelity means transcript-content fidelity, not preserving every intermediate render state or stream chunk exactly.
- Crash recovery should come from owned transcript/session state where possible, not from assuming the terminal contains the only transcript copy.

This model also opens the door to session resumption. The refactor should preserve structured transcript entries so a future `fpy --resume` can restore the visible transcript and input history. If the original Jupyter kernel is still alive, `fpy` could also reconnect to it and preserve real Python state. Restoring arbitrary Python state after kernel death is out of scope; replaying prior cells into a fresh kernel can be a separate best-effort feature.

## Scope boundaries

This refactor should be milestone-gated, not a single all-at-once rewrite. Preserve the recognizable `fpy` experience, but allow intentional visual changes when they simplify the architecture or improve reliability.

In scope:

- app-owned transcript/display state;
- structured transcript entries;
- pure rendering to frames/snapshots;
- recording backend and scriptable display fixtures;
- pi-like differential main-screen backend;
- vendored `edtui` render-plan extraction;
- semantic scrollback fidelity tests.

Out of scope for this refactor:

- implementing session persistence/resume, though data structures should be designed for it;
- implementing live-kernel detach/reattach UX, though lifecycle design should avoid blocking it;
- restoring arbitrary Python state after kernel death;
- rich display rendering beyond current text-oriented behavior, though MIME bundles should be preserved;
- completions, though the new component/render architecture should make them easier later;
- exact visual parity when a deliberate simplification or reliability improvement is better.

## What pi's TUI actually is

Pi's TUI is a line-oriented, differential terminal renderer. It is not `ratatui`, and it is not the same model as `fpy`'s custom inline `ratatui` viewport.

### Component API

The public component shape is:

```ts
interface Component {
  render(width: number): string[];
  handleInput?(data: string): void;
  wantsKeyRelease?: boolean;
  invalidate(): void;
}
```

Important details confirmed in implementation:

- Components render to an array of strings, one terminal row per string.
- The width contract is component-owned: each line must fit in the supplied width.
- Containers compose by concatenating child line arrays vertically.
- Components often cache render output by width/content and clear it from `invalidate()`.
- Input goes to the currently focused component, then the TUI requests another render.
- Pi's TUI schedules renders with a minimum interval (`MIN_RENDER_INTERVAL_MS = 16`) rather than writing immediately on every state change.

### Terminal session and input

`ProcessTerminal` owns terminal setup:

- raw mode;
- bracketed paste (`CSI ? 2004 h/l`);
- stdin buffering to split batched escape sequences;
- Kitty keyboard protocol negotiation, with `modifyOtherKeys` fallback;
- resize handling;
- optional raw ANSI write logging via `PI_TUI_WRITE_LOG`.

This separation is worth copying: component/input logic should not also manage terminal modes and protocol negotiation.

### Rendering and diffing

`TUI.doRender()` roughly does this:

1. Get terminal width/height.
2. Render all base components to `newLines`.
3. Composite visible overlays into those lines.
4. Extract a special cursor marker from the visible viewport.
5. Append a full SGR reset and OSC 8 reset to each non-image line.
6. Compare `newLines` with `previousLines`.
7. Emit ANSI to update only changed regions when possible.
8. Fall back to full redraw for first render, width changes, most height changes, shrink clearing, or complex cases.

Pi keeps `previousLines`, `previousWidth`, `previousHeight`, `cursorRow`, `hardwareCursorRow`, `previousViewportTop`, and image bookkeeping. This is a real renderer with state, not just `println!(render())`.

Important caveat for `fpy`: pi sometimes uses full screen/scrollback clears (`CSI 2J`, home, `CSI 3J`) on full redraw. `fpy` should not blindly copy that recovery path. Pi is still the right model in the important sense: it renders in the normal terminal, owns canonical application history, and lets terminal scrollback arise from normal rendering. For `fpy`, the additional invariant is that scrollback must remain a semantically faithful transcript of committed cell input/output.

### Cursor model

Pi has a `CURSOR_MARKER` zero-width escape sequence. Focusable components render that marker at the logical cursor position. The TUI scans the bottom visible viewport for the marker, removes it, calculates visible column width before the marker, and positions the hardware cursor there.

This is a strong idea for `fpy`: cursor position should be render data, not a side effect spread across editor/pane drawing code.

### Focus and overlays

Pi tracks a focused component. Focusable components get `focused = true/false` set by the TUI. Overlays are kept in an overlay stack with explicit focus restore rules, visibility checks, sizing, anchors, margins, and compositing into the rendered base lines.

For `fpy`, this is useful for command palette/history search/input prompts, but it should be simplified. `fpy` does not need pi's full extension overlay machinery initially.

### Styling and width discipline

Pi appends `\x1b[0m\x1b]8;;\x07` at line boundaries. Utilities such as `visibleWidth`, `truncateToWidth`, `wrapTextWithAnsi`, `sliceByColumn`, and `sliceWithWidth` are central. Overlay compositing includes a final visible-width check and truncation safeguard.

This is directly relevant to `fpy`; many display bugs come from mismatches between logical rows, terminal rows, ANSI escapes, and wide characters.

## Is copying pi's approach a good idea?

Yes, with an explicit product-level shift: `fpy` should own the canonical display state more like pi does, while still rendering into the normal terminal rather than switching to an alternate-screen fullscreen app.

The revised direction is:

> `fpy` owns the logical transcript, editor, status, and overlay state. The terminal is a render target. Normal terminal scrollback should remain a faithful transcript of committed cell input/output, but `fpy` state is the canonical source of truth.

This is different from the current model, where committed terminal scrollback is treated as the transcript and the live editor is carefully inserted below it. That model is shell-like, but it creates many hard boundary bugs: output insertion above the prompt, bottom-pinned scroll regions, stale cells, wrapped-line accounting, and resize recovery.

Good pi ideas to adopt:

- canonical app state renders to terminal lines;
- component-style render contracts;
- render-to-lines before terminal mutation;
- differential rendering against a previous frame;
- explicit invalidation/request-render;
- cursor position derived from rendered output;
- one terminal/session boundary for raw mode, paste, key protocols, cursor visibility, and cleanup;
- deterministic recording backend for tests;
- ANSI-aware width/truncation/wrapping utilities used everywhere.

Ideas not to copy blindly:

- full redraws that clear scrollback as the default recovery strategy;
- pi's exact overlay complexity before `fpy` has simpler component boundaries;
- TypeScript string-line rendering as the only internal representation if spans/buffers remain useful;
- assuming terminal scrollback must preserve byte-for-byte historical wrapping/styling/intermediate render states after every redraw.

The main tradeoff is semantic: `fpy` becomes less like "kernel output was physically appended and is now the authoritative transcript" and more like "the transcript is in `fpy` state and the terminal shows a normal-terminal rendering of it." This is still compatible with faithful terminal scrollback: redraw/reflow may change layout, but the backend must not introduce duplicate, missing, stale, reordered, or mangled committed cell content.

Recommendation: this is a good idea if the project accepts that shift. It should be done incrementally, with tests that check both the logical transcript and the terminal projection. A wholesale rewrite of `custom_terminal.rs` + `insert_history` + editor rendering would still be risky; the first milestone should be a state model and recording renderer that can run next to the current implementation.

## Current fpy display shape

Current relevant files:

- `src/ui/mod.rs`: event loop, editor controller, input handling, palette/history search, terminal lifecycle, render orchestration, and many terminal side effects.
- `src/ui/render.rs`: pane geometry and status helpers.
- `src/custom_terminal.rs`: custom double-buffered `ratatui` terminal that renders into an inline viewport instead of alternate screen.
- `src/insert_history/mod.rs`: writes transcript text above the pane, including bottom-pinned scroll-region handling.
- `src/ui/editor.rs`: `edtui` integration and editor visual helpers.
- `src/ui/transcript.rs`: transcript formatting and highlighted input echoes.

The refactor should preserve existing tmux repro scripts and `tests/tmux_e2e.rs`, while adding lower-level fixtures that can inspect both visible rows and scrollback rows.

## Proposed target architecture

### 1. Own canonical display state

Make the app state explicit and authoritative:

```rust
pub struct DisplayModel {
    pub transcript: TranscriptModel,
    pub editor: EditorModel,
    pub kernel_status: KernelStatus,
    pub overlays: OverlayStack,
    pub footer: FooterModel,
}

pub struct TerminalFrame {
    pub size: Size,
    pub full_rows: Vec<String>,
    pub cursor: CursorState,
}

// If string-only diffing is insufficient for backend classification,
// promote rows to a metadata-carrying representation:
pub struct TerminalRow {
    pub text: String,
    pub kind: RowKind, // committed transcript vs live UI, etc.
}

pub struct TestSnapshot {
    pub full_rows: Vec<String>,
    pub visible_rows: Vec<String>,
    pub expected_scrollback_rows: Vec<String>,
    pub cursor: CursorState,
}
```

`TerminalFrame` is the production render output. `TestSnapshot` is backend/test output, not core app state. The initial frame can use string rows, but backend classification may require row identity/metadata later; if so, promote `full_rows` from `Vec<String>` to a row type that marks committed transcript rows separately from live UI rows.

Definitions:

- `full_rows`: the complete idealized row rendering of the current `DisplayModel` for `size`. It includes both committed transcript rows and live UI rows such as editor, status, footer, and overlays.
- `visible_rows`: the rows currently visible for a given terminal height.
- `expected_scrollback_rows`: an idealized backend-level artifact describing what the recording backend expects normal terminal scrollback to contain after applying frames. It is not the canonical transcript, and a real terminal may differ in byte-level/layout details. It must still be semantically faithful to committed transcript entries. tmux tests remain the external validation for real terminal behavior.

The transcript should be retained as structured state, not only as already-written terminal rows. Use normalized transcript entries as the canonical model, not rendered rows and not raw Jupyter messages as the primary state. This is the least likely choice to be undone later: it preserves enough structure for rendering, resize/reflow, resume, replay, export, and richer displays without coupling display state to raw protocol details.

A future shape should look roughly like:

```rust
pub struct TranscriptModel {
    pub entries: Vec<TranscriptEntry>,
}

pub enum TranscriptEntry {
    Input(InputEntry),
    Stream(StreamEntry),
    ExecuteResult(OutputEntry),
    DisplayData(OutputEntry),
    Error(ErrorEntry),
    Stdin(StdinEntry),
    System(SystemEntry),
}
```

Implementation details can evolve, but the important rules are:

- store input cells as input entries, not only as highlighted `In [...]` text;
- store stream output with stream name (`stdout`/`stderr`) and coalesce adjacent chunks where safe;
- store rich output as MIME bundles even if the first renderer only uses `text/plain`;
- store errors structurally (`ename`, `evalue`, traceback lines);
- keep raw Jupyter messages only as optional debug/session-log data, not as the display model;
- make the model serde-friendly so session persistence can be added without reshaping the transcript.

That structure should include input cells, outputs, errors, execution counts, stream names, display data metadata, timestamps where useful, and enough kernel/session metadata to offer transcript resume or live-kernel reconnect later. The renderer derives terminal rows from that state. Tests should assert:

- the logical transcript entries;
- `full_rows`;
- `visible_rows`;
- `expected_scrollback_rows` where backend behavior matters;
- cursor position/style/visibility.

### 2. Add a component layer for the whole render tree

Add `src/ui/components/` with a Rust contract similar to pi's. Unlike the previous inline-pane-only design, components should eventually cover transcript entries, editor, status, footer, and overlays:

```rust
pub trait Component {
    fn render(&mut self, width: u16) -> Vec<RenderedLine>;
    fn handle_input(&mut self, input: UiInput) -> InputOutcome { InputOutcome::Ignored }
    fn invalidate(&mut self) {}
}

pub struct RenderedLine {
    pub text: String,
    pub cursor_marker: Option<CursorMarker>,
}

pub struct CursorState {
    pub position: Option<Position>,
    pub style: CursorStyle,
    pub visible: bool,
}
```

`render(&mut self)` permits cache bookkeeping, matching pi's component style. Render mutation should be limited to caches and layout metadata; semantic state changes should happen through input/actions. If this becomes hard to reason about, prefer `render(&self, ...)` plus explicit cache containers.

Line-local cursor markers are an intermediate representation. The final `TerminalFrame` owns the resolved cursor position, style, and visibility.

Initial components:

- transcript component that renders structured transcript entries;
- editor component wrapping `edtui`;
- transient status/throbber component;
- command palette/history search component;
- footer/status component;
- simple vertical stack/container.

The `edtui` integration is the riskiest component, but initial investigation suggests direct render-plan extraction is feasible. `EditorView::render()` already separates much of the hard work from `ratatui::Buffer`: it computes viewport offsets, generates styled spans, wraps spans, computes cursor position, and only then writes to a buffer. `fpy` also uses a narrow subset of `edtui` features today: no edtui block, no edtui status line, no edtui line numbers, `wrap(false)`, external fpy gutter, syntax highlighting, selection styling, and cursor position.

Preferred path:

1. Add an editor render-plan API inside vendored `edtui`.
2. Make the existing `ratatui::Widget` implementation render from that plan for parity.
3. Make `fpy` consume the same plan and convert it to `RenderedLine`s / frame rows.
4. Avoid a `ratatui::Buffer -> RenderedLine` bridge unless render-plan extraction proves unexpectedly difficult.

The render plan should be terminal-agnostic, not ANSI strings:

```rust
pub struct EditorRenderPlan {
    pub rows: Vec<EditorRenderRow>,
    pub cursor: Option<EditorRenderCursor>,
    pub viewport_offset: (usize, usize),
    pub screen_area: Rect,
}

pub struct EditorRenderRow {
    pub spans: Vec<EditorRenderSpan>,
}

pub struct EditorRenderSpan {
    pub content: String,
    pub style: Style,
}

pub struct EditorRenderCursor {
    pub row: u16,
    pub col: u16,
    pub style: Style,
}
```

The first render-plan implementation can target the current `fpy` subset and defer edtui block/status-line/line-number compatibility. It should reuse existing internals such as span generation, `RenderLine`, and `LineWrapper`. Keep the invariant: every rendered line's visible width is `<= width`.

### 3. Rendering pipeline

The intended pipeline is:

```text
DisplayModel
  -> RenderTree / Components
  -> TerminalFrame { size, full_rows, cursor }
  -> TerminalBackend
  -> optional TestSnapshot
```

Only `DisplayModel` is canonical app state. `TerminalFrame` is render output. `TestSnapshot` is a recording/testing artifact.

### 4. Split renderer/backend from UI state

Introduce a renderer/backend boundary along these lines:

```rust
pub trait TerminalBackend {
    fn size(&mut self) -> io::Result<Size>;
    fn draw_frame(&mut self, frame: TerminalFrame) -> io::Result<()>;
}

pub struct TerminalFrame {
    pub size: Size,
    pub full_rows: Vec<String>, // or Vec<TerminalRow> if row metadata is needed
    pub cursor: CursorState,
}
```

Implementations:

- `CrosstermMainScreenBackend`: renders frames in the normal terminal main screen, preserving useful terminal scrollback in normal operation where practical.
- `RecordingBackend`: virtual terminal with `full_rows`, `visible_rows`, and `expected_scrollback_rows` snapshots.

This is the most important testability boundary. It makes terminal contents a rendering of `DisplayModel`, not the model itself, while still allowing tests to enforce scrollback transcript fidelity.

`full_rows` includes both committed transcript rows and live UI rows. The backend must classify frame changes before writing:

- transcript growth: append the newly committed transcript content so it naturally enters normal terminal scrollback;
- editor/status/overlay-only changes: repaint visible live rows in place without appending old transcript as new output;
- resize/reflow: recompute rows for the new size and repaint visible rows without duplicating committed transcript content;
- recovery fallback: prefer conservative visible redraws that restore correctness without clearing scrollback or replaying old transcript as new content.

Backend implementation should go directly toward a pi-like full-row differential renderer:

1. Build the recording backend first so renderer behavior is observable.
2. Build `CrosstermMainScreenBackend` around `previous_full_rows -> new_full_rows` diffing in the normal terminal main screen.
3. Optimize for the common append path so ordinary transcript growth naturally enters terminal scrollback.
4. Use conservative full visible redraws as fallback for complex changes, resize, or recovery, but do not make a dumb full-redraw renderer the primary production model.

This is harder than a dumb full-redraw backend, but it avoids building a transitional renderer with the wrong semantics. The first implementation can still be conservative: append when append is clearly safe; otherwise repaint the visible region from state without appending historical transcript rows.

### 5. Replace scrollback insertion with frame rendering

The long-term goal is to retire `insert_history_text` as a core mechanism. Instead of surgically inserting transcript output above a mutable prompt, append kernel events to `TranscriptModel`, render a new full frame, and let the backend update the terminal.

During migration, keep the old insertion path as a legacy display driver while developing the new frame renderer. It may not implement the same backend trait cleanly at first. The new renderer should have pure tests for:

- growing transcript;
- prompt at bottom of terminal;
- long wrapped transcript rows;
- resize and reflow;
- output that fills or exceeds the visible window;
- redraw recovery from stale/corrupt visible cells.

Do not require byte-for-byte physical scrollback preservation across resize/reflow/redraw. Do require semantic transcript fidelity: committed cell content must appear once, in order, without stale fragments or corruption. Use the backend classification rules from section 4: actual transcript growth appends content, while editor changes, resize, and recovery repaint visible rows without appending old transcript as new output.

### 6. Add display fixtures for scripts

Add a small fixture binary or script mode, for example:

```bash
cargo run --bin fpy-display-fixture -- scenario bottom-pinned-output --width 80 --height 12
```

Output JSON:

```json
{
  "model": { "transcript_entries": 3 },
  "full_rows": ["..."],
  "visible_rows": ["..."],
  "expected_scrollback_rows": ["..."],
  "cursor": { "row": 10, "col": 4 }
}
```

Scenarios should not require a real kernel. They should drive the same `DisplayModel` and renderer used by production. Keep tmux tests as the final truth for actual terminal behavior.

### 7. Shrink `src/ui/mod.rs`

Move responsibilities out gradually:

- `ui/input.rs`: crossterm event to `UiInput`, paste normalization, keybindings.
- `ui/session.rs`: raw mode, bracketed paste, keyboard enhancement, cleanup. This should move early, before renderer replacement, because it is an immediate cleanup win and reduces risk when swapping backends.
- `ui/layout.rs`: pane geometry and viewport sizing.
- `ui/components/`: editor/status/palette.
- `ui/display.rs`: build full terminal frames from state/components.
- `ui/backend/`: main-screen crossterm renderer and recording backend.

`AppUi` should orchestrate state transitions and actions. It should not directly issue cursor moves, clears, or scroll-region commands.

## Migration plan

1. Add shared ANSI-aware width/truncation/wrapping helpers and tests.
2. Move terminal session setup/cleanup out of `AppUi`.
3. Introduce `DisplayModel` with structured transcript entries and editor/status state.
4. Add a pure renderer from `DisplayModel` to `TerminalFrame`.
5. Add `RecordingBackend` for `full_rows`, `visible_rows`, and `expected_scrollback_rows`.
6. Add JSON display fixture scenarios for known regressions.
7. Add vendored `edtui` render-plan extraction and make the existing widget render from the plan.
8. Wrap transcript/status/palette/editor rendering in components one at a time, with the editor consuming the new `edtui` render plan.
9. Make cursor position/style/visibility part of rendered frame data.
10. Build `CrosstermMainScreenBackend` as a pi-like full-row differential renderer in normal terminal mode.
11. Prioritize append-path correctness so transcript growth produces useful normal terminal scrollback where practical.
12. Keep the old `custom_terminal.rs`/`insert_history` path as a legacy display driver until the differential backend passes parity tests. It may not implement the same trait cleanly at first.
13. After tests pass, simplify `AppUi` and remove direct terminal writes.
14. Rerun `cargo test`, `cargo clippy --all-targets --all-features`, tmux repro scripts, and `cargo test --test tmux_e2e -- --nocapture` after each terminal-facing step.

## Acceptance criteria

- Fast tests can assert logical transcript entries, `full_rows`, `visible_rows`, `expected_scrollback_rows`, and cursor state without tmux.
- Existing tmux repros continue to pass or are intentionally updated to reflect the new app-owned-state semantics.
- No component returns a line wider than the available width.
- Resize/redraw behavior is tested without relying on the old prompt-above-output insertion model.
- Backend tests verify that append, edit, resize, and recovery sequences do not duplicate, drop, reorder, or mangle committed transcript content in expected scrollback.
- `AppUi` no longer mixes state transitions with low-level terminal mutation.
- Exit cleanup restores raw mode, bracketed paste, keyboard enhancement state, cursor visibility/style, and leaves the shell prompt in a sane place.
- `fpy` remains a normal-terminal app, not an alternate-screen fullscreen app.
- The canonical transcript lives in `fpy` state; terminal scrollback remains a semantically faithful normal-terminal rendering of committed transcript content.
