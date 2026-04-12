mod editor;
mod render;
mod transcript;

use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, MoveToColumn, Show},
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    queue,
    style::ResetColor,
    terminal::{self, Clear, ClearType, ScrollUp},
    terminal::{EnableLineWrap, disable_raw_mode, enable_raw_mode},
};
use edtui::{
    EditorEventHandler, EditorMode, EditorState, EditorView,
    actions::{Chainable, InsertChar, LineBreak, SwitchMode},
};
use futures::StreamExt;
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear as ClearWidget, Paragraph},
};
use std::io::{Write, stdout};
use throbber_widgets_tui::ThrobberState;

use self::{
    editor::{
        build_editor_state, editor_gutter_lines, editor_gutter_width, editor_palette_hint,
        editor_palette_hint_width, editor_status_prefix, editor_status_prefix_width,
        editor_syntax_highlighter, editor_theme, indent_width, move_editor_to_row, status_label,
    },
    render::{
        build_terminal, editor_status_height, editor_visible_line_count, initial_pane_top,
        max_pane_top, pane_rect_at, status_line_for, status_throbber, transient_status_label,
        viewport_height_for_editor,
    },
    transcript::highlighted_execute_input,
};
use crate::custom_terminal::DefaultTerminal;
use crate::insert_history::insert_history_text;
use crate::kernel::KernelStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteAction {
    Quit,
    InterruptKernel,
    RestartKernel,
    ClearInput,
    ShowConnectionInfo,
}

impl PaletteAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::InterruptKernel => "Interrupt Kernel",
            Self::RestartKernel => "Restart Kernel",
            Self::ClearInput => "Clear Input",
            Self::ShowConnectionInfo => "Show Connection Info",
        }
    }
}

#[derive(Debug)]
pub enum UiAction {
    Submit(String),
    ReplyInput(String),
    Interrupt,
    ClearScreen,
    Exit,
    Restart,
    ShowConnectionInfo,
}

pub struct AppUi {
    terminal: Option<DefaultTerminal>,
    events: EventStream,
    current_pane: Rect,
    pane_top: u16,
    editor: EditorState,
    editor_events: EditorEventHandler,
    pending_normal_count: String,
    history: Vec<String>,
    history_index: Option<usize>,
    palette_open: bool,
    palette_index: usize,
    awaiting_input: Option<(String, bool)>,
    last_execution_count: Option<u32>,
    status: KernelStatus,
    connection_summary: String,
    throbber_state: ThrobberState,
    session_ready: bool,
    last_palette_open: bool,
    restored: bool,
}

impl AppUi {
    pub fn new(connection_summary: String) -> Result<Self> {
        enable_raw_mode()?;
        let _ = execute!(
            stdout(),
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        );
        let (width, height) = terminal::size()?;
        let pane_top = initial_pane_top();
        let pane = pane_rect_at(
            width,
            height,
            pane_top,
            viewport_height_for_editor(1, false),
        );
        let terminal = build_terminal(pane)?;

        Ok(Self {
            terminal: Some(terminal),
            events: EventStream::new(),
            current_pane: pane,
            pane_top,
            editor: build_editor_state(""),
            editor_events: EditorEventHandler::default(),
            pending_normal_count: String::new(),
            history: Vec::new(),
            history_index: None,
            palette_open: false,
            palette_index: 0,
            awaiting_input: None,
            last_execution_count: None,
            status: KernelStatus::Connecting,
            connection_summary,
            throbber_state: ThrobberState::default(),
            session_ready: false,
            last_palette_open: false,
            restored: false,
        })
    }

    pub fn connection_summary(&self) -> &str {
        &self.connection_summary
    }

    pub fn set_connection_summary(&mut self, summary: String) {
        self.connection_summary = summary;
    }

    pub fn set_status(&mut self, status: KernelStatus) {
        if self.session_ready && status == KernelStatus::Connecting {
            return;
        }
        self.status = status;
        if status == KernelStatus::Disconnected {
            self.session_ready = false;
        }
    }

    pub fn set_last_execution_count(&mut self, count: Option<u32>) {
        if let Some(count) = count {
            self.last_execution_count = Some(count);
        }
    }

    pub fn needs_animation(&self) -> bool {
        self.status_spins()
    }

    pub fn begin_input_request(&mut self, prompt: String, password: bool) {
        self.awaiting_input = Some((prompt, password));
        self.reset_editor();
        self.status = KernelStatus::AwaitingInput;
    }

    pub fn clear_input_request(&mut self) {
        self.awaiting_input = None;
    }

    pub fn mark_session_ready(&mut self) {
        self.session_ready = true;
        self.status = KernelStatus::Idle;
    }

    pub fn insert_transcript(&mut self, text: impl Into<String>) -> Result<()> {
        self.sync_viewport()?;
        let pane = {
            let terminal = self.terminal_mut()?;
            insert_history_text(terminal, &text.into())?;
            terminal.viewport_area()
        };
        self.current_pane = pane;
        self.pane_top = pane.y;
        Ok(())
    }

    pub fn insert_execute_input(&mut self, execution_count: Option<u32>, code: &str) -> Result<()> {
        self.insert_transcript(highlighted_execute_input(execution_count, code))
    }

    pub fn shutdown(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        let pane = self.pane_rect()?;
        if let Some(mut terminal) = self.terminal.take() {
            let _ = terminal.show_cursor();
        }

        let mut handle = stdout();
        let _ = execute!(handle, DisableBracketedPaste, PopKeyboardEnhancementFlags);
        write!(handle, "\x1b[r\x1b[0m")?;
        execute!(handle, Show, ResetColor, EnableLineWrap)?;
        for row in pane.y..pane.bottom() {
            execute!(
                handle,
                crossterm::cursor::MoveTo(0, row),
                Clear(ClearType::UntilNewLine)
            )?;
        }
        execute!(
            handle,
            crossterm::cursor::MoveTo(0, pane.y),
            MoveToColumn(0)
        )?;
        handle.flush()?;
        disable_raw_mode()?;
        execute!(handle, ResetColor, EnableLineWrap, Show, MoveToColumn(0))?;
        handle.flush()?;
        self.restored = true;
        Ok(())
    }

    pub fn clear_screen(&mut self) -> Result<()> {
        let (width, height) = terminal::size()?;
        let pane = pane_rect_at(width, height, 0, self.pane_height());
        let terminal = self.terminal_mut()?;
        execute!(
            terminal.backend_mut(),
            crossterm::terminal::Clear(ClearType::All),
            MoveTo(0, 0)
        )?;
        terminal.set_viewport_area(pane);
        terminal.invalidate_viewport();
        self.current_pane = pane;
        self.pane_top = pane.y;
        Ok(())
    }

    pub fn redraw(&mut self) -> Result<()> {
        self.sync_viewport()?;
        let awaiting_input = self.awaiting_input.clone();
        let palette_open = self.palette_open;
        let palette_index = self.palette_index;
        let input_active = self.input_active();
        let prompt_label = self.prompt_label();
        let visible_lines = self.editor_visible_line_count();
        let status = self.status;
        let transient_status = if self.session_ready && status == KernelStatus::Connecting {
            None
        } else {
            transient_status_label(status)
        };

        if palette_open != self.last_palette_open {
            self.clear_current_pane_rows()?;
            self.terminal_mut()?.invalidate_viewport();
        }

        let AppUi {
            terminal,
            throbber_state,
            editor,
            ..
        } = self;
        let terminal = terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("terminal is not active"))?;
        if input_active && !palette_open {
            terminal.show_cursor()?;
        } else {
            terminal.hide_cursor()?;
        }

        terminal.draw(|frame| {
            let area = frame.area();
            if input_active {
                let [content_area, status_area] = Layout::vertical([
                    Constraint::Min(1),
                    Constraint::Length(editor_status_height()),
                ])
                .areas(area);
                if palette_open {
                    frame.render_widget(ClearWidget, area);
                    let palette_width = content_area.width.min(56).max(24);
                    let palette_height = content_area.height.max(3);
                    let palette_area = Rect::new(
                        content_area.x,
                        content_area.y,
                        palette_width,
                        palette_height,
                    );
                    frame.render_widget(
                        Block::default()
                            .title("Command Palette")
                            .borders(Borders::ALL),
                        palette_area,
                    );

                    let inner = palette_area.inner(Margin {
                        vertical: 1,
                        horizontal: 1,
                    });
                    let inner_width = usize::from(inner.width);
                    let buffer = frame.buffer_mut();
                    for row in 0..inner.height as usize {
                        let (text, style) = if let Some(action) = palette_items().get(row).copied() {
                            let style = if row == palette_index {
                                Style::default().fg(Color::Black).bg(Color::Yellow)
                            } else {
                                Style::default()
                            };
                            let text = if inner_width == 0 {
                                action.label().to_string()
                            } else {
                                format!("{:<width$}", action.label(), width = inner_width)
                            };
                            (text, style)
                        } else {
                            (" ".repeat(inner_width), Style::default())
                        };
                        buffer.set_stringn(inner.x, inner.y + row as u16, text, inner_width, style);
                    }
                } else {
                    frame.render_widget(ClearWidget, content_area);
                    frame.render_widget(ClearWidget, status_area);
                    let gutter_width =
                        editor_gutter_width(&awaiting_input, visible_lines).min(content_area.width);
                    let [gutter_area, content_area] =
                        Layout::horizontal([Constraint::Length(gutter_width), Constraint::Min(1)])
                            .areas(content_area);
                    let gutter_lines = editor_gutter_lines(
                        &awaiting_input,
                        gutter_area.height as usize,
                        visible_lines,
                    );
                    frame.render_widget(Paragraph::new(gutter_lines), gutter_area);

                    let editor_view = EditorView::new(editor)
                        .theme(editor_theme())
                        .tab_width(indent_width())
                        .wrap(false)
                        .syntax_highlighter(editor_syntax_highlighter())
                        .single_line(false);
                    frame.render_widget(editor_view, content_area);
                }
                if let Some(throbber) = status_throbber(status) {
                    let status_detail = status_label(&awaiting_input, &prompt_label);
                    let prefix_width = editor_status_prefix_width(editor.mode, status_detail);
                    let transient_width = transient_status
                        .map(|label| u16::try_from(label.chars().count()).unwrap_or(u16::MAX))
                        .unwrap_or(0);
                    let palette_hint_width = editor_palette_hint_width();
                    let [status_text_area, spinner_gap_area, spinner_area, transient_area, filler_area, palette_hint_area] =
                        Layout::horizontal([
                            Constraint::Length(prefix_width),
                            Constraint::Length(1),
                            Constraint::Length(2),
                            Constraint::Length(transient_width),
                            Constraint::Min(1),
                            Constraint::Length(palette_hint_width),
                        ])
                        .areas(status_area);
                    frame.render_widget(
                        editor_status_prefix(editor.mode, status_detail),
                        status_text_area,
                    );
                    frame.render_widget(Paragraph::new(" "), spinner_gap_area);
                    frame.render_stateful_widget(throbber, spinner_area, throbber_state);
                    if let Some(transient_status) = transient_status {
                        frame.render_widget(
                            Paragraph::new(Line::from(Span::styled(
                                transient_status.to_string(),
                                Style::default().fg(Color::Yellow),
                            ))),
                            transient_area,
                        );
                    }
                    frame.render_widget(Paragraph::new(""), filler_area);
                    frame.render_widget(editor_palette_hint(), palette_hint_area);
                } else {
                    let status_detail = status_label(&awaiting_input, &prompt_label);
                    let prefix_width = editor_status_prefix_width(editor.mode, status_detail);
                    let palette_hint_width = editor_palette_hint_width();
                    let [status_text_area, filler_area, palette_hint_area] = Layout::horizontal([
                        Constraint::Length(prefix_width),
                        Constraint::Min(1),
                        Constraint::Length(palette_hint_width),
                    ])
                    .areas(status_area);
                    frame.render_widget(
                        editor_status_prefix(editor.mode, status_detail),
                        status_text_area,
                    );
                    frame.render_widget(Paragraph::new(""), filler_area);
                    frame.render_widget(editor_palette_hint(), palette_hint_area);
                }

                if !palette_open
                    && let Some(position) = editor.cursor_screen_position()
                {
                    frame.set_cursor_position(position);
                }
            } else if let Some(status_line) = status_line_for(status) {
                frame.render_widget(Paragraph::new(status_line), area);
            } else {
                frame.render_widget(Paragraph::new(""), area);
            }
        })?;

        if self.status_spins() {
            self.throbber_state.calc_next();
        }
        self.last_palette_open = palette_open;

        Ok(())
    }

    pub async fn next_action(&mut self) -> Result<Option<UiAction>> {
        while let Some(event) = self.events.next().await {
            match event? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if let Some(action) = self.handle_key(key) {
                        return Ok(Some(action));
                    }
                    return Ok(None);
                }
                Event::Paste(text) => {
                    if self.editor_enabled() {
                        let text = text.replace("\r\n", "\n").replace('\r', "\n");
                        self.editor_events.on_paste_event(text, &mut self.editor);
                    }
                    return Ok(None);
                }
                Event::Resize(_, _) => {
                    self.sync_viewport()?;
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        if self.palette_open {
            return self.handle_palette_key(key);
        }

        if self.editor_enabled()
            && matches!(self.editor.mode, EditorMode::Normal | EditorMode::Visual)
        {
            if let Some(action) = self.handle_normal_mode_count_prefix(key) {
                return action;
            }
        } else {
            self.pending_normal_count.clear();
        }

        match key {
            KeyEvent {
                code: KeyCode::Char('p'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                self.palette_open = true;
                None
            }
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                if self.submit_ready() {
                    let _ = self.clear_editor_view();
                    None
                } else {
                    Some(UiAction::Interrupt)
                }
            }
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                && self.editor_enabled()
                && self.editor_is_empty() =>
            {
                Some(UiAction::Exit)
            }
            KeyEvent {
                code: KeyCode::Char('l'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => Some(UiAction::ClearScreen),
            KeyEvent {
                code: KeyCode::Tab, ..
            } if self.editor_enabled() && self.editor.mode == EditorMode::Insert => {
                for _ in 0..indent_width() {
                    self.editor.execute(InsertChar(' '));
                }
                None
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers,
                ..
            } if self.editor_enabled() && modifiers.contains(KeyModifiers::SHIFT) => {
                self.editor
                    .execute(SwitchMode(EditorMode::Insert).chain(LineBreak(1)));
                None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } if self.submit_ready() => {
                let text = self.take_editor_text();
                if text.trim().is_empty() {
                    return None;
                }
                self.history.push(text.clone());
                self.history_index = None;

                if self.awaiting_input.is_some() {
                    self.awaiting_input = None;
                    Some(UiAction::ReplyInput(text))
                } else {
                    Some(UiAction::Submit(text))
                }
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } if self.editor_enabled()
                && self.editor_is_single_line()
                && !self.history.is_empty() =>
            {
                self.history_up();
                None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } if self.editor_enabled()
                && self.editor_is_single_line()
                && !self.history.is_empty() =>
            {
                self.history_down();
                None
            }
            key if self.editor_enabled() => {
                self.editor_events.on_key_event(key, &mut self.editor);
                None
            }
            _ => None,
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        self.pending_normal_count.clear();
        match key.code {
            KeyCode::Esc => {
                self.palette_open = false;
                None
            }
            KeyCode::Up => {
                self.palette_index = self.palette_index.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                self.palette_index = (self.palette_index + 1).min(palette_items().len() - 1);
                None
            }
            KeyCode::Enter => {
                self.palette_open = false;
                match palette_items()[self.palette_index] {
                    PaletteAction::Quit => Some(UiAction::Exit),
                    PaletteAction::InterruptKernel => Some(UiAction::Interrupt),
                    PaletteAction::RestartKernel => Some(UiAction::Restart),
                    PaletteAction::ClearInput => {
                        let _ = self.clear_editor_view();
                        None
                    }
                    PaletteAction::ShowConnectionInfo => Some(UiAction::ShowConnectionInfo),
                }
            }
            _ => None,
        }
    }

    fn history_up(&mut self) {
        let next = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => self.history.len().saturating_sub(1),
        };
        self.history_index = Some(next);
        let text = self.history[next].clone();
        self.set_editor_text(&text);
    }

    fn history_down(&mut self) {
        match self.history_index {
            Some(index) if index + 1 < self.history.len() => {
                self.history_index = Some(index + 1);
                let text = self.history[index + 1].clone();
                self.set_editor_text(&text);
            }
            Some(_) => {
                self.history_index = None;
                let _ = self.clear_editor_view();
            }
            None => {}
        }
    }

    fn sync_viewport(&mut self) -> Result<()> {
        let (width, height) = terminal::size()?;
        let pane_height = self.pane_height().min(height.max(1));
        let pane_top = self.pane_top.min(max_pane_top(height, pane_height));
        let pane = pane_rect_at(width, height, pane_top, pane_height);
        if pane != self.current_pane {
            self.scroll_history_for_pane_growth(self.current_pane, pane, height)?;
            self.clear_stale_pane_rows(self.current_pane, pane)?;
            let terminal = self.terminal_mut()?;
            terminal.set_viewport_area(pane);
            terminal.invalidate_viewport();
            self.current_pane = pane;
            self.pane_top = pane.y;
        }
        Ok(())
    }

    fn pane_rect(&self) -> Result<Rect> {
        if self.terminal.is_none() {
            return Err(anyhow::anyhow!("terminal is not active"));
        }
        Ok(self.current_pane)
    }

    fn terminal_mut(&mut self) -> Result<&mut DefaultTerminal> {
        self.terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("terminal is not active"))
    }

    fn pane_height(&self) -> u16 {
        let input_rows = if self.editor_enabled() {
            self.editor_visible_line_count() as u16 + editor_status_height()
        } else {
            1 + editor_status_height()
        };
        viewport_height_for_editor(input_rows, self.palette_open)
    }

    fn input_active(&self) -> bool {
        self.editor_enabled()
    }

    fn editor_enabled(&self) -> bool {
        !matches!(self.status, KernelStatus::Disconnected)
    }

    fn submit_ready(&self) -> bool {
        self.session_ready
            && !matches!(self.status, KernelStatus::Busy | KernelStatus::Disconnected)
    }

    fn prompt_label(&self) -> String {
        match self.last_execution_count {
            Some(count) => format!("In [{}]", count.saturating_add(1)),
            None => "In [1]".to_string(),
        }
    }

    fn status_spins(&self) -> bool {
        matches!(self.status, KernelStatus::Busy)
            || (self.status == KernelStatus::Connecting && !self.session_ready)
    }

    fn editor_visible_line_count(&self) -> usize {
        editor_visible_line_count(self.editor.lines.len())
    }

    fn editor_is_empty(&self) -> bool {
        self.editor.lines.to_string().is_empty()
    }

    fn editor_is_single_line(&self) -> bool {
        self.editor.lines.len() <= 1
    }

    fn reset_editor(&mut self) {
        self.editor = build_editor_state("");
        self.editor_events = EditorEventHandler::default();
        self.pending_normal_count.clear();
    }

    fn set_editor_text(&mut self, text: &str) {
        self.editor = build_editor_state(text);
        self.editor_events = EditorEventHandler::default();
        self.pending_normal_count.clear();
    }

    fn take_editor_text(&mut self) -> String {
        let text = self.editor.lines.to_string();
        self.reset_editor();
        text
    }

    fn clear_editor_view(&mut self) -> Result<()> {
        self.clear_current_pane_rows()?;
        self.reset_editor();
        self.terminal_mut()?.invalidate_viewport();
        Ok(())
    }

    fn clear_stale_pane_rows(&mut self, old: Rect, new: Rect) -> Result<()> {
        if old.is_empty() {
            return Ok(());
        }

        let terminal = self.terminal_mut()?;
        let handle = terminal.backend_mut();
        for row in old.y..old.bottom() {
            if row < new.y || row >= new.bottom() {
                execute!(handle, MoveTo(0, row), Clear(ClearType::UntilNewLine))?;
            }
        }
        Ok(())
    }

    fn clear_current_pane_rows(&mut self) -> Result<()> {
        let pane = self.current_pane;
        if pane.is_empty() {
            return Ok(());
        }

        let terminal = self.terminal_mut()?;
        let handle = terminal.backend_mut();
        for row in pane.y..pane.bottom() {
            execute!(handle, MoveTo(0, row), Clear(ClearType::UntilNewLine))?;
        }
        handle.flush()?;
        Ok(())
    }

    fn scroll_history_for_pane_growth(&mut self, old: Rect, new: Rect, screen_height: u16) -> Result<()> {
        if old.is_empty()
            || new.height <= old.height
            || old.bottom() != screen_height
            || new.bottom() != screen_height
            || old.y == 0
        {
            return Ok(());
        }

        let growth = new.height.saturating_sub(old.height);
        let history_bottom = old.y.saturating_sub(1);
        let terminal = self.terminal_mut()?;
        let handle = terminal.backend_mut();
        queue!(
            handle,
            crossterm::style::Print(set_scroll_region(0, history_bottom)),
            MoveTo(0, history_bottom),
            ScrollUp(growth),
            crossterm::style::Print(reset_scroll_region()),
        )?;
        handle.flush()?;
        Ok(())
    }

    fn handle_normal_mode_count_prefix(&mut self, key: KeyEvent) -> Option<Option<UiAction>> {
        match key.code {
            KeyCode::Char(digit)
                if digit.is_ascii_digit()
                    && (!self.pending_normal_count.is_empty() || digit != '0') =>
            {
                self.pending_normal_count.push(digit);
                Some(None)
            }
            KeyCode::Char('G') if !self.pending_normal_count.is_empty() => {
                let count = self.pending_normal_count.parse::<usize>().ok();
                self.pending_normal_count.clear();
                if let Some(target_row) = count.and_then(|n| n.checked_sub(1)) {
                    move_editor_to_row(&mut self.editor, target_row);
                }
                Some(None)
            }
            _ => {
                self.pending_normal_count.clear();
                None
            }
        }
    }
}

impl Drop for AppUi {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.shutdown();
        }
    }
}

fn palette_items() -> [PaletteAction; 5] {
    [
        PaletteAction::Quit,
        PaletteAction::InterruptKernel,
        PaletteAction::RestartKernel,
        PaletteAction::ClearInput,
        PaletteAction::ShowConnectionInfo,
    ]
}

fn set_scroll_region(top: u16, bottom: u16) -> String {
    format!(
        "\x1b[{};{}r",
        top.saturating_add(1),
        bottom.saturating_add(1)
    )
}

fn reset_scroll_region() -> &'static str {
    "\x1b[r"
}
