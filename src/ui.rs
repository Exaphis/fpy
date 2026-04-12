use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, MoveToColumn, Show, position},
    event::{
        Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    style::ResetColor,
    terminal::{self, Clear, ClearType},
    terminal::{EnableLineWrap, disable_raw_mode, enable_raw_mode},
};
use edtui::{
    EditorEventHandler, EditorMode, EditorState, EditorTheme, EditorView, Index2, Lines,
    SyntaxHighlighter,
    actions::{Chainable, InsertChar, LineBreak, SwitchMode},
    syntect::{
        easy::HighlightLines,
        highlighting::ThemeSet,
        parsing::SyntaxSet,
        util::{LinesWithEndings, as_24_bit_terminal_escaped},
    },
};
use futures::StreamExt;
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear as ClearWidget, List, ListItem, Paragraph},
};
use std::{
    io::{Write, stdout},
    sync::LazyLock,
};
use throbber_widgets_tui::{BRAILLE_SIX, Throbber, ThrobberState, WhichUse};

use crate::custom_terminal::DefaultTerminal;
use crate::insert_history::insert_history_text;
use crate::kernel::KernelStatus;

const MIN_VIEWPORT_HEIGHT: u16 = 1;
const MAX_VIEWPORT_HEIGHT: u16 = 8;
const PALETTE_HEIGHT: u16 = 6;
const MAX_INPUT_LINES: usize = 4;
const EDITOR_STATUS_HEIGHT: u16 = 1;
const INDENT_WIDTH: usize = 4;
const EDITOR_THEME_NAME: &str = "base16-ocean-dark";
const TRANSCRIPT_THEME_NAME: &str = "base16-ocean.dark";
const PROMPT_ANSI: &str = "\x1b[36m";
const ANSI_RESET: &str = "\x1b[0m";

static TRANSCRIPT_SYNTAX_SET: LazyLock<SyntaxSet> =
    LazyLock::new(SyntaxSet::load_defaults_newlines);
static TRANSCRIPT_THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

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
    restored: bool,
}

impl AppUi {
    pub fn new(connection_summary: String) -> Result<Self> {
        enable_raw_mode()?;
        let _ = execute!(
            stdout(),
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
        self.status = status;
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
        let _ = execute!(handle, PopKeyboardEnhancementFlags);
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
                let [editor_area, status_area] = Layout::vertical([
                    Constraint::Min(1),
                    Constraint::Length(EDITOR_STATUS_HEIGHT),
                ])
                .areas(area);
                let gutter_width =
                    editor_gutter_width(&awaiting_input, visible_lines).min(editor_area.width);
                let [gutter_area, content_area] =
                    Layout::horizontal([Constraint::Length(gutter_width), Constraint::Min(1)])
                        .areas(editor_area);
                let gutter_lines = editor_gutter_lines(
                    &awaiting_input,
                    gutter_area.height as usize,
                    visible_lines,
                );
                frame.render_widget(Paragraph::new(gutter_lines), gutter_area);

                let editor_view = EditorView::new(editor)
                    .theme(editor_theme())
                    .tab_width(INDENT_WIDTH)
                    .wrap(false)
                    .syntax_highlighter(editor_syntax_highlighter())
                    .single_line(false);
                frame.render_widget(editor_view, content_area);
                frame.render_widget(
                    editor_status_line(editor.mode, status_label(&awaiting_input, &prompt_label)),
                    status_area,
                );

                if let Some(position) = editor.cursor_screen_position() {
                    frame.set_cursor_position(position);
                }
            } else if let Some(throbber) = status_throbber(status) {
                frame.render_stateful_widget(throbber, area, throbber_state);
            } else if let Some(status_line) = status_line_for(status) {
                frame.render_widget(Paragraph::new(status_line), area);
            } else {
                frame.render_widget(Paragraph::new(""), area);
            }

            if palette_open {
                let popup = centered_rect(60, 70, area);
                frame.render_widget(ClearWidget, popup);
                let items = palette_items()
                    .into_iter()
                    .enumerate()
                    .map(|(index, action)| {
                        let style = if index == palette_index {
                            Style::default().fg(Color::Black).bg(Color::Yellow)
                        } else {
                            Style::default()
                        };
                        ListItem::new(Line::from(Span::styled(action.label(), style)))
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(
                    List::new(items).block(
                        Block::default()
                            .title("Command Palette")
                            .borders(Borders::ALL),
                    ),
                    popup.inner(Margin {
                        vertical: 0,
                        horizontal: 0,
                    }),
                );
            }
        })?;

        if self.status_spins() {
            self.throbber_state.calc_next();
        }

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
                    if self.input_active() {
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

        if self.input_active()
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
                if self.input_active() {
                    self.reset_editor();
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
                && self.input_active()
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
            } if self.input_active() && self.editor.mode == EditorMode::Insert => {
                for _ in 0..INDENT_WIDTH {
                    self.editor.execute(InsertChar(' '));
                }
                None
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers,
                ..
            } if self.input_active() && modifiers.contains(KeyModifiers::SHIFT) => {
                self.editor
                    .execute(SwitchMode(EditorMode::Insert).chain(LineBreak(1)));
                None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } if self.input_active() => {
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
            } if self.input_active()
                && self.editor_is_single_line()
                && !self.history.is_empty() =>
            {
                self.history_up();
                None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } if self.input_active()
                && self.editor_is_single_line()
                && !self.history.is_empty() =>
            {
                self.history_down();
                None
            }
            key if self.input_active() => {
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
                        self.reset_editor();
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
                self.reset_editor();
            }
            None => {}
        }
    }

    fn sync_viewport(&mut self) -> Result<()> {
        let (width, height) = terminal::size()?;
        let pane_height = self.pane_height();
        self.pane_top = self.pane_top.min(max_pane_top(height, pane_height));
        let pane = pane_rect_at(width, height, self.pane_top, pane_height);
        if pane != self.current_pane {
            self.clear_stale_pane_rows(self.current_pane, pane)?;
            self.terminal_mut()?.set_viewport_area(pane);
            self.current_pane = pane;
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
        let input_rows = if self.input_active() {
            self.editor_visible_line_count() as u16 + EDITOR_STATUS_HEIGHT
        } else {
            1 + EDITOR_STATUS_HEIGHT
        };
        viewport_height_for_editor(input_rows, self.palette_open)
    }

    fn input_active(&self) -> bool {
        matches!(
            self.status,
            KernelStatus::Idle | KernelStatus::AwaitingInput
        )
    }

    fn prompt_label(&self) -> String {
        match self.last_execution_count {
            Some(count) => format!("In [{}]", count.saturating_add(1)),
            None => "In [1]".to_string(),
        }
    }

    fn status_spins(&self) -> bool {
        matches!(self.status, KernelStatus::Connecting | KernelStatus::Busy)
    }

    fn editor_visible_line_count(&self) -> usize {
        self.editor.lines.len().min(MAX_INPUT_LINES).max(1)
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

fn initial_pane_top() -> u16 {
    position()
        .map(|(_, row)| row.saturating_add(1))
        .unwrap_or(0)
}

fn build_editor_state(text: &str) -> EditorState {
    let mut editor = EditorState::new(Lines::from(text));
    editor.mode = EditorMode::Insert;

    let rows = text.split('\n').collect::<Vec<_>>();
    let row = rows.len().saturating_sub(1);
    let col = rows.last().map_or(0, |line| line.chars().count());
    editor.cursor = Index2::new(row, col);
    editor
}

fn move_editor_to_row(editor: &mut EditorState, target_row: usize) {
    let max_row = editor.lines.len().saturating_sub(1);
    let target_row = target_row.min(max_row);
    let current_row = editor.cursor.row;
    if target_row >= current_row {
        editor.execute(edtui::actions::MoveDown(target_row - current_row));
    } else {
        editor.execute(edtui::actions::MoveUp(current_row - target_row));
    }
}

fn build_terminal(pane: Rect) -> Result<DefaultTerminal> {
    let mut terminal =
        DefaultTerminal::with_options(ratatui::backend::CrosstermBackend::new(stdout()))?;
    terminal.set_viewport_area(pane);
    Ok(terminal)
}

fn pane_rect_at(width: u16, height: u16, pane_top: u16, pane_height: u16) -> Rect {
    let top = pane_top.min(max_pane_top(height, pane_height));
    let pane_height = pane_height.min(height.saturating_sub(top).max(1));
    Rect::new(0, top, width, pane_height)
}

fn max_pane_top(height: u16, pane_height: u16) -> u16 {
    height.saturating_sub(pane_height.min(height.max(1)))
}

fn viewport_height_for_editor(pane_rows: u16, palette_open: bool) -> u16 {
    let pane_rows = pane_rows.max(1);
    let height = if palette_open {
        pane_rows.max(PALETTE_HEIGHT)
    } else {
        pane_rows
    };
    height.clamp(MIN_VIEWPORT_HEIGHT, MAX_VIEWPORT_HEIGHT)
}

fn prompt_prefixes(awaiting_input: &Option<(String, bool)>) -> Option<(String, String)> {
    match awaiting_input {
        Some((prompt, true)) => {
            let first = format!("stdin (hidden) {prompt}");
            let continuation = " ".repeat(display_width(&first));
            Some((first, continuation))
        }
        Some((prompt, false)) => {
            let first = prompt.clone();
            let continuation = " ".repeat(display_width(&first));
            Some((first, continuation))
        }
        None => None,
    }
}

fn status_line_for(status: KernelStatus) -> Option<Line<'static>> {
    match status {
        KernelStatus::Disconnected => Some(Line::from(Span::styled(
            "Kernel disconnected",
            Style::default().fg(Color::Red),
        ))),
        _ => None,
    }
}

fn status_throbber(status: KernelStatus) -> Option<Throbber<'static>> {
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

fn prompt_gutter_width(prompt_prefix: &str, continuation_prefix: &str) -> u16 {
    u16::try_from(display_width(prompt_prefix).max(display_width(continuation_prefix)))
        .unwrap_or(u16::MAX)
        .max(1)
}

fn prompt_gutter_lines(
    prompt_prefix: &str,
    continuation_prefix: &str,
    height: usize,
) -> Vec<Line<'static>> {
    let width = display_width(prompt_prefix).max(display_width(continuation_prefix));
    (0..height.max(1))
        .map(|index| {
            let prefix = if index == 0 {
                prompt_prefix
            } else {
                continuation_prefix
            };
            Line::from(Span::styled(
                format!("{prefix:<width$}"),
                Style::default().fg(Color::Cyan),
            ))
        })
        .collect()
}

fn line_number_gutter_width(visible_lines: usize) -> u16 {
    let digits = visible_lines.max(1).to_string().len();
    u16::try_from(digits + 1).unwrap_or(u16::MAX).max(2)
}

fn line_number_gutter_lines(height: usize, visible_lines: usize) -> Vec<Line<'static>> {
    let width = usize::from(line_number_gutter_width(visible_lines)).saturating_sub(1);
    (0..height.max(1))
        .map(|index| {
            let line_number = index + 1;
            Line::from(Span::styled(
                format!("{line_number:>width$} ", width = width),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect()
}

fn editor_gutter_width(awaiting_input: &Option<(String, bool)>, visible_lines: usize) -> u16 {
    if let Some((prompt_prefix, continuation_prefix)) = prompt_prefixes(awaiting_input) {
        prompt_gutter_width(&prompt_prefix, &continuation_prefix)
    } else {
        line_number_gutter_width(visible_lines)
    }
}

fn editor_gutter_lines(
    awaiting_input: &Option<(String, bool)>,
    height: usize,
    visible_lines: usize,
) -> Vec<Line<'static>> {
    if let Some((prompt_prefix, continuation_prefix)) = prompt_prefixes(awaiting_input) {
        prompt_gutter_lines(&prompt_prefix, &continuation_prefix, height)
    } else {
        line_number_gutter_lines(height, visible_lines)
    }
}

fn editor_theme() -> EditorTheme<'static> {
    EditorTheme::default()
        .base(Style::default())
        .cursor_style(Style::default().add_modifier(Modifier::REVERSED))
        .selection_style(Style::default().bg(Color::DarkGray))
        .hide_status_line()
}

fn editor_syntax_highlighter() -> Option<SyntaxHighlighter> {
    SyntaxHighlighter::new(EDITOR_THEME_NAME, "py").ok()
}

fn editor_status_line(mode: EditorMode, detail: Option<&str>) -> Paragraph<'static> {
    let (label, style) = editor_mode_badge(mode);

    let mut spans = vec![Span::styled(format!(" {label} "), style)];
    if let Some(detail) = detail {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            detail.to_string(),
            Style::default().fg(Color::Cyan),
        ));
    }
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        "Ctrl-P palette",
        Style::default().fg(Color::DarkGray),
    ));

    Paragraph::new(Line::from(spans))
}

fn editor_mode_badge(mode: EditorMode) -> (&'static str, Style) {
    match mode {
        EditorMode::Insert => ("INS", Style::default().fg(Color::Black).bg(Color::Cyan)),
        EditorMode::Normal => ("NAV", Style::default().fg(Color::Black).bg(Color::Yellow)),
        EditorMode::Visual => ("VIS", Style::default().fg(Color::Black).bg(Color::Magenta)),
        EditorMode::Search => ("SRCH", Style::default().fg(Color::Black).bg(Color::Green)),
    }
}

fn status_label<'a>(
    awaiting_input: &'a Option<(String, bool)>,
    prompt_label: &'a str,
) -> Option<&'a str> {
    if awaiting_input.is_some() {
        Some("stdin")
    } else {
        Some(prompt_label)
    }
}

fn highlighted_execute_input(execution_count: Option<u32>, code: &str) -> String {
    let prompt = execution_count
        .map(|count| format!("In [{count}]: "))
        .unwrap_or_else(|| "In [?]: ".to_string());
    let continuation = format!("{:>width$}", "...: ", width = prompt.len());
    let syntax = TRANSCRIPT_SYNTAX_SET
        .find_syntax_by_extension("py")
        .unwrap_or_else(|| TRANSCRIPT_SYNTAX_SET.find_syntax_plain_text());
    let theme = &TRANSCRIPT_THEME_SET.themes[TRANSCRIPT_THEME_NAME];
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut rendered = String::new();

    for (index, line) in LinesWithEndings::from(code).enumerate() {
        if index > 0 {
            rendered.push_str(PROMPT_ANSI);
            rendered.push_str(&continuation);
            rendered.push_str(ANSI_RESET);
        } else {
            rendered.push_str(PROMPT_ANSI);
            rendered.push_str(&prompt);
            rendered.push_str(ANSI_RESET);
        }

        match highlighter.highlight_line(line, &TRANSCRIPT_SYNTAX_SET) {
            Ok(ranges) => rendered.push_str(&as_24_bit_terminal_escaped(&ranges, false)),
            Err(_) => rendered.push_str(line),
        }
    }

    if rendered.is_empty() {
        rendered.push_str(PROMPT_ANSI);
        rendered.push_str(&prompt);
        rendered.push_str(ANSI_RESET);
    }

    rendered
}

fn display_width(text: &str) -> usize {
    strip_ansi(text).chars().count()
}

#[cfg(test)]
fn rendered_line_count(text: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut logical_lines = normalized.split('\n').collect::<Vec<_>>();
    if normalized.ends_with('\n') && logical_lines.len() > 1 {
        logical_lines.pop();
    }
    let mut line_count = 0usize;

    for line in logical_lines {
        let visible_width = strip_ansi(line).chars().count();
        line_count += visible_width.max(1).div_ceil(width);
    }

    line_count.clamp(1, u16::MAX as usize) as u16
}

fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            chars.next();
            while let Some(next) = chars.next() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }

        result.push(ch);
    }

    result
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

fn centered_rect(
    width_percent: u16,
    height_percent: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
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

#[cfg(test)]
mod tests {
    use edtui::{EditorMode, Index2};

    use super::{
        build_editor_state, editor_gutter_lines, editor_mode_badge, editor_syntax_highlighter,
        highlighted_execute_input, line_number_gutter_lines, move_editor_to_row,
        prompt_gutter_lines, prompt_prefixes, rendered_line_count, status_label, status_throbber,
        strip_ansi,
    };
    use crate::insert_history::transcript_lines;
    use crate::kernel::KernelStatus;

    #[test]
    fn editor_state_starts_in_insert_mode_at_end() {
        let editor = build_editor_state("a\nbc");
        assert_eq!(editor.mode, EditorMode::Insert);
        assert_eq!(editor.cursor, Index2::new(1, 2));
    }

    #[test]
    fn splits_transcript_lines_without_extra_trailing_blank_line() {
        assert_eq!(transcript_lines("a\nb"), vec!["a", "b"]);
        assert_eq!(transcript_lines("a\r\nb\r\n"), vec!["a", "b"]);
    }

    #[test]
    fn strips_basic_ansi_sequences() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
    }

    #[test]
    fn counts_wrapped_rendered_lines() {
        assert_eq!(rendered_line_count("abcdef", 3), 2);
        assert_eq!(rendered_line_count("a\nbc", 10), 2);
        assert_eq!(rendered_line_count("a\n", 10), 1);
    }

    #[test]
    fn builds_ipython_prompt_prefixes() {
        let (first, continuation) = prompt_prefixes(&Some(("stdin> ".to_string(), false))).unwrap();
        assert_eq!(first, "stdin> ");
        assert_eq!(continuation, "       ");
    }

    #[test]
    fn builds_line_number_gutter_lines() {
        let lines = line_number_gutter_lines(3, 3);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn uses_line_numbers_for_normal_editor_gutter() {
        let lines = editor_gutter_lines(&None, 2, 2);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn uses_stdin_label_in_status_bar() {
        assert_eq!(
            status_label(&Some(("input".to_string(), false)), "In [3]"),
            Some("stdin")
        );
    }

    #[test]
    fn uses_prompt_label_in_status_bar() {
        assert_eq!(status_label(&None, "In [3]"), Some("In [3]"));
    }

    #[test]
    fn builds_stdin_prompt_prefixes() {
        let (first, continuation) =
            prompt_prefixes(&Some(("In [3]: ".to_string(), false))).unwrap();
        assert_eq!(first, "In [3]: ");
        assert_eq!(continuation, "        ");
    }

    #[test]
    fn renders_busy_spinner_status() {
        assert!(status_throbber(KernelStatus::Busy).is_some());
    }

    #[test]
    fn builds_python_syntax_highlighter() {
        assert!(editor_syntax_highlighter().is_some());
    }

    #[test]
    fn builds_prompt_gutter_lines() {
        let lines = prompt_gutter_lines("In [1]: ", "   ...: ", 2);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn maps_editor_modes_to_badges() {
        assert_eq!(editor_mode_badge(EditorMode::Insert).0, "INS");
        assert_eq!(editor_mode_badge(EditorMode::Normal).0, "NAV");
    }

    #[test]
    fn highlights_execute_input_with_prompt_and_ansi() {
        let rendered = highlighted_execute_input(Some(2), "x = 1");
        assert!(rendered.contains("In [2]: "));
        assert!(rendered.contains("\u{1b}["));
    }

    #[test]
    fn highlights_multiline_execute_input_with_ipython_continuation_prompt() {
        let rendered = strip_ansi(&highlighted_execute_input(Some(2), "x = 1\ny = 2"));
        assert!(rendered.contains("In [2]: x = 1\n   ...: y = 2"));
    }

    #[test]
    fn moves_editor_to_target_row_and_clamps_to_end() {
        let mut editor = build_editor_state("a\nb\nc");
        move_editor_to_row(&mut editor, 1);
        assert_eq!(editor.cursor.row, 1);

        move_editor_to_row(&mut editor, 99);
        assert_eq!(editor.cursor.row, 2);
    }
}
