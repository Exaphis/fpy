# Cell-attributed async output plan

## Problem

`fpy` currently appends `stream`, `execute_result`, and `error` IOPub messages to the end of the transcript in arrival order. This makes background output from older cells appear below newer prompts/cells, for example two daemon threads printing concurrently produce an interleaved global stream:

```text
In [2]: print_1_to_60_every_second()
1
Out[2]: <Thread(...)>
[20.0ms]
2
3
4
5
In [3]: print_1_to_60_every_second()
1
Out[3]: <Thread(...)>
[16.0ms]
6
2
7
3
```

Jupyter messages usually carry `parent_header.msg_id`, which should identify the `execute_request` that caused the output. `fpy` should use this metadata when available so late output can be associated with the originating cell instead of only with the current transcript tail.

## Current code path

Relevant files:

- `src/jupyter.rs`
  - `WireMessage` already stores `header`, `parent_header`, `metadata`, and `content`.
  - `MessageCodec::message()` generates an `execute_request` `header.msg_id`, but that id is not exposed to the app/UI.
- `src/kernel/runtime.rs`
  - The command task creates and sends `execute_request` messages.
  - Receive loops decode shell/IOPub/stdin messages and immediately map them into `KernelEvent`s.
- `src/kernel/messages.rs`
  - `iopub_message_to_events()` maps `execute_input`, `stream`, `execute_result`, `display_data`, `update_display_data`, and `error` without preserving `parent_header.msg_id`.
- `src/kernel/mod.rs`
  - `KernelEvent` variants do not include a parent message id or a stable cell id.
- `src/app.rs`
  - `handle_kernel_event()` forwards output events directly to `AppUi`.
- `src/ui/mod.rs` / `src/ui/display.rs`
  - `TranscriptModel` is a flat `Vec<TranscriptEntry>`.
  - `push_stream()` only merges with the immediately previous stream entry of the same stream name.
  - There is no cell/output grouping or insertion by parent cell.

## Desired behavior

When an IOPub output message has a known `parent_header.msg_id`:

1. Find the transcript cell created for that `execute_request`.
2. Insert or append the output within that cell's output region, even if newer cells/prompts already exist.
3. Preserve chronological order among outputs from the same parent.
4. Preserve a safe fallback: if attribution is missing or unknown, append at the transcript tail as today.

For the thread example, output from the first thread should continue to appear with `In [2]`, while output from the second thread appears with `In [3]`.

## Important design choices

### Track request ids from the kernel layer

Add a small typed id, for example:

```rust
pub struct ParentMessageId(pub String);
```

or use `String` directly at first.

Kernel events that can be attributed should carry it:

- `ExecuteInput { parent_msg_id, execution_count, code }`
- `Stream { parent_msg_id, name, text }`
- `ExecuteResult { parent_msg_id, execution_count, text }`
- `Error { parent_msg_id, traceback }`
- eventually `DisplayData` / `UpdateDisplayData` separately if richer display support is added

Extract it in `src/kernel/messages.rs` from:

```rust
message.parent_header.get("msg_id").and_then(Value::as_str)
```

Keep it as `Option<String>` because not all messages are guaranteed to have a useful parent.

### Correlate submitted requests and `execute_input`

`KernelSession::execute()` currently returns `Result<()>`. The command loop generates the actual `execute_request` id internally, so app/UI code cannot know it at submit time.

Recommended first pass: create cells on `execute_input`, not on local submit. `execute_input` should include the parent id from the kernel's IOPub message. This matches the current UI behavior and avoids changing command send acknowledgement semantics.

Later improvement: have `execute()` return/enqueue a locally generated request id so fpy can create a pending cell immediately, then reconcile with `execute_input`.

### Represent cells in the transcript model

The current flat transcript makes retroactive insertion possible but awkward: fpy must find an `Input` entry and insert output before the next `Input` entry. A more explicit model is preferable.

Minimal incremental approach:

- Add `parent_msg_id: Option<String>` to `InputEntry`, `StreamEntry`, `OutputEntry`, and `ErrorEntry`.
- Add `TranscriptModel` helpers:
  - `push_input(parent_msg_id, execution_count, code)`
  - `push_stream_for_parent(parent_msg_id, name, text)`
  - `push_execute_result_for_parent(parent_msg_id, execution_count, mime)`
  - `push_error_for_parent(parent_msg_id, traceback)`
- For attributed output, find the cell range:
  - start: matching `InputEntry.parent_msg_id`
  - end: next `Input` after start, or transcript end
- Insert/merge the output just before `end`.
- For unattributed output or unknown parent, use existing tail append behavior.

Longer-term approach:

- Introduce `TranscriptEntry::Cell(CellEntry { parent_msg_id, execution_count, input, outputs, timing })`.
- Render cells by flattening at component-render time.
- This is cleaner, but larger and likely should follow the minimal implementation once behavior is proven.

### Stream coalescing rules

Current `push_stream()` coalesces only with the last transcript entry. With attribution, coalescing should be scoped to the parent cell:

- If the last output in the target cell is a `Stream` with the same stream name and parent id, append text to it.
- Otherwise insert a new `Stream` at the end of that cell's output region.

Do not merge stdout/stderr across different parent ids even when they are adjacent in global arrival order.

### Rendering/backend impact

Retroactive insertion changes rows above the current viewport tail. The Pi-style backend already treats changed rows above the previous viewport / non-tail growth as unsafe and can recover with a full redraw. That is acceptable for a first implementation.

Potential follow-up optimization: once cell-attributed output is common, improve diffing for in-place growth inside visible history. Correctness should come first.

## Implementation phases

### Phase 1: empirical check and tests

1. Add a small unit test in `src/kernel/messages.rs` proving `parent_header.msg_id` is extracted into stream/output/error events.
2. Add a display/transcript unit test showing two inputs with distinct parent ids and late streams route to the correct cell ranges.
3. Add a tmux/e2e repro if practical:
   - define the thread-printing function
   - start two threads from separate cells
   - assert later output is displayed under the originating cells, not only at the tail

Before implementation, optionally log or fixture real ipykernel IOPub messages for threaded stdout to confirm whether ipykernel preserves distinct parent ids for background threads in the target version.

### Phase 2: preserve parent ids in kernel events

1. Update `KernelEvent` variants with `parent_msg_id: Option<String>`.
2. Add a helper in `src/kernel/messages.rs` to extract parent ids.
3. Update existing message tests and all `handle_kernel_event()` matches.
4. Keep behavior unchanged initially by ignoring the id in UI calls if desired.

### Phase 3: transcript attribution helpers

1. Add `parent_msg_id` fields to relevant display entries.
2. Update serialization defaults with `#[serde(default)]` if needed for fixture compatibility.
3. Implement parent-aware insertion helpers in `TranscriptModel`.
4. Preserve old `push_*` methods as wrappers for fallback/unattributed output if useful.

### Phase 4: wire app/UI behavior

1. Pass parent ids from `KernelEvent` through `AppUi` methods.
2. Use parent-aware transcript insertion for streams, execute results, and errors.
3. Continue using tail append for pager/info/warning/fatal/system messages.
4. Confirm runtime lines remain associated with the active execution. A later cleanup may store runtime as cell metadata instead of a system line.

### Phase 5: polish and edge cases

- Unknown parent id: append at tail and optionally emit debug logging only.
- Output before `execute_input`: either buffer briefly by parent id or append at tail. Buffering can be a follow-up if real kernels exhibit this.
- Multiple frontends attached to one kernel: parent ids may refer to requests fpy did not create. Treat as unknown unless an input is present.
- `display_id` updates: currently mapped to `ExecuteResult`; true update-in-place support should be handled separately from parent attribution.
- Stdin prompts: `input_request` uses parent headers too, but routing interactive stdin into old cells is more complex and not required for the thread-printing case.

## Acceptance criteria

- Existing `cargo test` and `cargo clippy --all-targets --all-features` pass.
- Existing tmux regressions continue to pass.
- A reproducer with two background threads shows each thread's later `stdout` under its originating `In [...]` block when ipykernel provides distinct parent ids.
- If parent ids are absent, fpy behaves no worse than today.
