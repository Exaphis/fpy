use chrono::{DateTime, Utc};
use edtui::EditorState;
use ratatui::layout::{Position, Size};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::kernel::KernelStatus;

use super::{
    backend::{RecordingBackend, TerminalBackend},
    components::{
        Component, EditorComponent, FooterComponent, OverlayComponent, RenderedLine,
        TranscriptComponent,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct DisplayModel {
    pub transcript: TranscriptModel,
    pub editor: EditorModel,
    pub overlay: OverlayModel,
    pub kernel_status: DisplayKernelStatus,
    pub footer: FooterModel,
}

impl DisplayModel {
    pub(crate) fn new() -> Self {
        Self {
            transcript: TranscriptModel::default(),
            editor: EditorModel::default(),
            overlay: OverlayModel::None,
            kernel_status: DisplayKernelStatus::Connecting,
            footer: FooterModel::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TranscriptModel {
    pub entries: Vec<TranscriptEntry>,
    #[serde(default)]
    pub visible_start: usize,
}

impl TranscriptModel {
    pub(crate) fn clear_visible(&mut self) {
        self.visible_start = self.entries.len();
    }

    pub(crate) fn visible_entries(&self) -> &[TranscriptEntry] {
        &self.entries[self.visible_start.min(self.entries.len())..]
    }

    pub(crate) fn push_input(&mut self, execution_count: Option<u32>, code: impl Into<String>) {
        self.entries.push(TranscriptEntry::Input(InputEntry {
            execution_count,
            code: code.into(),
            timestamp: Some(Utc::now()),
        }));
    }

    pub(crate) fn push_system(&mut self, text: impl Into<String>) {
        self.entries.push(TranscriptEntry::System(SystemEntry {
            text: text.into(),
            timestamp: Some(Utc::now()),
        }));
    }

    pub(crate) fn push_stream(&mut self, name: StreamName, text: impl Into<String>) {
        let text = text.into();
        if let Some(TranscriptEntry::Stream(previous)) = self.entries.last_mut()
            && previous.name == name
        {
            previous.text.push_str(&text);
            return;
        }

        self.entries.push(TranscriptEntry::Stream(StreamEntry {
            name,
            text,
            timestamp: Some(Utc::now()),
        }));
    }

    pub(crate) fn push_execute_result(&mut self, execution_count: Option<u32>, mime: MimeBundle) {
        self.entries
            .push(TranscriptEntry::ExecuteResult(OutputEntry {
                execution_count,
                mime,
                metadata: serde_json::Value::Object(serde_json::Map::new()),
                timestamp: Some(Utc::now()),
            }));
    }

    pub(crate) fn push_error(&mut self, traceback: Vec<String>) {
        self.entries.push(TranscriptEntry::Error(ErrorEntry {
            ename: None,
            evalue: None,
            traceback,
            timestamp: Some(Utc::now()),
        }));
    }

    pub(crate) fn push_stdin(&mut self, prompt: String, password: bool) -> usize {
        let index = self.entries.len();
        self.entries.push(TranscriptEntry::Stdin(StdinEntry {
            prompt,
            password,
            value: None,
            timestamp: Some(Utc::now()),
        }));
        index
    }

    pub(crate) fn fill_stdin_value(&mut self, index: usize, value: String) {
        if let Some(TranscriptEntry::Stdin(stdin)) = self.entries.get_mut(index) {
            stdin.value = Some(value);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TranscriptEntry {
    Input(InputEntry),
    Stream(StreamEntry),
    ExecuteResult(OutputEntry),
    DisplayData(OutputEntry),
    Error(ErrorEntry),
    Stdin(StdinEntry),
    System(SystemEntry),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct InputEntry {
    pub execution_count: Option<u32>,
    pub code: String,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamName {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StreamEntry {
    pub name: StreamName,
    pub text: String,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OutputEntry {
    pub execution_count: Option<u32>,
    pub mime: MimeBundle,
    pub metadata: serde_json::Value,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MimeBundle {
    pub text_plain: Option<String>,
    #[serde(default)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

impl MimeBundle {
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            text_plain: Some(text.into()),
            other: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ErrorEntry {
    pub ename: Option<String>,
    pub evalue: Option<String>,
    pub traceback: Vec<String>,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StdinEntry {
    pub prompt: String,
    pub password: bool,
    pub value: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SystemEntry {
    pub text: String,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct EditorModel {
    pub text: String,
    pub prompt: String,
    #[serde(skip)]
    pub render_state: Option<EditorState>,
}

impl std::fmt::Debug for EditorModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorModel")
            .field("text", &self.text)
            .field("prompt", &self.prompt)
            .finish_non_exhaustive()
    }
}

impl PartialEq for EditorModel {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.prompt == other.prompt
    }
}

impl Eq for EditorModel {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum OverlayModel {
    #[default]
    None,
    Palette(PaletteOverlayModel),
    HistorySearch(HistorySearchOverlayModel),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PaletteOverlayModel {
    pub items: Vec<String>,
    pub selected: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HistorySearchOverlayModel {
    pub query: String,
    pub results: Vec<HistorySearchResultModel>,
    pub selected: usize,
    pub preview_lines: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HistorySearchResultModel {
    pub summary: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DisplayKernelStatus {
    Connecting,
    Idle,
    Busy,
    AwaitingInput,
    Disconnected,
}

impl From<KernelStatus> for DisplayKernelStatus {
    fn from(status: KernelStatus) -> Self {
        match status {
            KernelStatus::Connecting => Self::Connecting,
            KernelStatus::Idle => Self::Idle,
            KernelStatus::Busy => Self::Busy,
            KernelStatus::AwaitingInput => Self::AwaitingInput,
            KernelStatus::Disconnected => Self::Disconnected,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FooterModel {
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalFrame {
    pub size: Size,
    pub full_rows: Vec<TerminalRow>,
    pub cursor: CursorState,
    pub transcript_append_safe: bool,
}

impl TerminalFrame {
    pub(crate) fn visible_rows(&self) -> Vec<String> {
        let height = self.size.height as usize;
        let start = self.full_rows.len().saturating_sub(height);
        self.full_rows[start..]
            .iter()
            .map(|row| row.text.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalRow {
    pub text: String,
    pub kind: RowKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RowKind {
    CommittedTranscript,
    LiveUi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorState {
    pub position: Option<Position>,
    pub style: FrameCursorStyle,
    pub visible: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            position: None,
            style: FrameCursorStyle::Default,
            visible: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrameCursorStyle {
    Default,
    Block,
    Bar,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DisplayRenderer;

impl DisplayRenderer {
    pub(crate) fn render(&mut self, model: &DisplayModel, size: Size) -> TerminalFrame {
        let width = size.width.max(1);
        let mut full_rows = Vec::new();

        let mut transcript = TranscriptComponent::new(&model.transcript);
        for line in transcript.render(width) {
            full_rows.push(TerminalRow {
                text: line.text,
                kind: RowKind::CommittedTranscript,
            });
        }

        let mut cursor_marker = None;

        let has_overlay = !matches!(model.overlay, OverlayModel::None);
        if !has_overlay {
            let mut editor = EditorComponent::new(&model.editor);
            for line in editor.render(width) {
                update_cursor_marker(&mut cursor_marker, &line, full_rows.len());
                full_rows.push(TerminalRow {
                    text: line.text,
                    kind: RowKind::LiveUi,
                });
            }
        } else {
            let mut overlay = OverlayComponent::new(&model.overlay);
            for line in overlay.render(width) {
                update_cursor_marker(&mut cursor_marker, &line, full_rows.len());
                full_rows.push(TerminalRow {
                    text: line.text,
                    kind: RowKind::LiveUi,
                });
            }
        }

        let mut footer = FooterComponent::new(&model.footer);
        for line in footer.render(width) {
            update_cursor_marker(&mut cursor_marker, &line, full_rows.len());
            full_rows.push(TerminalRow {
                text: line.text,
                kind: RowKind::LiveUi,
            });
        }

        let cursor = resolve_cursor_marker(cursor_marker, full_rows.len(), size);

        TerminalFrame {
            size,
            full_rows,
            transcript_append_safe: !has_overlay,
            cursor,
        }
    }
}

pub(crate) fn fixture_json(scenario: &str, width: u16, height: u16) -> anyhow::Result<String> {
    let mut model = DisplayModel::new();
    match scenario {
        "bottom-pinned-output" => {
            model.transcript.push_system("fpy 0.1.0");
            model
                .transcript
                .push_input(Some(1), "for i in range(3):\n    print(i)");
            model.transcript.push_stream(StreamName::Stdout, "0\n1\n2");
            model.editor.text.clear();
        }
        "wrapped-output" => {
            model.transcript.push_input(Some(1), "print('abcdef')");
            model
                .transcript
                .push_stream(StreamName::Stdout, "abcdefghijklmnopqrstuvwxyz");
        }
        "stderr-stream" => {
            model.transcript.push_input(Some(1), "import sys");
            model
                .transcript
                .push_stream(StreamName::Stderr, "warning on stderr\n");
        }
        "stdin-reply" => {
            model
                .transcript
                .push_input(Some(1), "name = input('Name: ')");
            let stdin = model.transcript.push_stdin("Name: ".to_string(), false);
            model.transcript.fill_stdin_value(stdin, "Ada".to_string());
        }
        "resize-reflow" => {
            model
                .transcript
                .push_system("a transcript row that is intentionally wider than the fixture");
            model.editor.text = "2 + 2".to_string();
        }
        "palette" => {
            model.overlay = OverlayModel::Palette(PaletteOverlayModel {
                items: vec![
                    "Quit".to_string(),
                    "Interrupt Kernel".to_string(),
                    "Restart Kernel".to_string(),
                ],
                selected: 1,
            });
            model.footer.text = Some("Ctrl-P palette".to_string());
        }
        other => anyhow::bail!("unknown display fixture scenario: {other}"),
    }

    let mut renderer = DisplayRenderer;
    let mut backend = RecordingBackend::new(Size::new(width, height));
    let size = backend.size()?;
    backend.draw_frame(renderer.render(&model, size))?;
    let snapshot = backend
        .snapshot()
        .ok_or_else(|| anyhow::anyhow!("fixture did not produce a snapshot"))?;
    Ok(serde_json::to_string_pretty(&json!({
        "model": {
            "transcript_entries": model.transcript.entries.len(),
            "transcript": model.transcript,
            "kernel_status": model.kernel_status,
        },
        "full_rows": snapshot.full_rows,
        "visible_rows": snapshot.visible_rows,
        "expected_scrollback_rows": snapshot.expected_scrollback_rows,
        "cursor": cursor_json(&snapshot.cursor),
    }))?)
}

pub(crate) fn fixture_sequence_json(
    sequence: &str,
    width: u16,
    height: u16,
) -> anyhow::Result<String> {
    let mut renderer = DisplayRenderer;
    let mut backend = RecordingBackend::new(Size::new(width, height));
    let mut model = DisplayModel::new();
    let mut frames = Vec::new();

    match sequence {
        "append-edit-resize" => {
            model.transcript.push_system("ready");
            push_sequence_frame("initial", &mut renderer, &mut backend, &model, &mut frames)?;

            model.editor.text = "editing".to_string();
            push_sequence_frame(
                "live-ui-edit",
                &mut renderer,
                &mut backend,
                &model,
                &mut frames,
            )?;

            model.transcript.push_input(Some(1), "1 + 1");
            model
                .transcript
                .push_execute_result(Some(1), MimeBundle::plain("2"));
            push_sequence_frame(
                "transcript-append",
                &mut renderer,
                &mut backend,
                &model,
                &mut frames,
            )?;

            let resized = Size::new(width.saturating_div(2).max(1), height);
            backend.draw_frame(renderer.render(&model, resized))?;
            frames.push(sequence_snapshot_json("resize-reflow", &backend)?);
        }
        other => anyhow::bail!("unknown display fixture sequence: {other}"),
    }

    Ok(serde_json::to_string_pretty(&json!({
        "sequence": sequence,
        "frames": frames,
    }))?)
}

fn push_sequence_frame(
    name: &str,
    renderer: &mut DisplayRenderer,
    backend: &mut RecordingBackend,
    model: &DisplayModel,
    frames: &mut Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    let size = backend.size()?;
    backend.draw_frame(renderer.render(model, size))?;
    frames.push(sequence_snapshot_json(name, backend)?);
    Ok(())
}

fn sequence_snapshot_json(
    name: &str,
    backend: &RecordingBackend,
) -> anyhow::Result<serde_json::Value> {
    let snapshot = backend
        .snapshot()
        .ok_or_else(|| anyhow::anyhow!("sequence frame did not produce a snapshot"))?;
    Ok(json!({
        "name": name,
        "update_kind": backend.last_update_kind(),
        "full_rows": snapshot.full_rows,
        "visible_rows": snapshot.visible_rows,
        "expected_scrollback_rows": snapshot.expected_scrollback_rows,
        "cursor": cursor_json(&snapshot.cursor),
    }))
}

fn cursor_json(cursor: &CursorState) -> serde_json::Value {
    match cursor.position {
        Some(position) => json!({
            "row": position.y,
            "col": position.x,
            "visible": cursor.visible,
            "style": cursor.style,
        }),
        None => json!({
            "row": null,
            "col": null,
            "visible": cursor.visible,
            "style": cursor.style,
        }),
    }
}

fn update_cursor_marker(
    cursor_marker: &mut Option<(usize, Position, FrameCursorStyle)>,
    line: &RenderedLine,
    full_row: usize,
) {
    if let Some(marker) = line.cursor_marker {
        *cursor_marker = Some((full_row, marker.position, marker.style));
    }
}

fn resolve_cursor_marker(
    cursor_marker: Option<(usize, Position, FrameCursorStyle)>,
    full_row_count: usize,
    size: Size,
) -> CursorState {
    let Some((full_row, marker_position, style)) = cursor_marker else {
        return CursorState::default();
    };
    let visible_start = full_row_count.saturating_sub(size.height as usize);
    if full_row < visible_start {
        return CursorState::default();
    }

    CursorState {
        position: Some(Position::new(marker_position.x, full_row as u16)),
        style,
        visible: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::transcript::{display_width, strip_ansi};

    fn stripped(rows: &[String]) -> Vec<String> {
        rows.iter().map(|row| strip_ansi(row)).collect()
    }

    #[test]
    fn transcript_model_keeps_structured_input_entries() {
        let mut model = DisplayModel::new();
        model.transcript.push_input(Some(7), "x = 1\ny = 2");

        assert_eq!(model.transcript.entries.len(), 1);
        assert!(matches!(
            &model.transcript.entries[0],
            TranscriptEntry::Input(InputEntry {
                execution_count: Some(7),
                code,
                ..
            }) if code == "x = 1\ny = 2"
        ));
    }

    #[test]
    fn transcript_model_records_stdin_prompt_and_reply() {
        let mut model = DisplayModel::new();
        let index = model.transcript.push_stdin("Name: ".to_string(), false);
        model.transcript.fill_stdin_value(index, "Ada".to_string());

        assert!(matches!(
            &model.transcript.entries[index],
            TranscriptEntry::Stdin(StdinEntry {
                prompt,
                password: false,
                value: Some(value),
                ..
            }) if prompt == "Name: " && value == "Ada"
        ));
    }

    #[test]
    fn transcript_model_can_clear_visible_rows_without_dropping_entries() {
        let mut model = DisplayModel::new();
        model.transcript.push_input(Some(1), "1 + 1");
        model
            .transcript
            .push_execute_result(Some(1), MimeBundle::plain("2"));

        model.transcript.clear_visible();
        model.transcript.push_input(Some(2), "2 + 2");

        assert_eq!(model.transcript.entries.len(), 3);
        assert_eq!(model.transcript.visible_entries().len(), 1);
        assert!(matches!(
            &model.transcript.visible_entries()[0],
            TranscriptEntry::Input(InputEntry {
                execution_count: Some(2),
                code,
                ..
            }) if code == "2 + 2"
        ));
    }

    #[test]
    fn renderer_marks_transcript_and_live_ui_rows() {
        let mut model = DisplayModel::new();
        model.transcript.push_system("ready");
        model.editor.text = "2 + 2".to_string();

        let frame = DisplayRenderer.render(&model, Size::new(80, 24));

        assert_eq!(frame.full_rows[0].kind, RowKind::CommittedTranscript);
        assert_eq!(frame.full_rows[1].kind, RowKind::LiveUi);
        assert_eq!(
            stripped(&frame.visible_rows()),
            vec!["ready", "      1 2 + 2"]
        );
        assert_eq!(
            frame.cursor.position,
            Some(Position::new("      1 2 + 2".len() as u16, 1))
        );
    }

    #[test]
    fn renderer_omits_transcript_entries_before_visible_start() {
        let mut model = DisplayModel::new();
        model.transcript.push_input(Some(1), "1 + 1");
        model
            .transcript
            .push_execute_result(Some(1), MimeBundle::plain("2"));
        model.transcript.clear_visible();
        model.editor.text.clear();

        let frame = DisplayRenderer.render(&model, Size::new(80, 24));

        assert_eq!(stripped(&frame.visible_rows()), vec!["      1 "]);
        assert!(
            frame
                .full_rows
                .iter()
                .all(|row| row.kind == RowKind::LiveUi)
        );
    }

    #[test]
    fn renderer_resolves_cursor_marker_before_footer_rows() {
        let mut model = DisplayModel::new();
        model.editor.prompt = "In [1]: ".to_string();
        model.editor.text = "x".to_string();
        model.footer.text = Some("Kernel busy".to_string());

        let frame = DisplayRenderer.render(&model, Size::new(80, 24));

        assert_eq!(
            stripped(&frame.visible_rows()),
            vec!["      1 x", "Kernel busy"]
        );
        assert_eq!(
            frame.cursor.position,
            Some(Position::new("      1 x".len() as u16, 0))
        );
        assert_eq!(frame.cursor.style, FrameCursorStyle::Bar);
    }

    #[test]
    fn renderer_cursor_position_uses_full_frame_rows_when_clipped() {
        let mut model = DisplayModel::new();
        model.transcript.push_system("one");
        model.transcript.push_system("two");
        model.transcript.push_system("three");
        model.editor.prompt = "In [1]: ".to_string();
        model.editor.text = "x".to_string();

        let frame = DisplayRenderer.render(&model, Size::new(80, 2));

        assert_eq!(stripped(&frame.visible_rows()), vec!["three", "      1 x"]);
        assert_eq!(
            frame.cursor.position,
            Some(Position::new("      1 x".len() as u16, 3))
        );
    }

    #[test]
    fn renderer_uses_overlay_rows_instead_of_editor_rows() {
        let mut model = DisplayModel::new();
        model.editor.prompt = "In [1]: ".to_string();
        model.editor.text = "hidden".to_string();
        model.overlay = OverlayModel::Palette(PaletteOverlayModel {
            items: vec!["Quit".to_string(), "Restart".to_string()],
            selected: 1,
        });

        let frame = DisplayRenderer.render(&model, Size::new(80, 24));

        assert_eq!(
            stripped(&frame.visible_rows()),
            vec!["Command Palette", "  Quit", "> Restart"]
        );
        assert_eq!(frame.cursor.position, None);
        assert!(!frame.cursor.visible);
        assert!(!frame.transcript_append_safe);
    }

    #[test]
    fn renderer_marks_only_editor_projection_as_transcript_append_safe() {
        let mut model = DisplayModel::new();
        model.editor.prompt = "In [1]: ".to_string();
        model.editor.text = "x".to_string();

        let editor_frame = DisplayRenderer.render(&model, Size::new(80, 24));
        assert!(editor_frame.transcript_append_safe);

        model.overlay = OverlayModel::HistorySearch(HistorySearchOverlayModel {
            query: "x".to_string(),
            results: vec![HistorySearchResultModel {
                summary: "x".to_string(),
                selected: true,
            }],
            selected: 0,
            preview_lines: vec!["x".to_string()],
        });

        let overlay_frame = DisplayRenderer.render(&model, Size::new(80, 24));
        assert!(!overlay_frame.transcript_append_safe);
    }

    #[test]
    fn rendered_rows_do_not_exceed_width_after_ansi_stripping() {
        let mut model = DisplayModel::new();
        model.transcript.push_input(Some(1), "abcdef");
        model.transcript.push_stream(StreamName::Stdout, "ghijk語");

        let frame = DisplayRenderer.render(&model, Size::new(5, 10));

        for row in frame.full_rows {
            assert!(display_width(&row.text) <= 5, "row too wide: {:?}", row);
        }
    }

    #[test]
    fn fixture_json_includes_structured_transcript_entries() {
        let rendered = fixture_json("stderr-stream", 80, 24).expect("fixture json");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");

        assert_eq!(
            value["model"]["transcript"]["entries"][1]["kind"],
            serde_json::Value::String("stream".to_string())
        );
        assert_eq!(
            value["model"]["transcript"]["entries"][1]["name"],
            serde_json::Value::String("stderr".to_string())
        );
    }

    #[test]
    fn fixture_sequence_json_records_backend_update_kinds() {
        let rendered = fixture_sequence_json("append-edit-resize", 80, 6).expect("fixture json");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        let update_kinds = value["frames"]
            .as_array()
            .expect("frames")
            .iter()
            .map(|frame| frame["update_kind"].as_str().expect("update kind"))
            .collect::<Vec<_>>();

        assert_eq!(
            update_kinds,
            vec![
                "initial",
                "live_ui_only",
                "transcript_append",
                "resize_or_reflow"
            ]
        );
    }

    #[test]
    fn fixture_json_includes_stdin_reply_entry() {
        let rendered = fixture_json("stdin-reply", 80, 24).expect("fixture json");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");

        assert_eq!(
            value["model"]["transcript"]["entries"][1]["kind"],
            serde_json::Value::String("stdin".to_string())
        );
        assert_eq!(
            value["model"]["transcript"]["entries"][1]["value"],
            serde_json::Value::String("Ada".to_string())
        );
    }
}
