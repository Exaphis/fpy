use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, MoveToColumn, SetCursorStyle, Show},
    queue,
    style::Print,
    terminal::{self, Clear, ClearType},
};
use ratatui::layout::Size;
use serde::Serialize;

use super::display::{CursorState, FrameCursorStyle, RowKind, TerminalFrame};

pub(crate) trait TerminalBackend {
    fn size(&mut self) -> io::Result<Size>;
    fn draw_frame(&mut self, frame: TerminalFrame) -> io::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestSnapshot {
    pub full_rows: Vec<String>,
    pub visible_rows: Vec<String>,
    pub expected_scrollback_rows: Vec<String>,
    pub cursor: CursorState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrameUpdateKind {
    Initial,
    TranscriptAppend,
    LiveUiOnly,
    ResizeOrReflow,
    Recovery,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordingBackend {
    size: Size,
    previous_committed_rows: Vec<String>,
    previous_size: Option<Size>,
    previous_append_safe: bool,
    last_update_kind: Option<FrameUpdateKind>,
    expected_scrollback_rows: Vec<String>,
    snapshot: Option<TestSnapshot>,
}

pub(crate) struct CrosstermMainScreenBackend<W: Write> {
    writer: W,
    size: Size,
    origin_y: u16,
    previous_origin_y: Option<u16>,
    last_visible_row_count: usize,
    previous_committed_rows: Vec<String>,
    previous_size: Option<Size>,
    previous_visible_rows: Vec<String>,
    previous_append_safe: bool,
    needs_full_projection_reset: bool,
    last_update_kind: Option<FrameUpdateKind>,
}

impl<W: Write> CrosstermMainScreenBackend<W> {
    pub(crate) fn new(writer: W, size: Size) -> Self {
        Self::with_origin(writer, size, 0)
    }

    pub(crate) fn with_origin(writer: W, size: Size, origin_y: u16) -> Self {
        Self {
            writer,
            size,
            origin_y,
            previous_origin_y: None,
            last_visible_row_count: 0,
            previous_committed_rows: Vec::new(),
            previous_size: None,
            previous_visible_rows: Vec::new(),
            previous_append_safe: false,
            needs_full_projection_reset: false,
            last_update_kind: None,
        }
    }

    #[cfg(test)]
    fn last_update_kind(&self) -> Option<FrameUpdateKind> {
        self.last_update_kind
    }

    #[cfg(test)]
    fn writer(&self) -> &W {
        &self.writer
    }

    fn draw_origin_y(&self, size: Size) -> u16 {
        self.origin_y.min(size.height.saturating_sub(1))
    }

    fn visible_rows(&self, frame: &TerminalFrame) -> (Vec<String>, usize, u16) {
        let origin_y = self.draw_origin_y(frame.size);
        let available_height = frame.size.height.saturating_sub(origin_y).max(1) as usize;
        let start = frame.full_rows.len().saturating_sub(available_height);
        let rows = frame.full_rows[start..]
            .iter()
            .map(|row| row.text.clone())
            .collect();
        (rows, start, origin_y)
    }

    fn draw_visible_rows(&mut self, frame: &TerminalFrame) -> io::Result<(usize, u16)> {
        let (visible_rows, first_full_row, origin_y) = self.visible_rows(frame);
        if let Some(previous_origin_y) = self.previous_origin_y
            && previous_origin_y != origin_y
        {
            for row in 0..self.previous_visible_rows.len() {
                queue!(
                    self.writer,
                    MoveTo(0, previous_origin_y.saturating_add(row as u16)),
                    Clear(ClearType::CurrentLine)
                )?;
            }
        }
        for (row, text) in visible_rows.iter().enumerate() {
            if self.previous_visible_rows.get(row) == Some(text) {
                continue;
            }
            queue!(
                self.writer,
                MoveTo(0, origin_y.saturating_add(row as u16)),
                Clear(ClearType::CurrentLine),
                Print(text)
            )?;
        }
        for row in visible_rows.len()..self.previous_visible_rows.len() {
            queue!(
                self.writer,
                MoveTo(0, origin_y.saturating_add(row as u16)),
                Clear(ClearType::CurrentLine)
            )?;
        }
        self.previous_visible_rows = visible_rows;
        self.previous_origin_y = Some(origin_y);
        self.last_visible_row_count = self.previous_visible_rows.len();
        Ok((first_full_row, origin_y))
    }

    fn append_committed_rows(&mut self, rows: &[String]) -> io::Result<()> {
        for row in rows {
            queue!(
                self.writer,
                MoveToColumn(0),
                Clear(ClearType::CurrentLine),
                Print(row),
                Print("\r\n")
            )?;
        }
        Ok(())
    }

    fn full_projection_reset(&mut self) -> io::Result<()> {
        queue!(
            self.writer,
            Hide,
            MoveTo(0, 0),
            Clear(ClearType::All),
            Clear(ClearType::Purge),
            MoveTo(0, 0)
        )?;
        self.origin_y = 0;
        self.previous_origin_y = None;
        self.last_visible_row_count = 0;
        self.previous_committed_rows.clear();
        self.previous_visible_rows.clear();
        self.previous_append_safe = false;
        self.needs_full_projection_reset = false;
        self.previous_size = None;
        Ok(())
    }

    pub(crate) fn clear_screen(&mut self) -> io::Result<()> {
        queue!(self.writer, Hide, MoveTo(0, 0), Clear(ClearType::All))?;
        self.origin_y = 0;
        self.previous_origin_y = None;
        self.last_visible_row_count = 0;
        self.previous_committed_rows.clear();
        self.previous_visible_rows.clear();
        self.previous_append_safe = false;
        self.needs_full_projection_reset = false;
        self.previous_size = None;
        self.last_update_kind = Some(FrameUpdateKind::Recovery);
        self.writer.flush()
    }

    pub(crate) fn prepare_shutdown(&mut self) -> io::Result<u16> {
        let origin_y = self.draw_origin_y(self.size);
        let shell_row = origin_y.saturating_add(self.last_visible_row_count as u16);
        let row = if shell_row >= self.size.height {
            let bottom = self.size.height.saturating_sub(1);
            queue!(self.writer, MoveTo(0, bottom), Print("\r\n"))?;
            bottom
        } else {
            queue!(
                self.writer,
                MoveTo(0, shell_row),
                Clear(ClearType::CurrentLine)
            )?;
            shell_row
        };
        self.writer.flush()?;
        Ok(row)
    }
}

impl CrosstermMainScreenBackend<std::io::Stdout> {
    #[allow(dead_code)]
    pub(crate) fn stdout() -> io::Result<Self> {
        let (width, height) = terminal::size()?;
        Ok(Self::new(std::io::stdout(), Size::new(width, height)))
    }

    pub(crate) fn refresh_size(&mut self) -> io::Result<Size> {
        let (width, height) = terminal::size()?;
        let new_size = Size::new(width, height);
        if new_size != self.size {
            self.origin_y = self.origin_y.min(height.saturating_sub(1));
            self.needs_full_projection_reset = true;
        }
        self.size = new_size;
        Ok(self.size)
    }
}

impl RecordingBackend {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            size,
            previous_committed_rows: Vec::new(),
            previous_size: None,
            previous_append_safe: false,
            last_update_kind: None,
            expected_scrollback_rows: Vec::new(),
            snapshot: None,
        }
    }

    pub(crate) fn snapshot(&self) -> Option<&TestSnapshot> {
        self.snapshot.as_ref()
    }

    pub(crate) fn last_update_kind(&self) -> Option<FrameUpdateKind> {
        self.last_update_kind
    }

    fn update_scrollback_expectation(&mut self, frame: &TerminalFrame) {
        let committed_rows = committed_rows(frame);

        let update_kind = classify_frame_update(
            self.previous_size,
            &self.previous_committed_rows,
            frame.size,
            &committed_rows,
        );

        match update_kind {
            FrameUpdateKind::Initial | FrameUpdateKind::Recovery => {
                self.expected_scrollback_rows = committed_rows.clone();
            }
            FrameUpdateKind::ResizeOrReflow => {
                self.expected_scrollback_rows.clear();
            }
            FrameUpdateKind::TranscriptAppend if self.previous_append_safe => {
                self.expected_scrollback_rows
                    .extend_from_slice(&committed_rows[self.previous_committed_rows.len()..]);
            }
            FrameUpdateKind::TranscriptAppend => {}
            FrameUpdateKind::LiveUiOnly => {}
        }

        self.previous_committed_rows = committed_rows;
        self.previous_size = Some(frame.size);
        self.previous_append_safe = frame.transcript_append_safe;
        self.last_update_kind = Some(update_kind);
    }
}

impl Default for RecordingBackend {
    fn default() -> Self {
        Self::new(Size::new(80, 24))
    }
}

fn committed_rows(frame: &TerminalFrame) -> Vec<String> {
    frame
        .full_rows
        .iter()
        .filter(|row| row.kind == RowKind::CommittedTranscript)
        .map(|row| row.text.clone())
        .collect()
}

fn classify_frame_update(
    previous_size: Option<Size>,
    previous_committed_rows: &[String],
    new_size: Size,
    new_committed_rows: &[String],
) -> FrameUpdateKind {
    let Some(previous_size) = previous_size else {
        return FrameUpdateKind::Initial;
    };

    if previous_size != new_size {
        return FrameUpdateKind::ResizeOrReflow;
    }

    if new_committed_rows == previous_committed_rows {
        return FrameUpdateKind::LiveUiOnly;
    }

    if new_committed_rows.starts_with(previous_committed_rows)
        && new_committed_rows.len() > previous_committed_rows.len()
    {
        return FrameUpdateKind::TranscriptAppend;
    }

    FrameUpdateKind::Recovery
}

impl TerminalBackend for RecordingBackend {
    fn size(&mut self) -> io::Result<Size> {
        Ok(self.size)
    }

    fn draw_frame(&mut self, frame: TerminalFrame) -> io::Result<()> {
        self.size = frame.size;
        self.update_scrollback_expectation(&frame);
        self.snapshot = Some(TestSnapshot {
            full_rows: frame.full_rows.iter().map(|row| row.text.clone()).collect(),
            visible_rows: frame.visible_rows(),
            expected_scrollback_rows: self.expected_scrollback_rows.clone(),
            cursor: frame.cursor,
        });
        Ok(())
    }
}

impl<W: Write> TerminalBackend for CrosstermMainScreenBackend<W> {
    fn size(&mut self) -> io::Result<Size> {
        Ok(self.size)
    }

    fn draw_frame(&mut self, frame: TerminalFrame) -> io::Result<()> {
        let committed_rows = committed_rows(&frame);
        let update_kind = classify_frame_update(
            self.previous_size,
            &self.previous_committed_rows,
            frame.size,
            &committed_rows,
        );

        let full_projection_reset =
            self.needs_full_projection_reset || update_kind == FrameUpdateKind::ResizeOrReflow;
        if full_projection_reset {
            self.full_projection_reset()?;
        } else if update_kind == FrameUpdateKind::TranscriptAppend && self.previous_append_safe {
            self.append_committed_rows(&committed_rows[self.previous_committed_rows.len()..])?;
            self.previous_visible_rows.clear();
        }

        let (first_full_row, origin_y) = self.draw_visible_rows(&frame)?;
        draw_cursor(
            &mut self.writer,
            &frame.cursor,
            first_full_row,
            origin_y,
            frame.size,
        )?;
        self.writer.flush()?;
        self.previous_committed_rows = committed_rows;
        self.previous_size = Some(frame.size);
        self.previous_append_safe = frame.transcript_append_safe;
        self.last_update_kind = Some(update_kind);
        self.size = frame.size;
        Ok(())
    }
}

fn cursor_row_offset(cursor: &CursorState, first_full_row: usize, size: Size) -> Option<u16> {
    let position = cursor.position.filter(|_| cursor.visible)?;
    let frame_row = position.y as usize;
    if frame_row < first_full_row {
        return None;
    }
    let offset = (frame_row - first_full_row) as u16;
    (offset < size.height).then_some(offset)
}

fn draw_cursor(
    writer: &mut impl Write,
    cursor: &CursorState,
    first_full_row: usize,
    origin_y: u16,
    size: Size,
) -> io::Result<()> {
    if let Some(position) = cursor.position.filter(|_| cursor.visible) {
        let Some(offset) = cursor_row_offset(cursor, first_full_row, size) else {
            queue!(writer, Hide)?;
            return Ok(());
        };
        let screen_y = origin_y.saturating_add(offset);
        if screen_y >= size.height {
            queue!(writer, Hide)?;
            return Ok(());
        }
        queue!(
            writer,
            Show,
            to_crossterm_cursor_style(cursor.style),
            MoveTo(position.x.min(size.width.saturating_sub(1)), screen_y)
        )?;
    } else {
        queue!(writer, Hide)?;
    }
    Ok(())
}

fn to_crossterm_cursor_style(style: FrameCursorStyle) -> SetCursorStyle {
    match style {
        FrameCursorStyle::Default => SetCursorStyle::DefaultUserShape,
        FrameCursorStyle::Block => SetCursorStyle::SteadyBlock,
        FrameCursorStyle::Bar => SetCursorStyle::SteadyBar,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Size;

    use super::*;
    use crate::ui::{
        display::{
            DisplayModel, DisplayRenderer, HistorySearchOverlayModel, HistorySearchResultModel,
            OverlayModel, PaletteOverlayModel,
        },
        transcript::strip_ansi,
    };

    fn render_into(
        renderer: &mut DisplayRenderer,
        backend: &mut RecordingBackend,
        model: &DisplayModel,
        size: Size,
    ) {
        backend
            .draw_frame(renderer.render(model, size))
            .expect("draw frame");
    }

    fn stripped(rows: &[String]) -> Vec<String> {
        rows.iter().map(|row| strip_ansi(row)).collect()
    }

    #[test]
    fn recording_backend_appends_only_new_committed_rows() {
        let mut renderer = DisplayRenderer;
        let mut backend = RecordingBackend::default();
        let mut model = DisplayModel::new();

        model.transcript.push_system("first");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));

        model.editor.text = "editing".to_string();
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));

        model.transcript.push_system("second");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));

        let snapshot = backend.snapshot().expect("snapshot");
        assert_eq!(
            snapshot.expected_scrollback_rows,
            vec!["first".to_string(), "second".to_string()]
        );
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::TranscriptAppend)
        );
    }

    #[test]
    fn recording_backend_does_not_append_live_ui_edits() {
        let mut renderer = DisplayRenderer;
        let mut backend = RecordingBackend::default();
        let mut model = DisplayModel::new();

        model.transcript.push_system("first");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));

        model.editor.text = "a".to_string();
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));
        model.editor.text = "ab".to_string();
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));

        let snapshot = backend.snapshot().expect("snapshot");
        assert_eq!(snapshot.expected_scrollback_rows, vec!["first".to_string()]);
        assert_eq!(
            stripped(&snapshot.visible_rows),
            vec!["first", "In [?]: ab"]
        );
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::LiveUiOnly)
        );
    }

    #[test]
    fn recording_backend_clears_expected_scrollback_on_resize() {
        let mut renderer = DisplayRenderer;
        let mut backend = RecordingBackend::default();
        let mut model = DisplayModel::new();

        model.transcript.push_system("abcdef");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));
        render_into(&mut renderer, &mut backend, &model, Size::new(3, 3));

        let snapshot = backend.snapshot().expect("snapshot");
        assert!(snapshot.expected_scrollback_rows.is_empty());
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::ResizeOrReflow)
        );
    }

    #[test]
    fn recording_backend_recovers_from_non_prefix_committed_change() {
        let mut renderer = DisplayRenderer;
        let mut backend = RecordingBackend::default();
        let mut model = DisplayModel::new();

        model.transcript.push_system("old");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));
        model.transcript.entries.clear();
        model.transcript.push_system("new");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));

        let snapshot = backend.snapshot().expect("snapshot");
        assert_eq!(snapshot.expected_scrollback_rows, vec!["new".to_string()]);
        assert_eq!(backend.last_update_kind(), Some(FrameUpdateKind::Recovery));
    }

    #[test]
    fn recording_backend_classifies_first_frame_as_initial() {
        let mut renderer = DisplayRenderer;
        let mut backend = RecordingBackend::default();
        let mut model = DisplayModel::new();

        model.transcript.push_system("first");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 3));

        assert_eq!(backend.last_update_kind(), Some(FrameUpdateKind::Initial));
    }

    #[test]
    fn main_screen_backend_appends_only_transcript_growth() {
        let mut renderer = DisplayRenderer;
        let mut backend = CrosstermMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));
        let mut model = DisplayModel::new();

        model.transcript.push_system("first");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 3));
        let initial_len = backend.writer().len();

        model.editor.text = "editing".to_string();
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 3));
        let edit_output = String::from_utf8_lossy(&backend.writer()[initial_len..]);
        assert!(!edit_output.contains("first\r\n"));
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::LiveUiOnly)
        );

        let before_append = backend.writer().len();
        model.transcript.push_system("second");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 3));
        let append_output = String::from_utf8_lossy(&backend.writer()[before_append..]);
        assert!(append_output.contains("second\r\n"));
        assert!(!append_output.contains("first\r\n"));
        assert!(append_output.contains("\u{1b}[2Ksecond\r\n"));
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::TranscriptAppend)
        );
    }

    #[test]
    fn main_screen_backend_does_not_append_transcript_from_palette_projection() {
        let mut renderer = DisplayRenderer;
        let mut backend = CrosstermMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 8));
        let mut model = DisplayModel::new();

        model.overlay = OverlayModel::Palette(PaletteOverlayModel {
            items: vec![
                "Quit".to_string(),
                "Interrupt Kernel".to_string(),
                "Restart Kernel".to_string(),
            ],
            selected: 1,
        });
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 8));

        let before_append = backend.writer().len();
        model.overlay = OverlayModel::None;
        model.transcript.push_system("kernel restarted");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 8));
        let output = String::from_utf8_lossy(&backend.writer()[before_append..]);

        assert!(!output.contains("kernel restarted\r\n"));
        assert!(!output.contains("Interrupt Kernel\r\n"));
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::TranscriptAppend)
        );
    }

    #[test]
    fn main_screen_backend_does_not_append_transcript_from_history_search_projection() {
        let mut renderer = DisplayRenderer;
        let mut backend = CrosstermMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 8));
        let mut model = DisplayModel::new();

        model.overlay = OverlayModel::HistorySearch(HistorySearchOverlayModel {
            query: "restart".to_string(),
            results: vec![HistorySearchResultModel {
                summary: "Restart Kernel".to_string(),
                selected: true,
            }],
            selected: 0,
            preview_lines: vec!["Restart Kernel".to_string()],
        });
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 8));

        let before_append = backend.writer().len();
        model.overlay = OverlayModel::None;
        model.transcript.push_system("kernel restarted");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 8));
        let output = String::from_utf8_lossy(&backend.writer()[before_append..]);

        assert!(!output.contains("kernel restarted\r\n"));
        assert!(!output.contains("Restart Kernel\r\n"));
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::TranscriptAppend)
        );
    }

    #[test]
    fn main_screen_backend_full_resets_without_appending_on_resize() {
        let mut renderer = DisplayRenderer;
        let mut backend = CrosstermMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));
        let mut model = DisplayModel::new();

        model.transcript.push_system("abcdef");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 3));
        let before_resize = backend.writer().len();

        render_into_main(&mut renderer, &mut backend, &model, Size::new(3, 3));
        let resize_output = String::from_utf8_lossy(&backend.writer()[before_resize..]);

        assert!(resize_output.contains("\u{1b}[2J"));
        assert!(resize_output.contains("\u{1b}[3J"));
        assert!(!resize_output.contains("abc\r\n"));
        assert!(!resize_output.contains("def\r\n"));
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::ResizeOrReflow)
        );
        assert_eq!(backend.previous_origin_y, Some(0));
        assert_eq!(
            backend.previous_committed_rows,
            vec!["abc".to_string(), "def".to_string()]
        );
        assert_eq!(
            stripped(&backend.previous_visible_rows),
            vec!["In ", "[?]", ": "]
        );
        assert_eq!(backend.previous_size, Some(Size::new(3, 3)));

        let before_append = backend.writer().len();
        model.transcript.push_system("ghi");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(3, 3));
        let append_output = String::from_utf8_lossy(&backend.writer()[before_append..]);

        assert!(append_output.contains("ghi\r\n"));
        assert!(!append_output.contains("abc\r\n"));
        assert!(!append_output.contains("\u{1b}[3J"));
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::TranscriptAppend)
        );
    }

    #[test]
    fn main_screen_backend_clears_stale_rows_after_shorter_frame() {
        let mut renderer = DisplayRenderer;
        let mut backend = CrosstermMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 5));
        let mut model = DisplayModel::new();

        model.transcript.push_system("one");
        model.transcript.push_system("two");
        model.editor.text = "three".to_string();
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 5));
        let before_shorter_frame = backend.writer().len();

        model.transcript.entries.clear();
        model.editor.text = "one".to_string();
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 5));
        let output = String::from_utf8_lossy(&backend.writer()[before_shorter_frame..]);

        assert_eq!(output.matches("\u{1b}[2K").count(), 3);
        assert_eq!(backend.last_update_kind(), Some(FrameUpdateKind::Recovery));
    }

    #[test]
    fn main_screen_backend_skips_unchanged_visible_rows() {
        let mut renderer = DisplayRenderer;
        let mut backend = CrosstermMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 5));
        let mut model = DisplayModel::new();

        model.transcript.push_system("stable");
        model.editor.text = "a".to_string();
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 5));

        let before_duplicate = backend.writer().len();
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 5));
        let duplicate_output = String::from_utf8_lossy(&backend.writer()[before_duplicate..]);
        assert!(!duplicate_output.contains("\u{1b}[2K"));
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::LiveUiOnly)
        );

        let before_edit = backend.writer().len();
        model.editor.text = "ab".to_string();
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 5));
        let edit_output = String::from_utf8_lossy(&backend.writer()[before_edit..]);

        assert_eq!(edit_output.matches("\u{1b}[2K").count(), 1);
        assert!(!edit_output.contains("stable"));
    }

    #[test]
    fn main_screen_backend_draws_relative_to_startup_origin() {
        let mut renderer = DisplayRenderer;
        let mut backend =
            CrosstermMainScreenBackend::with_origin(Vec::<u8>::new(), Size::new(80, 10), 5);
        let mut model = DisplayModel::new();

        model.transcript.push_system("first");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 10));
        let output = String::from_utf8_lossy(backend.writer());

        assert!(output.contains("\u{1b}[6;1H\u{1b}[2Kfirst"));
        assert!(output.contains("\u{1b}[7;1H\u{1b}[2K\u{1b}[36mIn [?]: "));
        assert!(output.contains("\u{1b}[7;9H"));
    }

    #[test]
    fn main_screen_backend_clips_frame_to_rows_below_origin() {
        let mut renderer = DisplayRenderer;
        let mut backend =
            CrosstermMainScreenBackend::with_origin(Vec::<u8>::new(), Size::new(80, 3), 1);
        let mut model = DisplayModel::new();

        model.transcript.push_system("hidden");
        model.transcript.push_system("visible");
        render_into_main(&mut renderer, &mut backend, &model, Size::new(80, 3));
        let output = String::from_utf8_lossy(backend.writer());

        assert!(!output.contains("hidden"));
        assert!(output.contains("\u{1b}[2;1H\u{1b}[2Kvisible"));
        assert!(output.contains("\u{1b}[3;1H\u{1b}[2K\u{1b}[36mIn [?]: "));
        assert!(output.contains("\u{1b}[3;9H"));
    }

    fn render_into_main(
        renderer: &mut DisplayRenderer,
        backend: &mut CrosstermMainScreenBackend<Vec<u8>>,
        model: &DisplayModel,
        size: Size,
    ) {
        backend
            .draw_frame(renderer.render(model, size))
            .expect("draw frame");
    }
}
