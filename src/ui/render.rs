use anyhow::Result;
use crossterm::cursor::position;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};
use throbber_widgets_tui::{BRAILLE_SIX, Throbber, WhichUse};

use crate::{custom_terminal::DefaultTerminal, kernel::KernelStatus};

const MIN_VIEWPORT_HEIGHT: u16 = 1;
const PALETTE_HEIGHT: u16 = 6;
const EDITOR_STATUS_HEIGHT: u16 = 1;

pub(super) fn initial_pane_top() -> u16 {
    position()
        .map(|(_, row)| row.saturating_add(1))
        .unwrap_or(0)
}

pub(super) fn build_terminal(pane: Rect) -> Result<DefaultTerminal> {
    let mut terminal =
        DefaultTerminal::with_options(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;
    terminal.set_viewport_area(pane);
    Ok(terminal)
}

pub(super) fn pane_rect_at(width: u16, height: u16, pane_top: u16, pane_height: u16) -> Rect {
    let top = pane_top.min(max_pane_top(height, pane_height));
    let pane_height = pane_height.min(height.saturating_sub(top).max(1));
    Rect::new(0, top, width, pane_height)
}

pub(super) fn max_pane_top(height: u16, pane_height: u16) -> u16 {
    height.saturating_sub(pane_height.min(height.max(1)))
}

pub(super) fn viewport_height_for_editor(pane_rows: u16, palette_open: bool) -> u16 {
    let pane_rows = pane_rows.max(1);
    if palette_open {
        pane_rows.max(PALETTE_HEIGHT)
    } else {
        pane_rows
    }
    .max(MIN_VIEWPORT_HEIGHT)
}

pub(super) fn editor_visible_line_count(editor_line_count: usize) -> usize {
    editor_line_count.max(1)
}

pub(super) fn status_line_for(status: KernelStatus) -> Option<Line<'static>> {
    match status {
        KernelStatus::Disconnected => Some(Line::from(Span::styled(
            "Kernel disconnected",
            Style::default().fg(Color::Red),
        ))),
        _ => None,
    }
}

pub(super) fn transient_status_label(status: KernelStatus) -> Option<&'static str> {
    match status {
        KernelStatus::Connecting => Some("Connecting to kernel..."),
        KernelStatus::Busy => Some("Kernel busy. Ctrl-C to interrupt"),
        _ => None,
    }
}

pub(super) fn status_throbber(status: KernelStatus) -> Option<Throbber<'static>> {
    match status {
        KernelStatus::Connecting => Some(
            Throbber::default()
                .label("")
                .style(Style::default())
                .throbber_style(Style::default().fg(Color::Yellow))
                .throbber_set(BRAILLE_SIX)
                .use_type(WhichUse::Spin),
        ),
        KernelStatus::Busy => Some(
            Throbber::default()
                .label("")
                .style(Style::default())
                .throbber_style(Style::default().fg(Color::Yellow))
                .throbber_set(BRAILLE_SIX)
                .use_type(WhichUse::Spin),
        ),
        _ => None,
    }
}

pub(super) fn editor_status_height() -> u16 {
    EDITOR_STATUS_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::{status_throbber, transient_status_label};
    use crate::kernel::KernelStatus;

    #[test]
    fn renders_busy_spinner_status() {
        assert!(status_throbber(KernelStatus::Busy).is_some());
    }

    #[test]
    fn renders_busy_status_label() {
        assert_eq!(
            transient_status_label(KernelStatus::Busy),
            Some("Kernel busy. Ctrl-C to interrupt")
        );
    }
}
