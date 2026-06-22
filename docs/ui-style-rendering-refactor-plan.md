# UI style and ANSI rendering refactor plan

## Goal

Reduce styling and ANSI-rendering drift across live editor rows, transcript rows,
history search, overlays, and footer/status UI.

`fpy` currently has several valid but separate render paths:

- live editor rows consume `edtui` render-plan spans and convert `ratatui::Style` to ANSI;
- submitted transcript and history preview rows use syntect ANSI helpers;
- prompt and runtime styling use hand-written ANSI strings;
- footer styling is assembled from semantic status text and converted directly to ANSI;
- width/wrapping utilities operate on ANSI strings after styling has already happened.

This works, but it makes visual parity bugs easy to introduce. The recent prompt
and syntax-highlighting fixes reduced some duplication, but the next step should
make style ownership explicit instead of scattering ANSI literals and style
conversion rules through renderers.

## Non-goals

- Do not change the normal-terminal display model or backend semantics.
- Do not replace `edtui` or rewrite editor rendering.
- Do not make footer `In [x]` share input-prompt styling; footer prompt text is
  intentionally status text.
- Do not change live multiline editor continuation rows to transcript `...:`
  continuation rows; that difference is intentional.
- Do not introduce a full rich-text layout engine unless the smaller model below
  proves insufficient.

## Proposed model

Add a small shared style layer, probably `src/ui/style.rs` or `src/ui/ansi.rs`.

The layer should define semantic app styles first:

```rust
pub enum UiStyle {
    InputPrompt,
    OutputPrompt,
    Runtime,
    FooterHint,
    ModeInsert,
    ModeNormal,
    ModeVisual,
    ModeSearch,
    Selection,
    Plain,
}
```

Semantic styles are useful for app-authored UI, but they are too narrow for
syntax highlighting and `edtui` render-plan spans. The styled text model should
therefore support both semantic styles and arbitrary concrete styles.

One reasonable shape:

```rust
pub enum TextStyle {
    Semantic(UiStyle),
    Raw(ratatui::style::Style),
}

pub struct StyledSegment {
    pub text: String,
    pub style: TextStyle,
}

pub struct StyledLine {
    pub segments: Vec<StyledSegment>,
}
```

Another acceptable shape is to normalize everything to an owned concrete style:

```rust
pub struct UiTextStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub modifiers: Modifier,
}
```

In that model, semantic styles map to `UiTextStyle`, and syntect / `edtui` spans
also map to `UiTextStyle`. The exact shape can be decided during implementation,
but the key requirement is that syntax highlighting must not remain a permanent
special case.

The exact names are not important. The important property is that component
renderers can express "this is an output prompt" or "this is footer hint text"
without hand-writing escape sequences.

## Raw ANSI boundary

The style model is for app-authored styled text. Kernel stream output, tracebacks,
and other foreign text may already contain ANSI escapes from user code or Python
libraries. That text should remain a raw ANSI path.

Keep a clear distinction between:

- app-authored styled text, which can be represented as `StyledLine`;
- raw/foreign ANSI text, which should continue to use ANSI-aware helpers such as
  `wrap_ansi_to_width()`.

`wrap_ansi_to_width()` is likely to remain necessary indefinitely for kernel
output, even if most app-authored UI moves to styled segments.

## Conversion responsibilities

The shared layer should own these conversions:

1. `UiStyle -> ratatui::Style`
2. `ratatui::Style -> ANSI SGR`
3. `StyledLine -> ANSI String`
4. `StyledLine -> visible width`
5. `StyledLine -> wrapped StyledLine rows`

Initially, wrapping can still lower to ANSI and use the existing
`wrap_ansi_to_width()` if that keeps the change smaller. The end state should be
wrapping styled segments before ANSI conversion, because that avoids having
wrapping logic parse escape sequences it just emitted.

The shared layer should render whole lines or segment sequences, not just expose
style prefixes. Correct reset behavior is part of rendering, not an optional
caller responsibility.

## Migration plan

### 1. Extract style conversion

Move the current `style_prefix()` / `ansi_color()` logic out of
`src/ui/components/mod.rs` into the shared style module.

Keep behavior identical at first. This is a mechanical move with unit tests for:

- basic foreground colors;
- background colors;
- RGB colors;
- reset handling;
- bold and italic modifiers.

### 2. Move semantic ANSI constants

Move prompt, runtime, footer-hint, and mode-badge styling into semantic helpers.

Current examples that should stop owning raw ANSI directly:

- input prompt color;
- output prompt color;
- runtime line color;
- footer `Ctrl-P palette` dim text;
- insert/normal/visual/search mode badge colors.

Tests should assert semantic intent and rendered ANSI, for example:

- `UiStyle::InputPrompt` renders cyan;
- `UiStyle::OutputPrompt` renders red;
- `UiStyle::FooterHint` renders dark gray;
- `UiStyle::ModeInsert` renders black-on-cyan.

### 3. Introduce prompt helpers as styled segments

Introduce or keep `src/ui/prompt.rs` as the semantic owner of prompt text, and
have it return or expose styled forms through the style layer.

Examples:

- `input_prompt(Some(3)) -> "In [3]: "`
- `styled_input_prompt(...) -> StyledLine` or `StyledSegment`
- `output_prompt(Some(3)) -> "Out[3]"`
- `styled_output_prompt(...) -> StyledLine` or `StyledSegment`

Transcript and editor rendering should use the same styled prompt source, then
lower to ANSI only at the component/frame boundary.

### 4. Normalize syntax-highlight output

The shared syntax module currently makes editor, transcript, and history search
use the same syntect theme database and theme name. The next improvement is to
make syntax highlighting return styled segments instead of prebuilt ANSI strings
where practical.

There are two reasonable paths:

- Short path: keep using syntect ANSI for transcript/history, but parse or
  compare only through tests. This is smaller but leaves two conversion systems.
- Better path: add a helper that converts syntect ranges to `StyledSegment`s
  with RGB styles, and have both transcript/history and any future non-`edtui`
  code lower those through the shared style module.

The editor can keep consuming `edtui` render plans. Its `ratatui::Span`s should
be converted to ANSI through the same shared `ratatui::Style -> ANSI` function.

### 5. Move wrapping toward styled lines

The current `wrap_ansi_to_width()` is useful and well-tested, but it is working
after ANSI emission. Once enough renderers produce `StyledLine`, add styled-line
wrapping:

```rust
fn wrap_styled_line(line: &StyledLine, width: u16) -> Vec<StyledLine>;
```

This should preserve active styles across wrapped rows without replaying raw SGR
strings manually. Keep `wrap_ansi_to_width()` during migration for stream/error
text that already arrives as raw ANSI.

Wrapping should use terminal display width. Prefer grapheme clusters over bare
chars if practical, so combining marks and emoji sequences are not split in the
middle. This is not a blocker for the initial extraction, but it should be
tracked before making styled-line wrapping the only app-authored wrapping path.

### 6. Update component contracts only if useful

Do not immediately change the `Component` trait unless the migration starts
fighting the current `Vec<RenderedLine>` shape.

Current:

```rust
fn render(&mut self, width: u16) -> Vec<RenderedLine>;
```

Potential later shape:

```rust
pub struct RenderedLine {
    pub line: StyledLine,
    pub cursor_marker: Option<CursorMarker>,
}
```

Only make that change after transcript, footer, and overlay rendering can all
produce styled lines cleanly. Until then, local helper functions are enough.

## Test plan

Use unit tests for style conversion and pure rendering:

- `UiStyle` to ANSI output;
- `ratatui::Style` to ANSI output;
- rendering a whole `StyledLine`, not just style prefixes;
- prompt styling;
- output prompt styling;
- footer mode badge styling;
- syntax-highlight color parity for `time.sleep(1)`;
- wrapping preserves visible width and style boundaries.

Reset semantics need explicit tests:

- foreground reset;
- background reset;
- modifier reset;
- adjacent segments with different styles;
- styled segment followed by plain segment;
- line-end reset to prevent style bleed into following terminal content.

Use display/backend tests for frame-level behavior:

- cursor positions still use visible width, not ANSI byte length;
- cursor position with a styled prompt before editor text;
- cursor position after ANSI-styled editor text before the cursor;
- clipped frames still position the cursor correctly;
- rendered rows remain within terminal width after ANSI stripping.

Use tmux e2e only where real terminal behavior matters:

- live prompt color appears in ANSI capture;
- bottom-pinned prompt and footer still repaint cleanly;
- exit cleanup and second invocation remain sane.

## Suggested order

1. Extract `style_prefix()` / `ansi_color()` into a shared style module.
2. Move raw ANSI constants for prompt/runtime/footer/mode badges into semantic
   style helpers.
3. Add `StyledLine -> ANSI`, `ratatui::Style -> ANSI`, and
   `UiStyle -> ratatui::Style`.
4. Convert prompt helpers to use semantic styles.
5. Convert footer rendering to semantic styles.
6. Convert transcript output prompt/runtime rendering to semantic styles.
7. Keep raw kernel ANSI on the existing ANSI-aware path.
8. Add styled-line wrapping for new structured render paths.
9. Decide whether syntax highlighting should return styled segments or remain
   ANSI-based behind a small adapter.

## Success criteria

- Theme and prompt styling decisions live in one place.
- Adding or changing a semantic UI color does not require editing transcript,
  editor, footer, and overlay renderers independently.
- Live editor and submitted transcript syntax colors remain visually consistent.
- Cursor placement and row wrapping are based on visible width, not ANSI bytes.
- Raw ANSI emitted by user/kernel output remains supported.
- Styled app-authored output resets styles so terminal state does not bleed.
- Existing tmux terminal behavior remains stable.
