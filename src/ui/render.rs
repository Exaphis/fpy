use anyhow::Result;
use crossterm::cursor::position;
use ratatui::{
    layout::{Constraint, Layout, Rect},
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

pub(super) fn status_throbber(status: KernelStatus) -> Option<Throbber<'static>> {
    match status {
        KernelStatus::Connecting => Some(
            Throbber::default()
                .label("Connecting to kernel...")
                .style(Style::default())
                .throbber_style(Style::default().fg(Color::Yellow))
                .throbber_set(BRAILLE_SIX)
                .use_type(WhichUse::Spin),
        ),
        KernelStatus::Busy => Some(
            Throbber::default()
                .label("Kernel busy. Ctrl-C to interrupt")
                .style(Style::default())
                .throbber_style(Style::default().fg(Color::Yellow))
                .throbber_set(BRAILLE_SIX)
                .use_type(WhichUse::Spin),
        ),
        _ => None,
    }
}

pub(super) fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_percent) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage((100 - height_percent) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - width_percent) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage((100 - width_percent) / 2),
    ])
    .split(vertical[1])[1]
}

pub(super) fn editor_status_height() -> u16 {
    EDITOR_STATUS_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::status_throbber;
    use crate::kernel::KernelStatus;

    #[test]
    fn renders_busy_spinner_status() {
        assert!(status_throbber(KernelStatus::Busy).is_some());
    }
}
