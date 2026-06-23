use std::io::{self, Write};

use crossterm::terminal;
use ratatui::layout::Size;
use serde::Serialize;

use super::display::{CursorState, FrameCursorStyle, RowKind, TerminalFrame, TerminalRow};
use super::transcript::display_width;

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
    pending_scrollback_recovery: bool,
    last_update_kind: Option<FrameUpdateKind>,
    expected_scrollback_rows: Vec<String>,
    snapshot: Option<TestSnapshot>,
}

pub(crate) struct PiStyleMainScreenBackend<W: Write> {
    writer: W,
    size: Size,
    previous_lines: Vec<String>,
    previous_width: u16,
    previous_height: u16,
    previous_viewport_top: usize,
    hardware_cursor_row: usize,
    last_update_kind: Option<FrameUpdateKind>,
}

const SYNC_OUTPUT_ENABLE: &str = "\x1b[?2026h";
const SYNC_OUTPUT_DISABLE: &str = "\x1b[?2026l";
const CSI_RESET: &str = "\x1b[0m";
const CSI_CLEAR_LINE: &str = "\x1b[2K";
const CSI_CLEAR_SCREEN: &str = "\x1b[2J";
const CSI_CLEAR_SCROLLBACK: &str = "\x1b[3J";
const CSI_HOME: &str = "\x1b[H";
const CSI_HIDE_CURSOR: &str = "\x1b[?25l";
const CSI_SHOW_CURSOR: &str = "\x1b[?25h";

fn sync_output(body: String) -> String {
    format!("{SYNC_OUTPUT_ENABLE}{body}{SYNC_OUTPUT_DISABLE}")
}

impl<W: Write> PiStyleMainScreenBackend<W> {
    pub(crate) fn new(writer: W, size: Size) -> Self {
        Self {
            writer,
            size,
            previous_lines: Vec::new(),
            previous_width: size.width,
            previous_height: size.height,
            previous_viewport_top: 0,
            hardware_cursor_row: 0,
            last_update_kind: None,
        }
    }

    #[cfg(test)]
    fn writer(&self) -> &W {
        &self.writer
    }

    #[cfg(test)]
    fn last_update_kind(&self) -> Option<FrameUpdateKind> {
        self.last_update_kind
    }

    fn viewport_top_for(&self, line_count: usize) -> usize {
        line_count.saturating_sub(self.size.height as usize)
    }

    fn full_render(
        &mut self,
        new_lines: &[String],
        cursor: &CursorState,
        clear: bool,
    ) -> io::Result<()> {
        let mut output = String::new();
        output.push_str(SYNC_OUTPUT_ENABLE);
        if clear {
            output.push_str(CSI_HIDE_CURSOR);
            output.push_str(CSI_HOME);
            output.push_str(CSI_CLEAR_SCREEN);
            output.push_str(CSI_CLEAR_SCROLLBACK);
            output.push_str(CSI_HOME);
        }
        for (index, line) in new_lines.iter().enumerate() {
            if index > 0 {
                output.push_str("\r\n");
            }
            output.push_str(line);
            output.push_str(CSI_RESET);
        }

        self.hardware_cursor_row = new_lines.len().saturating_sub(1);
        let viewport_top = self.viewport_top_for(new_lines.len());
        self.position_cursor_into(&mut output, cursor, viewport_top);
        output.push_str(SYNC_OUTPUT_DISABLE);
        self.writer.write_all(output.as_bytes())?;
        self.writer.flush()?;

        self.previous_lines = new_lines.to_vec();
        self.previous_width = self.size.width;
        self.previous_height = self.size.height;
        self.previous_viewport_top = viewport_top;
        Ok(())
    }

    fn differential_render(
        &mut self,
        new_lines: &[String],
        cursor: &CursorState,
    ) -> io::Result<()> {
        let old_len = self.previous_lines.len();
        let new_len = new_lines.len();
        let max_len = old_len.max(new_len);
        let first_changed = (0..max_len).find(|&index| {
            self.previous_lines.get(index).map(String::as_str)
                != new_lines.get(index).map(String::as_str)
        });
        let Some(first_changed) = first_changed else {
            let mut body = String::new();
            self.position_cursor_into(&mut body, cursor, self.previous_viewport_top);
            self.writer.write_all(sync_output(body).as_bytes())?;
            self.writer.flush()?;
            return Ok(());
        };
        let last_changed = (0..max_len)
            .rev()
            .find(|&index| {
                self.previous_lines.get(index).map(String::as_str)
                    != new_lines.get(index).map(String::as_str)
            })
            .unwrap_or(first_changed);

        let old_viewport_top = self.previous_viewport_top;
        let new_viewport_top = self.viewport_top_for(new_len);
        let is_tail_append = old_len <= new_len
            && self
                .previous_lines
                .iter()
                .zip(new_lines.iter())
                .all(|(old, new)| old == new);
        if first_changed < old_viewport_top || new_viewport_top < old_viewport_top {
            self.last_update_kind = Some(FrameUpdateKind::Recovery);
            return self.full_render(new_lines, cursor, true);
        }
        if new_viewport_top > old_viewport_top && !is_tail_append {
            self.last_update_kind = Some(FrameUpdateKind::Recovery);
            return self.full_render(new_lines, cursor, true);
        }

        let mut current_viewport_top = old_viewport_top;
        let height = self.size.height as usize;
        let viewport_scroll = if is_tail_append {
            new_viewport_top.saturating_sub(old_viewport_top)
        } else {
            0
        };
        let mut body = String::new();
        let visible_growth_scroll = if new_viewport_top == 0
            && old_viewport_top == 0
            && old_len < height
            && new_len > old_len
        {
            new_len
                .saturating_sub(old_len)
                .min(height.saturating_sub(old_len))
        } else {
            0
        };
        let pre_scroll = visible_growth_scroll;
        if pre_scroll > 0 {
            if old_len > 0 {
                self.move_to_logical_row_into(
                    &mut body,
                    old_len.saturating_sub(1),
                    old_viewport_top,
                )?;
            }
            for _ in 0..pre_scroll {
                body.push_str("\r\n");
            }
            self.hardware_cursor_row = old_len.saturating_sub(1).saturating_add(pre_scroll);
        }
        if viewport_scroll > 0 {
            let old_visible_bottom = old_viewport_top
                .saturating_add(height.saturating_sub(1))
                .min(old_len.saturating_sub(1));
            self.move_to_logical_row_into(&mut body, old_visible_bottom, old_viewport_top)?;
            for _ in 0..viewport_scroll {
                body.push_str("\r\n");
            }
            current_viewport_top = new_viewport_top;
            self.hardware_cursor_row = old_visible_bottom.saturating_add(viewport_scroll);
        }

        let new_visible_bottom = new_viewport_top
            .saturating_add(height.saturating_sub(1))
            .min(new_len.saturating_sub(1));
        if new_len < old_len {
            let old_visible_bottom = current_viewport_top
                .saturating_add(height.saturating_sub(1))
                .min(old_len.saturating_sub(1));
            if old_visible_bottom.saturating_sub(new_visible_bottom) >= height {
                self.last_update_kind = Some(FrameUpdateKind::Recovery);
                return self.full_render(new_lines, cursor, true);
            }
            let clear_start = first_changed
                .max(current_viewport_top)
                .max(new_viewport_top);
            for row in clear_start..=old_visible_bottom {
                self.clear_logical_row_into(&mut body, row, current_viewport_top)?;
            }
        }

        let repaint_all_visible = pre_scroll > 0 || viewport_scroll > 0;
        let repaint_start = if repaint_all_visible {
            new_viewport_top
        } else {
            first_changed.max(new_viewport_top)
        };
        let repaint_end = if new_len == 0 {
            None
        } else {
            Some(last_changed.max(new_visible_bottom).min(new_visible_bottom))
        };
        if let Some(repaint_end) = repaint_end {
            for row in repaint_start..=repaint_end {
                if let Some(line) = new_lines.get(row) {
                    self.repaint_logical_row_into(&mut body, row, new_viewport_top, line)?;
                }
            }
        }

        self.position_cursor_into(&mut body, cursor, new_viewport_top);
        self.writer.write_all(sync_output(body).as_bytes())?;
        self.writer.flush()?;

        self.previous_lines = new_lines.to_vec();
        self.previous_width = self.size.width;
        self.previous_height = self.size.height;
        self.previous_viewport_top = new_viewport_top;
        Ok(())
    }

    fn move_to_logical_row_into(
        &mut self,
        output: &mut String,
        target_row: usize,
        viewport_top: usize,
    ) -> io::Result<()> {
        let height = self.size.height as usize;
        if target_row < viewport_top || target_row >= viewport_top.saturating_add(height) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target row outside visible viewport",
            ));
        }
        let current_screen_row = self.hardware_cursor_row.saturating_sub(viewport_top);
        let target_screen_row = target_row.saturating_sub(viewport_top);
        if target_screen_row > current_screen_row {
            output.push_str(&format!("\x1b[{}B", target_screen_row - current_screen_row));
        } else if current_screen_row > target_screen_row {
            output.push_str(&format!("\x1b[{}A", current_screen_row - target_screen_row));
        }
        output.push('\r');
        self.hardware_cursor_row = target_row;
        Ok(())
    }

    fn clear_logical_row_into(
        &mut self,
        output: &mut String,
        target_row: usize,
        viewport_top: usize,
    ) -> io::Result<()> {
        self.move_to_logical_row_into(output, target_row, viewport_top)?;
        output.push_str(CSI_CLEAR_LINE);
        output.push_str(CSI_RESET);
        Ok(())
    }

    fn repaint_logical_row_into(
        &mut self,
        output: &mut String,
        target_row: usize,
        viewport_top: usize,
        line: &str,
    ) -> io::Result<()> {
        self.move_to_logical_row_into(output, target_row, viewport_top)?;
        output.push_str(CSI_CLEAR_LINE);
        output.push_str(line);
        output.push_str(CSI_RESET);
        Ok(())
    }

    fn position_cursor_into(
        &mut self,
        output: &mut String,
        cursor: &CursorState,
        viewport_top: usize,
    ) {
        let Some(position) = cursor.position.filter(|_| cursor.visible) else {
            output.push_str(CSI_HIDE_CURSOR);
            return;
        };
        let target_row = position.y as usize;
        let height = self.size.height as usize;
        if target_row < viewport_top || target_row >= viewport_top.saturating_add(height) {
            output.push_str(CSI_HIDE_CURSOR);
            return;
        }
        if self
            .move_to_logical_row_into(output, target_row, viewport_top)
            .is_err()
        {
            output.push_str(CSI_HIDE_CURSOR);
            return;
        }
        output.push_str(&format!(
            "\x1b[{}G",
            position
                .x
                .min(self.size.width.saturating_sub(1))
                .saturating_add(1)
        ));
        output.push_str(cursor_style_sequence(cursor.style));
        output.push_str(CSI_SHOW_CURSOR);
        self.hardware_cursor_row = target_row;
    }

    pub(crate) fn clear_screen(&mut self) -> io::Result<()> {
        self.previous_lines.clear();
        self.previous_viewport_top = 0;
        self.hardware_cursor_row = 0;
        self.last_update_kind = Some(FrameUpdateKind::Recovery);
        self.writer
            .write_all(format!("{CSI_HIDE_CURSOR}{CSI_HOME}{CSI_CLEAR_SCREEN}").as_bytes())?;
        self.writer.flush()
    }

    pub(crate) fn prepare_shutdown(&mut self) -> io::Result<u16> {
        let viewport_top = self.previous_viewport_top;
        let visible_len = self
            .previous_lines
            .len()
            .saturating_sub(viewport_top)
            .min(self.size.height as usize);
        let bottom_row = viewport_top.saturating_add(visible_len.saturating_sub(1));
        let mut output = String::new();
        output.push_str(CSI_RESET);
        output.push_str(CSI_SHOW_CURSOR);
        if visible_len > 0 {
            let _ = self.move_to_logical_row_into(&mut output, bottom_row, viewport_top);
        }
        output.push_str("\r\n");
        self.writer.write_all(output.as_bytes())?;
        self.writer.flush()?;
        Ok(self.size.height.saturating_sub(1))
    }
}

impl PiStyleMainScreenBackend<std::io::Stdout> {
    pub(crate) fn refresh_size(&mut self) -> io::Result<Size> {
        let (width, height) = terminal::size()?;
        self.size = Size::new(width, height);
        Ok(self.size)
    }
}

impl<W: Write> TerminalBackend for PiStyleMainScreenBackend<W> {
    fn size(&mut self) -> io::Result<Size> {
        Ok(self.size)
    }

    fn draw_frame(&mut self, frame: TerminalFrame) -> io::Result<()> {
        if frame.size.width == 0 || frame.size.height == 0 {
            return Ok(());
        }

        for (row_index, row) in frame.full_rows.iter().enumerate() {
            let width = display_width(&row.text);
            if width > frame.size.width as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "rendered row {row_index} exceeds terminal width: {width} > {}",
                        frame.size.width
                    ),
                ));
            }
        }

        self.size = frame.size;
        let new_lines = frame
            .full_rows
            .iter()
            .map(|row| row.text.clone())
            .collect::<Vec<_>>();
        let update_kind = if self.previous_lines.is_empty() {
            FrameUpdateKind::Initial
        } else if self.previous_width != frame.size.width
            || self.previous_height != frame.size.height
        {
            FrameUpdateKind::ResizeOrReflow
        } else {
            FrameUpdateKind::LiveUiOnly
        };
        self.last_update_kind = Some(update_kind);
        if self.previous_lines.is_empty() {
            return self.full_render(&new_lines, &frame.cursor, false);
        }
        if self.previous_width != frame.size.width || self.previous_height != frame.size.height {
            return self.full_render(&new_lines, &frame.cursor, true);
        }
        self.differential_render(&new_lines, &frame.cursor)
    }
}

impl RecordingBackend {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            size,
            previous_committed_rows: Vec::new(),
            previous_size: None,
            previous_append_safe: false,
            pending_scrollback_recovery: false,
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

        let append_can_project = update_kind == FrameUpdateKind::TranscriptAppend
            && self.previous_append_safe
            && frame.transcript_append_safe;
        let append_needs_recovery =
            update_kind == FrameUpdateKind::TranscriptAppend && !append_can_project;
        let recover_scrollback = update_kind == FrameUpdateKind::Recovery
            || ((self.pending_scrollback_recovery || append_needs_recovery)
                && frame.transcript_append_safe);

        match update_kind {
            FrameUpdateKind::Initial => {
                self.expected_scrollback_rows = committed_rows.clone();
                self.pending_scrollback_recovery = false;
            }
            _ if recover_scrollback => {
                self.expected_scrollback_rows = committed_rows.clone();
                self.pending_scrollback_recovery = false;
            }
            FrameUpdateKind::ResizeOrReflow => {
                self.expected_scrollback_rows.clear();
                self.pending_scrollback_recovery = false;
            }
            FrameUpdateKind::TranscriptAppend if append_can_project => {
                self.expected_scrollback_rows
                    .extend_from_slice(&committed_rows[self.previous_committed_rows.len()..]);
            }
            FrameUpdateKind::TranscriptAppend => {
                self.pending_scrollback_recovery = true;
            }
            FrameUpdateKind::LiveUiOnly => {}
            FrameUpdateKind::Recovery => {
                unreachable!("recover_scrollback handles recovery updates")
            }
        }

        let snapshot_frame = if append_needs_recovery && !frame.transcript_append_safe {
            recovery_pending_frame(frame, &self.previous_committed_rows)
        } else {
            frame.clone()
        };
        if !append_needs_recovery || recover_scrollback {
            self.previous_committed_rows = committed_rows;
        }
        self.previous_size = Some(frame.size);
        self.previous_append_safe = frame.transcript_append_safe;
        self.last_update_kind = Some(update_kind);
        self.snapshot = Some(TestSnapshot {
            full_rows: snapshot_frame
                .full_rows
                .iter()
                .map(|row| row.text.clone())
                .collect(),
            visible_rows: snapshot_frame.visible_rows(),
            expected_scrollback_rows: self.expected_scrollback_rows.clone(),
            cursor: snapshot_frame.cursor,
        });
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

fn recovery_pending_frame(frame: &TerminalFrame, committed_rows: &[String]) -> TerminalFrame {
    let current_committed_count = frame
        .full_rows
        .iter()
        .filter(|row| row.kind == RowKind::CommittedTranscript)
        .count();
    let dropped_committed_rows = current_committed_count.saturating_sub(committed_rows.len());
    let mut full_rows = committed_rows
        .iter()
        .cloned()
        .map(|text| TerminalRow {
            text,
            kind: RowKind::CommittedTranscript,
        })
        .collect::<Vec<_>>();
    full_rows.extend(
        frame
            .full_rows
            .iter()
            .filter(|row| row.kind == RowKind::LiveUi)
            .cloned(),
    );

    let mut cursor = frame.cursor.clone();
    if let Some(position) = cursor.position.as_mut() {
        position.y = position.y.saturating_sub(dropped_committed_rows as u16);
    }

    TerminalFrame {
        size: frame.size,
        full_rows,
        cursor,
        transcript_append_safe: frame.transcript_append_safe,
    }
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
        Ok(())
    }
}

fn cursor_style_sequence(style: FrameCursorStyle) -> &'static str {
    match style {
        FrameCursorStyle::Default => "\x1b[0 q",
        FrameCursorStyle::Block => "\x1b[2 q",
        FrameCursorStyle::Bar => "\x1b[6 q",
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Size;

    use super::*;
    use crate::ui::{
        display::{
            DisplayModel, DisplayRenderer, OverlayModel, PaletteOverlayModel,
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
            vec!["first", "      1 ab"]
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
    fn recording_backend_recovers_after_overlay_blocks_transcript_append() {
        let mut renderer = DisplayRenderer;
        let mut backend = RecordingBackend::default();
        let mut model = DisplayModel::new();

        model.overlay = OverlayModel::Palette(PaletteOverlayModel {
            items: vec![
                "Quit".to_string(),
                "Interrupt Kernel".to_string(),
                "Restart Kernel".to_string(),
            ],
            selected: 1,
        });
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 8));

        model.transcript.push_system("kernel restarted");
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 8));

        let snapshot = backend.snapshot().expect("snapshot");
        assert!(snapshot.expected_scrollback_rows.is_empty());
        assert!(backend.previous_committed_rows.is_empty());
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::TranscriptAppend)
        );

        model.overlay = OverlayModel::None;
        render_into(&mut renderer, &mut backend, &model, Size::new(80, 8));

        let snapshot = backend.snapshot().expect("snapshot");
        assert_eq!(
            snapshot.expected_scrollback_rows,
            vec!["kernel restarted".to_string()]
        );
        assert_eq!(
            backend.last_update_kind(),
            Some(FrameUpdateKind::TranscriptAppend)
        );
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

    fn frame(size: Size, rows: &[&str], cursor: CursorState) -> TerminalFrame {
        TerminalFrame {
            size,
            full_rows: rows
                .iter()
                .map(|text| TerminalRow {
                    text: (*text).to_string(),
                    kind: RowKind::CommittedTranscript,
                })
                .collect(),
            cursor,
            transcript_append_safe: true,
        }
    }

    fn hidden_cursor() -> CursorState {
        CursorState::default()
    }

    #[test]
    fn pi_style_first_render_preserves_shell_context() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));

        backend
            .draw_frame(frame(Size::new(80, 3), &["one", "two"], hidden_cursor()))
            .expect("draw frame");

        let output = String::from_utf8_lossy(backend.writer());
        assert!(output.starts_with(SYNC_OUTPUT_ENABLE));
        assert!(!output.contains(CSI_CLEAR_SCREEN));
        assert!(!output.contains(CSI_CLEAR_SCROLLBACK));
        assert!(!output.contains(CSI_HOME));
        assert!(output.contains("one\u{1b}[0m\r\ntwo\u{1b}[0m"));
        assert_eq!(backend.previous_lines, vec!["one", "two"]);
        assert_eq!(backend.previous_viewport_top, 0);
        assert_eq!(backend.last_update_kind(), Some(FrameUpdateKind::Initial));
    }

    #[test]
    fn pi_style_unsafe_change_above_viewport_triggers_full_clear() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));
        backend
            .draw_frame(frame(
                Size::new(80, 3),
                &["one", "two", "three", "four"],
                hidden_cursor(),
            ))
            .expect("initial draw");
        let before = backend.writer().len();

        backend
            .draw_frame(frame(
                Size::new(80, 3),
                &["ONE", "two", "three", "four"],
                hidden_cursor(),
            ))
            .expect("recovery draw");

        let output = String::from_utf8_lossy(&backend.writer()[before..]);
        assert!(output.contains(CSI_CLEAR_SCREEN));
        assert!(output.contains(CSI_CLEAR_SCROLLBACK));
        assert_eq!(backend.last_update_kind(), Some(FrameUpdateKind::Recovery));
    }

    #[test]
    fn pi_style_append_scrolls_by_viewport_delta_and_repaints_tail() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));
        backend
            .draw_frame(frame(Size::new(80, 3), &["one", "two"], hidden_cursor()))
            .expect("initial draw");
        let before = backend.writer().len();

        backend
            .draw_frame(frame(
                Size::new(80, 3),
                &["one", "two", "three", "four"],
                hidden_cursor(),
            ))
            .expect("append draw");

        let output = String::from_utf8_lossy(&backend.writer()[before..]);
        assert!(output.contains("\r\n"));
        assert!(output.contains("\u{1b}[2Ktwo\u{1b}[0m"));
        assert!(output.contains("\u{1b}[2Kthree\u{1b}[0m"));
        assert!(output.contains("\u{1b}[2Kfour\u{1b}[0m"));
        assert!(!output.contains(CSI_CLEAR_SCREEN));
        assert_eq!(backend.previous_viewport_top, 1);
    }

    #[test]
    fn pi_style_multi_row_tail_append_scrolls_by_full_viewport_delta() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));
        backend
            .draw_frame(frame(
                Size::new(80, 3),
                &["one", "two", "three"],
                hidden_cursor(),
            ))
            .expect("initial draw");
        let before = backend.writer().len();

        backend
            .draw_frame(frame(
                Size::new(80, 3),
                &["one", "two", "three", "four", "five"],
                hidden_cursor(),
            ))
            .expect("append draw");

        let output = String::from_utf8_lossy(&backend.writer()[before..]);
        assert_eq!(output.matches("\r\n").count(), 2);
        assert!(output.contains("\u{1b}[2Kthree\u{1b}[0m"));
        assert!(output.contains("\u{1b}[2Kfour\u{1b}[0m"));
        assert!(output.contains("\u{1b}[2Kfive\u{1b}[0m"));
        assert_eq!(backend.previous_viewport_top, 2);
    }

    #[test]
    fn pi_style_short_tail_append_creates_terminal_room_before_repaint() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 5));
        backend
            .draw_frame(frame(Size::new(80, 5), &["one", "two"], hidden_cursor()))
            .expect("initial draw");
        let before = backend.writer().len();

        backend
            .draw_frame(frame(
                Size::new(80, 5),
                &["one", "two", "three"],
                hidden_cursor(),
            ))
            .expect("append draw");

        let output = String::from_utf8_lossy(&backend.writer()[before..]);
        let scroll_index = output.find("\r\n").expect("append creates terminal room");
        let repaint_index = output.find("three").expect("tail row is repainted");
        assert!(
            scroll_index < repaint_index,
            "terminal room should be created before repainting the appended row: {output:?}"
        );
        assert!(output.contains("\u{1b}[2Kone\u{1b}[0m"));
        assert!(output.contains("\u{1b}[2Ktwo\u{1b}[0m"));
        assert!(output.contains("\u{1b}[2Kthree\u{1b}[0m"));
        assert!(!output.contains(CSI_CLEAR_SCREEN));
        assert_eq!(backend.previous_viewport_top, 0);
    }

    #[test]
    fn pi_style_safe_shrink_clears_stale_visible_rows_without_full_clear() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 5));
        backend
            .draw_frame(frame(
                Size::new(80, 5),
                &["one", "two", "three", "four", "five"],
                hidden_cursor(),
            ))
            .expect("initial draw");
        let before = backend.writer().len();

        backend
            .draw_frame(frame(
                Size::new(80, 5),
                &["one", "two", "THREE"],
                hidden_cursor(),
            ))
            .expect("shrink draw");

        let output = String::from_utf8_lossy(&backend.writer()[before..]);
        assert!(!output.contains(CSI_CLEAR_SCREEN));
        assert!(output.matches(CSI_CLEAR_LINE).count() >= 3);
        assert!(output.contains("THREE"));
        assert_eq!(
            backend.previous_lines,
            vec!["one".to_string(), "two".to_string(), "THREE".to_string()]
        );
    }

    #[test]
    fn pi_style_cursor_positioning_uses_relative_moves() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));
        backend
            .draw_frame(frame(
                Size::new(80, 3),
                &["one", "two", "three"],
                hidden_cursor(),
            ))
            .expect("initial draw");
        let before = backend.writer().len();

        let cursor = CursorState {
            position: Some(ratatui::layout::Position::new(2, 1)),
            style: FrameCursorStyle::Bar,
            visible: true,
        };
        backend
            .draw_frame(frame(Size::new(80, 3), &["one", "TWO", "three"], cursor))
            .expect("cursor draw");

        let output = String::from_utf8_lossy(&backend.writer()[before..]);
        assert!(!output.contains("\u{1b}[2;3H"));
        assert!(output.contains("\u{1b}[2G") || output.contains("\u{1b}[3G"));
        assert!(output.contains(CSI_SHOW_CURSOR));
    }

    #[test]
    fn pi_style_zero_sized_terminal_does_not_mutate_state() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(80, 3));
        backend
            .draw_frame(frame(Size::new(0, 0), &["one"], hidden_cursor()))
            .expect("zero draw");

        assert!(backend.writer().is_empty());
        assert!(backend.previous_lines.is_empty());
        assert_eq!(backend.previous_width, 80);
        assert_eq!(backend.previous_height, 3);
    }

    #[test]
    fn pi_style_rejects_rows_wider_than_terminal_width() {
        let mut backend = PiStyleMainScreenBackend::new(Vec::<u8>::new(), Size::new(3, 3));

        let error = backend
            .draw_frame(frame(Size::new(3, 3), &["abcd"], hidden_cursor()))
            .expect_err("over-wide row should fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("exceeds terminal width"));
        assert!(backend.writer().is_empty());
        assert!(backend.previous_lines.is_empty());
    }
}
