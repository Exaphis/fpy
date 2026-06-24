# Frontend previous-cell stdout magics

## Goal

Implement fpy-owned magics that can retrieve stdout produced by a previous executed cell. This is something normal IPython cannot do reliably because previous cell output is owned by the frontend transcript, not by the kernel namespace.

Initial scope: **stdout only**. Ignore stderr-specific and combined stdout/stderr variants for now.

Useful initial commands:

- `%fpy_out` — print stdout from the previous executed cell into the transcript.
- `%fpy_out N` — print stdout from execution count `N`.
- `%fpy_out -K` — print stdout from the Kth previous executed cell, where `-1` is previous.
- `%fpy_clipout` / `%fpy_clipout N` / `%fpy_clipout -K` — copy the same stdout text to the system clipboard and print a short confirmation.

Do **not** send these commands to ipykernel. They are frontend commands parsed and handled by fpy before `KernelSession::execute`.

## Current relevant architecture

- `src/app.rs`
  - Top-level event/action loop.
  - `handle_ready_ui_action` receives `UiAction::Submit(code)` and currently always sends submitted code to `kernel.execute(code)`.
  - `handle_kernel_event` receives decoded `KernelEvent`s and inserts them into the UI.

- `src/kernel/mod.rs`
  - Defines `KernelEvent`.
  - Relevant variants:
    - `ExecuteInput { execution_count, code }`
    - `Stream { name, text }`
    - `ExecuteResult { execution_count, text }`
    - `Error { traceback }`

- `src/kernel/messages.rs`
  - Maps Jupyter IOPub `stream` messages to `KernelEvent::Stream { name, text }`.

- `src/ui/mod.rs`
  - `AppUi::insert_execute_input`
  - `AppUi::insert_stream`
  - `AppUi::insert_transcript`

The Jupyter protocol associates `execute_input` with an `execution_count`, but `stream` messages themselves do not include the count. fpy therefore needs to maintain a small capture state while cells execute.

## Proposed design

Add an app-level stdout capture store. Keep this outside the display renderer; the renderer/transcript should remain focused on terminal presentation.

Suggested new module:

```text
src/frontend_magic.rs
```

or, if preferred, split later into:

```text
src/frontend_magic/mod.rs
src/frontend_magic/capture.rs
src/frontend_magic/parse.rs
src/frontend_magic/clipboard.rs
```

### Capture model

Add structs similar to:

```rust
#[derive(Debug, Default)]
pub(crate) struct OutputCaptureStore {
    active_execution_count: Option<u32>,
    cells: Vec<CellStdoutCapture>,
}

#[derive(Debug, Clone)]
pub(crate) struct CellStdoutCapture {
    pub execution_count: u32,
    pub code: String,
    pub stdout: String,
}
```

Behavior:

1. On `KernelEvent::ExecuteInput { execution_count: Some(n), code }`:
   - Set `active_execution_count = Some(n)`.
   - Push a new `CellStdoutCapture { execution_count: n, code, stdout: String::new() }`.
   - Allow duplicate execution counts. Kernel restarts can reset execution counts while the visible transcript and old captures remain. Positive execution-count lookups should resolve to the most recent matching capture.

2. On `KernelEvent::Stream { name: Stdout, text }`:
   - If `active_execution_count` is `Some(n)`, append `text` to that cell's `stdout`.
   - If no active count exists, ignore for this feature but still render normally.

3. On `KernelEvent::Status(Idle)`:
   - Clear `active_execution_count`.

4. On restart/disconnect:
   - Prefer keeping old captures, because transcript remains visible.
   - Do not deduplicate by execution count across restarts. If the kernel later produces another `In [1]`, keep both captures and resolve `%fpy_out 1` to the most recent matching `In [1]`.
   - If future UX suggests otherwise, clear captures only on explicit screen/history clear.

This captures stdout in the order fpy receives stream messages for a cell. It intentionally ignores execute result display text and errors in the first implementation.

### Resolving target cells

Support these target forms:

- Missing argument: previous executed cell.
- Positive integer `N`: execution count `N`.
- Negative integer `-K`: relative to the end of captured cells; `-1` means previous captured cell, `-2` means one before that.

Suggested API:

```rust
pub(crate) struct ResolvedStdout<'a> {
    pub(crate) execution_count: u32,
    pub(crate) stdout: &'a str,
}

impl OutputCaptureStore {
    pub(crate) fn begin_cell(&mut self, execution_count: Option<u32>, code: &str);
    pub(crate) fn append_stream(&mut self, name: StreamName, text: &str);
    pub(crate) fn finish_active(&mut self);
    pub(crate) fn resolve_stdout(&self, target: OutputTarget) -> Result<ResolvedStdout<'_>>;
}
```

`resolve_stdout(OutputTarget::ExecutionCount(n))` should search from the end of `cells` and return the most recent capture with execution count `n`. This avoids ambiguity after kernel restarts reset execution counts.

When a target cannot be resolved, insert a user-facing transcript line such as:

```text
fpy: no captured stdout for previous cell
fpy: no captured stdout for In [12]
```

If the cell exists but stdout is empty, say:

```text
fpy: In [12] produced no stdout
```

## Frontend magic parsing

Intercept only simple single-line commands submitted as the entire cell. Examples:

```text
%fpy_out
%fpy_out 12
%fpy_out -1
%fpy_clipout
%fpy_clipout 12
```

Do not try to parse these embedded in a larger Python cell. If a user writes additional Python code, send it to the kernel as usual.

Suggested enums:

```rust
pub(crate) enum FrontendMagicParse {
    NotFrontendMagic,
    Magic(FrontendMagic),
    Error(String),
}

pub(crate) enum FrontendMagic {
    PrintStdout { target: OutputTarget },
    ClipStdout { target: OutputTarget },
}

pub(crate) enum OutputTarget {
    Previous,
    ExecutionCount(u32),
    RelativePrevious(usize),
}
```

Use three parser outcomes rather than `Option<FrontendMagic>` so fpy can distinguish unknown/kernel-owned magics from malformed fpy-owned commands.

Parser rules:

- Trim leading/trailing whitespace.
- Return `NotFrontendMagic` if the trimmed text contains `\n`.
- Split on ASCII whitespace.
- Command must be exactly `%fpy_out` or `%fpy_clipout`; unknown `%` commands return `NotFrontendMagic` and are sent to the kernel normally.
- Allow zero or one argument only.
- Invalid arguments or too many arguments for `%fpy_out` / `%fpy_clipout` return `Error(...)` and should be handled by fpy with a transcript error, not sent to the kernel.

## App integration

In `src/app.rs`:

1. Add an `OutputCaptureStore` near the other app state.
2. Pass it into `handle_kernel_event` and update it alongside UI rendering:
   - `ExecuteInput` -> `captures.begin_cell(...)`
   - `Stream` -> `captures.append_stream(...)`
   - `Status(Idle)` / fatal / disconnect -> `captures.finish_active()`
3. In `handle_ready_ui_action`, before recording history or calling `kernel.execute`, check whether `code` is a frontend magic.

Important: frontend magics should not create kernel history entries and should not set the kernel status to busy.

Pseudo-flow:

```rust
UiAction::Submit(code) => {
    match parse_frontend_magic(&code) {
        FrontendMagicParse::Magic(magic) => {
            handle_frontend_magic(ui, &captures, magic)?;
            return Ok(false);
        }
        FrontendMagicParse::Error(message) => {
            ui.insert_transcript(&format!("fpy: {message}\n"));
            return Ok(false);
        }
        FrontendMagicParse::NotFrontendMagic => {}
    }

    // existing path: record history, send to kernel, set busy
}
```

For `%fpy_out`, insert the captured stdout into the transcript with `ui.insert_transcript(stdout)`.

For `%fpy_clipout`, copy to clipboard and insert a confirmation:

```text
fpy: copied stdout from In [12] to clipboard
```

## Clipboard implementation

Keep clipboard support small and platform-friendly. The existing user magic uses:

- macOS: `pbcopy`
- Linux: `xclip -selection clipboard`

For fpy, implement a helper that tries common tools:

- macOS: `pbcopy`
- Linux: try `wl-copy`, then `xclip -selection clipboard`, then `xsel --clipboard --input`

Return a clear error if no tool works:

```text
fpy: clipboard unavailable: install wl-copy, xclip, or xsel
```

Use `std::process::Command` with piped stdin. This can be synchronous; clipboard writes are small. If this ever blocks visibly, move it to a blocking task later.

## Tests

Add focused Rust unit tests for the new module:

- parser accepts `%fpy_out`, `%fpy_out 3`, `%fpy_out -1`, `%fpy_clipout`.
- parser rejects multiline cells and unknown `%` commands by returning `None`.
- parser reports invalid `%fpy_out abc` / too many args as a frontend magic parse error.
- capture store records stdout for execution count 1.
- capture store appends multiple stdout stream chunks.
- capture store ignores stderr.
- positive and negative target resolution work.
- positive execution-count resolution returns the most recent matching capture when duplicate counts exist.
- empty stdout is distinguishable from missing cell.

Add an app-level or tmux e2e test if practical:

1. Run `print('hello')`.
2. Run `%fpy_out -1`.
3. Assert `hello` appears again in the capture.

Clipboard e2e is optional because CI/dev environments may not have clipboard utilities. Unit-test clipboard command selection if it is factored cleanly, but avoid requiring a real system clipboard.

## Non-goals for first pass

- Capturing stderr.
- Combined stdout/stderr with interleaving.
- Capturing rich display output or execute result text.
- Exporting a Python object into the kernel.
- Registering real IPython magics inside the kernel.
- Supporting frontend magic commands embedded in larger Python cells.

## Naming notes

Use the `%fpy_` prefix initially to avoid clobbering user-defined IPython magics such as `~/.ipython/profile_default/startup/01-magics.py`'s `%clipout`. Aliases like `%out` or `%clipout` can be considered later once collision behavior is explicit.
