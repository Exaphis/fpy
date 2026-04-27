mod editor;
mod render;
mod transcript;

use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, MoveToColumn, SetCursorStyle, Show},
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute, queue,
    style::ResetColor,
    terminal::{self, Clear, ClearType, ScrollUp},
    terminal::{EnableLineWrap, disable_raw_mode, enable_raw_mode},
};
use edtui::{
    EditorEventHandler, EditorMode, EditorState, EditorView,
    actions::{Chainable, InsertChar, LineBreak, SwitchMode},
    syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet},
};
use futures::StreamExt;
use nucleo::{
    Config as NucleoConfig, Matcher as NucleoMatcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear as ClearWidget, Paragraph},
};
use std::{
    io::{Write, stdout},
    sync::LazyLock,
    time::Duration,
};
use throbber_widgets_tui::ThrobberState;

use self::{
    editor::{
        PendingStdin, build_editor_state, editor_gutter_lines, editor_gutter_width,
        editor_palette_hint, editor_palette_hint_width, editor_status_prefix,
        editor_status_prefix_width, editor_syntax_highlighter, editor_theme, indent_width,
        status_label,
    },
    render::{
        build_terminal, editor_status_height, editor_visible_line_count, initial_pane_top,
        max_pane_top, pane_rect_at, status_line_for, status_throbber, transient_status_label,
        viewport_height_for_editor,
    },
    transcript::{highlighted_execute_input, runtime_line},
};
use crate::custom_terminal::{CursorStyle, DefaultTerminal};
use crate::history::{HistoryEntry, HistoryOutcome};
use crate::insert_history::insert_history_text;
use crate::kernel::KernelStatus;

const HISTORY_SEARCH_THEME_NAME: &str = "base16-ocean.dark";

static HISTORY_SEARCH_SYNTAX_SET: LazyLock<SyntaxSet> =
    LazyLock::new(SyntaxSet::load_defaults_newlines);
static HISTORY_SEARCH_THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

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
    ReplyInput {
        value: String,
        prompt: Option<String>,
        password: bool,
    },
    Interrupt,
    ClearScreen,
    Exit,
    Restart,
    ShowConnectionInfo,
}

const HISTORY_SEARCH_RESULT_LIMIT: usize = 10;
const HISTORY_SEARCH_MIN_PANE_HEIGHT: u16 = 18;

struct EditorController {
    editor: EditorState,
    editor_events: EditorEventHandler,
    history: Vec<String>,
    history_index: Option<usize>,
    pending_stdin: Option<PendingStdin>,
}

impl EditorController {
    fn new() -> Self {
        Self {
            editor: build_editor_state(""),
            editor_events: EditorEventHandler::default(),
            history: Vec::new(),
            history_index: None,
            pending_stdin: None,
        }
    }

    fn begin_input_request(&mut self, prompt: String, password: bool) {
        self.pending_stdin = Some(PendingStdin::new(prompt, password));
        self.reset();
    }

    fn clear_input_request(&mut self) {
        self.pending_stdin = None;
    }

    fn take_pending_stdin(&mut self) -> Option<PendingStdin> {
        self.pending_stdin.take()
    }

    fn pending_stdin(&self) -> Option<&PendingStdin> {
        self.pending_stdin.as_ref()
    }

    fn awaiting_input(&self) -> bool {
        self.pending_stdin.is_some()
    }

    fn mode(&self) -> EditorMode {
        self.editor.mode
    }

    fn cursor_screen_position(&self) -> Option<ratatui::layout::Position> {
        self.editor.cursor_screen_position()
    }

    fn first_visible_line(&self) -> usize {
        self.editor.viewport_offset().1
    }

    fn visible_line_count(&self) -> usize {
        editor_visible_line_count(self.editor.lines.len())
    }

    fn is_empty(&self) -> bool {
        self.editor.lines.to_string().is_empty()
    }

    fn is_single_line(&self) -> bool {
        self.editor.lines.len() <= 1
    }

    fn editor_mut(&mut self) -> &mut EditorState {
        &mut self.editor
    }

    fn reset(&mut self) {
        self.editor = build_editor_state("");
        self.editor_events = EditorEventHandler::default();
    }

    fn set_text(&mut self, text: &str) {
        self.editor = build_editor_state(text);
        self.editor_events = EditorEventHandler::default();
    }

    fn take_text(&mut self) -> String {
        let text = self.editor.lines.to_string();
        self.reset();
        text
    }

    fn history_up(&mut self) {
        let next = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => self.history.len().saturating_sub(1),
        };
        self.history_index = Some(next);
        let text = self.history[next].clone();
        self.set_text(&text);
    }

    fn history_down(&mut self) {
        match self.history_index {
            Some(index) if index + 1 < self.history.len() => {
                self.history_index = Some(index + 1);
                let text = self.history[index + 1].clone();
                self.set_text(&text);
            }
            Some(_) => {
                self.history_index = None;
                self.reset();
            }
            None => {}
        }
    }

    fn extend_history(&mut self, history: impl IntoIterator<Item = String>) {
        self.history.extend(history);
        self.history_index = None;
    }

    fn push_history(&mut self, text: String) {
        self.history.push(text);
        self.history_index = None;
    }

    fn select_history(&mut self, index: usize) {
        if let Some(text) = self.history.get(index).cloned() {
            self.history_index = Some(index);
            self.set_text(&text);
        }
    }

    fn has_history(&self) -> bool {
        !self.history.is_empty()
    }

    fn history_position(&self) -> Option<(usize, usize)> {
        self.history_index
            .map(|index| (index + 1, self.history.len()))
    }

    fn on_paste(&mut self, text: String) {
        self.editor_events.on_paste_event(text, &mut self.editor);
    }

    fn on_key(&mut self, key: KeyEvent) {
        self.editor_events.on_key_event(key, &mut self.editor);
    }

    fn insert_indent(&mut self) {
        for _ in 0..indent_width() {
            self.editor.execute(InsertChar(' '));
        }
    }

    fn insert_line_break(&mut self) {
        self.editor
            .execute(SwitchMode(EditorMode::Insert).chain(LineBreak(1)));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistorySearchEntry {
    code: String,
    search_text: String,
    first_line: String,
    line_count: usize,
    duration_ns: Option<u64>,
    outcome: Option<HistoryOutcome>,
}

struct HistorySearchState {
    open: bool,
    query: String,
    matcher: NucleoMatcher,
    results: Vec<usize>,
    selected: usize,
    scroll: usize,
}

impl HistorySearchState {
    fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            matcher: NucleoMatcher::new(NucleoConfig::DEFAULT),
            results: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayKind {
    None,
    Palette,
    HistorySearch,
}

pub struct AppUi {
    terminal: Option<DefaultTerminal>,
    events: EventStream,
    current_pane: Rect,
    pane_top: u16,
    editor: EditorController,
    palette_open: bool,
    palette_index: usize,
    history_entries: Vec<HistorySearchEntry>,
    history_search: HistorySearchState,
    last_execution_count: Option<u32>,
    status: KernelStatus,
    connection_summary: String,
    throbber_state: ThrobberState,
    session_ready: bool,
    last_overlay_kind: OverlayKind,
    status_row_dirty: bool,
    dirty: bool,
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
            editor: EditorController::new(),
            palette_open: false,
            palette_index: 0,
            history_entries: Vec::new(),
            history_search: HistorySearchState::new(),
            last_execution_count: None,
            status: KernelStatus::Connecting,
            connection_summary,
            throbber_state: ThrobberState::default(),
            session_ready: false,
            last_overlay_kind: OverlayKind::None,
            status_row_dirty: false,
            dirty: true,
            restored: false,
        })
    }

    pub fn load_history(&mut self, history: Vec<HistoryEntry>) {
        self.editor
            .extend_history(history.iter().map(|entry| entry.code.clone()));
        self.history_entries = history
            .into_iter()
            .map(HistorySearchEntry::from_history_entry)
            .collect();
    }

    pub fn record_history_submission(&mut self, code: &str) -> usize {
        self.history_entries
            .push(HistorySearchEntry::new(code.to_string()));
        self.history_entries.len().saturating_sub(1)
    }

    pub fn record_history_completion(
        &mut self,
        history_index: usize,
        duration: Duration,
        outcome: HistoryOutcome,
    ) {
        if let Some(entry) = self.history_entries.get_mut(history_index) {
            entry.duration_ns = Some(duration.as_nanos().min(u128::from(u64::MAX)) as u64);
            entry.outcome = Some(outcome);
        }
    }

    pub fn connection_summary(&self) -> &str {
        &self.connection_summary
    }

    pub fn set_connection_summary(&mut self, summary: String) {
        self.connection_summary = summary;
        self.dirty = true;
    }

    pub fn set_status(&mut self, status: KernelStatus) {
        if self.session_ready && status == KernelStatus::Connecting {
            return;
        }
        self.status = status;
        if status == KernelStatus::Disconnected {
            self.session_ready = false;
        }
        self.dirty = true;
    }

    pub fn set_last_execution_count(&mut self, count: Option<u32>) {
        if let Some(count) = count {
            self.last_execution_count = Some(count);
            self.dirty = true;
        }
    }

    pub fn needs_animation(&self) -> bool {
        self.status_spins()
    }

    pub fn needs_redraw(&self) -> bool {
        self.dirty
    }

    pub fn request_redraw(&mut self) {
        self.dirty = true;
    }

    pub fn begin_input_request(&mut self, prompt: String, password: bool) {
        self.editor.begin_input_request(prompt, password);
        self.status = KernelStatus::AwaitingInput;
        self.dirty = true;
    }

    pub fn clear_input_request(&mut self) {
        self.editor.clear_input_request();
        self.dirty = true;
    }

    pub fn mark_session_ready(&mut self) {
        self.session_ready = true;
        self.status = KernelStatus::Idle;
        self.dirty = true;
    }

    pub fn insert_transcript(&mut self, text: impl Into<String>) -> Result<()> {
        self.sync_viewport()?;
        let new_pane = {
            let terminal = self.terminal_mut()?;
            insert_history_text(terminal, &text.into())?;
            terminal.viewport_area()
        };
        self.current_pane = new_pane;
        self.pane_top = new_pane.y;
        self.status_row_dirty = true;
        self.dirty = true;
        Ok(())
    }

    pub fn insert_execute_input(&mut self, execution_count: Option<u32>, code: &str) -> Result<()> {
        self.insert_transcript(highlighted_execute_input(execution_count, code))
    }

    pub fn insert_runtime(&mut self, duration: Duration) -> Result<()> {
        self.insert_transcript(runtime_line(duration))
    }

    pub fn shutdown(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        let pane = self.pane_rect()?;
        if let Some(mut terminal) = self.terminal.take() {
            let _ = terminal.set_cursor_style(CursorStyle::Default);
            let _ = terminal.show_cursor();
        }

        let mut handle = stdout();
        let _ = execute!(handle, DisableBracketedPaste, PopKeyboardEnhancementFlags);
        write!(handle, "\x1b[r\x1b[0m")?;
        execute!(
            handle,
            Show,
            SetCursorStyle::DefaultUserShape,
            ResetColor,
            EnableLineWrap
        )?;
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
        execute!(
            handle,
            ResetColor,
            EnableLineWrap,
            Show,
            SetCursorStyle::DefaultUserShape,
            MoveToColumn(0)
        )?;
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
        self.dirty = true;
        Ok(())
    }

    pub fn redraw(&mut self) -> Result<()> {
        self.sync_viewport()?;
        if self.status_row_dirty && self.input_active() {
            self.clear_status_row()?;
            self.terminal_mut()?.invalidate_viewport();
            self.status_row_dirty = false;
        }
        let awaiting_input = self.editor.pending_stdin().cloned();
        let palette_index = self.palette_index;
        let input_active = self.input_active();
        let prompt_label = self.prompt_label();
        let visible_lines = self.editor.visible_line_count();
        let status = self.status;
        let overlay_kind = self.overlay_kind();
        let transient_status = if self.session_ready && status == KernelStatus::Connecting {
            None
        } else {
            transient_status_label(status)
        };

        if overlay_kind != self.last_overlay_kind {
            self.clear_current_pane_rows()?;
            self.terminal_mut()?.invalidate_viewport();
        }

        let AppUi {
            terminal,
            throbber_state,
            editor,
            history_entries,
            history_search,
            ..
        } = self;
        let terminal = terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("terminal is not active"))?;
        let cursor_style = if input_active && overlay_kind == OverlayKind::None {
            if editor.mode() == EditorMode::Insert {
                CursorStyle::Bar
            } else {
                CursorStyle::Block
            }
        } else {
            CursorStyle::Block
        };
        terminal.set_cursor_style(cursor_style)?;

        terminal.draw(|frame| {
            let area = frame.area();
            if input_active {
                let [content_area, status_area] = Layout::vertical([
                    Constraint::Min(1),
                    Constraint::Length(editor_status_height()),
                ])
                .areas(area);

                match overlay_kind {
                    OverlayKind::Palette => {
                        frame.render_widget(ClearWidget, area);
                        render_palette_popup(frame, content_area, palette_index);
                    }
                    OverlayKind::HistorySearch => {
                        frame.render_widget(ClearWidget, area);
                        render_history_search_popup(
                            frame,
                            content_area,
                            history_search,
                            history_entries,
                        );
                    }
                    OverlayKind::None => {
                        frame.render_widget(ClearWidget, content_area);
                        frame.render_widget(ClearWidget, status_area);
                        let gutter_width =
                            editor_gutter_width(awaiting_input.as_ref(), visible_lines)
                                .min(content_area.width);
                        let [gutter_area, content_area] = Layout::horizontal([
                            Constraint::Length(gutter_width),
                            Constraint::Min(1),
                        ])
                        .areas(content_area);
                        let editor_view = EditorView::new(editor.editor_mut())
                            .theme(editor_theme())
                            .tab_width(indent_width())
                            .wrap(false)
                            .syntax_highlighter(editor_syntax_highlighter())
                            .single_line(false);
                        frame.render_widget(editor_view, content_area);

                        let gutter_lines = editor_gutter_lines(
                            awaiting_input.as_ref(),
                            gutter_area.height as usize,
                            visible_lines,
                            editor.first_visible_line(),
                        );
                        frame.render_widget(Paragraph::new(gutter_lines), gutter_area);
                    }
                }

                if overlay_kind == OverlayKind::HistorySearch {
                    render_history_search_status(frame, status_area);
                } else if let Some(throbber) = status_throbber(status) {
                    let status_detail = status_label(
                        awaiting_input.as_ref(),
                        &prompt_label,
                        editor.history_position(),
                    );
                    let prefix_width =
                        editor_status_prefix_width(editor.mode(), status_detail.as_deref());
                    let transient_width = transient_status
                        .map(|label| u16::try_from(label.chars().count()).unwrap_or(u16::MAX))
                        .unwrap_or(0);
                    let palette_hint_width = editor_palette_hint_width();
                    let [
                        status_text_area,
                        spinner_gap_area,
                        spinner_area,
                        transient_area,
                        filler_area,
                        palette_hint_area,
                    ] = Layout::horizontal([
                        Constraint::Length(prefix_width),
                        Constraint::Length(1),
                        Constraint::Length(2),
                        Constraint::Length(transient_width),
                        Constraint::Min(1),
                        Constraint::Length(palette_hint_width),
                    ])
                    .areas(status_area);
                    frame.render_widget(
                        editor_status_prefix(editor.mode(), status_detail.as_deref()),
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
                    let status_detail = status_label(
                        awaiting_input.as_ref(),
                        &prompt_label,
                        editor.history_position(),
                    );
                    let prefix_width =
                        editor_status_prefix_width(editor.mode(), status_detail.as_deref());
                    let palette_hint_width = editor_palette_hint_width();
                    let [status_text_area, filler_area, palette_hint_area] = Layout::horizontal([
                        Constraint::Length(prefix_width),
                        Constraint::Min(1),
                        Constraint::Length(palette_hint_width),
                    ])
                    .areas(status_area);
                    frame.render_widget(
                        editor_status_prefix(editor.mode(), status_detail.as_deref()),
                        status_text_area,
                    );
                    frame.render_widget(Paragraph::new(""), filler_area);
                    frame.render_widget(editor_palette_hint(), palette_hint_area);
                }

                if overlay_kind == OverlayKind::None
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
        self.last_overlay_kind = overlay_kind;
        self.dirty = false;

        Ok(())
    }

    pub async fn next_action(&mut self) -> Result<Option<UiAction>> {
        while let Some(event) = self.events.next().await {
            match event? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    let action = self.handle_key(key);
                    self.request_redraw();
                    if let Some(action) = action {
                        return Ok(Some(action));
                    }
                    return Ok(None);
                }
                Event::Paste(text) => {
                    let text = text.replace("\r\n", "\n").replace('\r', "\n");
                    if self.history_search.open {
                        self.history_search.query.push_str(&text.replace('\n', " "));
                        self.refresh_history_search_results();
                    } else if self.editor_enabled() {
                        self.editor.on_paste(text);
                    }
                    self.request_redraw();
                    return Ok(None);
                }
                Event::Resize(_, _) => {
                    self.sync_viewport()?;
                    self.request_redraw();
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        if self.history_search.open {
            return self.handle_history_search_key(key);
        }
        if self.palette_open {
            return self.handle_palette_key(key);
        }

        match key {
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_history_search();
                None
            }
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
                if self.editor.awaiting_input() {
                    Some(UiAction::Interrupt)
                } else if self.submit_ready() {
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
                && !self.editor.awaiting_input()
                && self.editor.is_empty() =>
            {
                Some(UiAction::Exit)
            }
            KeyEvent {
                code: KeyCode::Char('l'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => Some(UiAction::ClearScreen),
            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                && self.editor_enabled()
                && self.editor.has_history() =>
            {
                self.editor.history_up();
                None
            }
            KeyEvent {
                code: KeyCode::Char('j'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                && self.editor_enabled()
                && self.editor.has_history() =>
            {
                self.editor.history_down();
                None
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } if self.editor_enabled() && self.editor.mode() == EditorMode::Insert => {
                self.editor.insert_indent();
                None
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers,
                ..
            } if self.editor_enabled() && modifiers.contains(KeyModifiers::SHIFT) => {
                self.editor.insert_line_break();
                None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } if self.submit_ready() => {
                let text = self.editor.take_text();
                if let Some(stdin) = self.editor.take_pending_stdin() {
                    Some(UiAction::ReplyInput {
                        value: text,
                        prompt: stdin.visible_prompt().map(str::to_owned),
                        password: stdin.password(),
                    })
                } else {
                    if text.trim().is_empty() {
                        return None;
                    }
                    self.editor.push_history(text.clone());
                    Some(UiAction::Submit(text))
                }
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } if self.editor_enabled()
                && self.editor.is_single_line()
                && self.editor.has_history() =>
            {
                self.editor.history_up();
                None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } if self.editor_enabled()
                && self.editor.is_single_line()
                && self.editor.has_history() =>
            {
                self.editor.history_down();
                None
            }
            key if self.editor_enabled() => {
                self.editor.on_key(key);
                None
            }
            _ => None,
        }
    }

    fn handle_history_search_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.history_search.open = false;
                None
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.history_search.selected = self.history_search.selected.saturating_sub(1);
                self.adjust_history_search_scroll();
                None
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                let max = self.history_search.results.len().saturating_sub(1);
                self.history_search.selected = (self.history_search.selected + 1).min(max);
                self.adjust_history_search_scroll();
                None
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                if let Some(&entry_index) = self
                    .history_search
                    .results
                    .get(self.history_search.selected)
                {
                    self.editor.select_history(entry_index);
                }
                self.history_search.open = false;
                None
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.history_search.query.pop();
                self.refresh_history_search_results();
                None
            }
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                let max = self.history_search.results.len().saturating_sub(1);
                self.history_search.selected = (self.history_search.selected + 1).min(max);
                self.adjust_history_search_scroll();
                None
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.history_search.query.push(ch);
                self.refresh_history_search_results();
                None
            }
            _ => None,
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) -> Option<UiAction> {
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

    fn open_history_search(&mut self) {
        self.palette_open = false;
        self.history_search.open = true;
        self.history_search.query.clear();
        self.refresh_history_search_results();
    }

    fn refresh_history_search_results(&mut self) {
        let query = self.history_search.query.trim();
        if query.is_empty() {
            self.history_search.results = (0..self.history_entries.len()).rev().collect();
            self.history_search.selected = 0;
            self.history_search.scroll = 0;
            return;
        }

        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut matches = self
            .history_entries
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, entry)| {
                pattern
                    .score(
                        Utf32Str::new(&entry.search_text, &mut buf),
                        &mut self.history_search.matcher,
                    )
                    .map(|score| (index, score))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
        self.history_search.results = matches.into_iter().map(|(index, _)| index).collect();
        self.history_search.selected = 0;
        self.history_search.scroll = 0;
    }

    fn adjust_history_search_scroll(&mut self) {
        let visible_rows = self.history_search_visible_rows();
        if visible_rows == 0 {
            self.history_search.scroll = 0;
            return;
        }

        let max_scroll = self
            .history_search
            .results
            .len()
            .saturating_sub(visible_rows);
        let preferred_row = visible_rows.saturating_sub(1).min(4);
        let target_scroll = self
            .history_search
            .selected
            .saturating_sub(preferred_row)
            .min(max_scroll);
        self.history_search.scroll = target_scroll;
    }

    fn history_search_visible_rows(&self) -> usize {
        history_search_layout_for_popup(
            self.current_pane.height.saturating_sub(1),
            self.history_search.results.len(),
            self.history_search_selected_preview_rows(),
        )
        .0 as usize
    }

    fn history_search_selected_preview_rows(&self) -> usize {
        self.history_search
            .results
            .get(self.history_search.selected)
            .and_then(|&entry_index| self.history_entries.get(entry_index))
            .map(|entry| entry.line_count)
            .unwrap_or(1)
    }

    fn overlay_kind(&self) -> OverlayKind {
        if self.history_search.open {
            OverlayKind::HistorySearch
        } else if self.palette_open {
            OverlayKind::Palette
        } else {
            OverlayKind::None
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
            self.dirty = true;
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
            self.editor.visible_line_count() as u16 + editor_status_height()
        } else {
            1 + editor_status_height()
        };
        match self.overlay_kind() {
            OverlayKind::HistorySearch => input_rows.max(HISTORY_SEARCH_MIN_PANE_HEIGHT.max(
                history_search_desired_pane_height(
                    self.history_search.results.len(),
                    self.history_search_selected_preview_rows(),
                ),
            )),
            OverlayKind::Palette => viewport_height_for_editor(input_rows, true),
            OverlayKind::None => viewport_height_for_editor(input_rows, false),
        }
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

    fn clear_editor_view(&mut self) -> Result<()> {
        self.clear_current_pane_rows()?;
        self.editor.reset();
        self.terminal_mut()?.invalidate_viewport();
        self.dirty = true;
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

    fn clear_status_row(&mut self) -> Result<()> {
        let pane = self.current_pane;
        if pane.is_empty() {
            return Ok(());
        }

        let status_row = pane.bottom().saturating_sub(1);
        let terminal = self.terminal_mut()?;
        let handle = terminal.backend_mut();
        execute!(
            handle,
            MoveTo(0, status_row),
            Clear(ClearType::UntilNewLine)
        )?;
        handle.flush()?;
        Ok(())
    }

    fn scroll_history_for_pane_growth(
        &mut self,
        old: Rect,
        new: Rect,
        screen_height: u16,
    ) -> Result<()> {
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
}

impl HistorySearchEntry {
    fn new(code: String) -> Self {
        let first_line = code.lines().next().unwrap_or_default().to_string();
        let line_count = code.lines().count().max(1);
        Self {
            search_text: normalize_history_search_text(&code),
            code,
            first_line,
            line_count,
            duration_ns: None,
            outcome: None,
        }
    }

    fn from_history_entry(entry: HistoryEntry) -> Self {
        let mut search = Self::new(entry.code);
        search.duration_ns = entry.duration_ns;
        search.outcome = entry.outcome;
        search
    }

    fn summary(&self, width: usize) -> String {
        let left = if self.line_count > 1 {
            format!("{} …", self.first_line)
        } else {
            self.first_line.clone()
        };
        let right = self.metadata();
        if width == 0 {
            return String::new();
        }
        if right.is_empty() {
            return truncate_chars(&left, width);
        }
        let right_width = right.chars().count();
        if right_width >= width {
            return truncate_chars(&right, width);
        }
        let left_width = width.saturating_sub(right_width + 1);
        format!(
            "{:<left_width$} {}",
            truncate_chars(&left, left_width),
            right
        )
    }

    fn metadata(&self) -> String {
        let mut parts = Vec::new();
        if let Some(duration_ns) = self.duration_ns {
            parts.push(format_duration_ns(duration_ns));
        }
        if let Some(outcome) = self.outcome {
            match outcome {
                HistoryOutcome::Ok => {}
                HistoryOutcome::Error => parts.push("error".to_string()),
                HistoryOutcome::Interrupted => parts.push("interrupted".to_string()),
            }
        }
        parts.join(" ")
    }
}

fn render_palette_popup(
    frame: &mut crate::custom_terminal::Frame<'_>,
    content_area: Rect,
    palette_index: usize,
) {
    let palette_width = content_area.width.clamp(24, 56);
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
}

fn render_history_search_popup(
    frame: &mut crate::custom_terminal::Frame<'_>,
    content_area: Rect,
    history_search: &HistorySearchState,
    history_entries: &[HistorySearchEntry],
) {
    let popup_width = content_area.width.clamp(32, 96);
    let selected_preview_rows = history_search
        .results
        .get(history_search.selected)
        .and_then(|&entry_index| history_entries.get(entry_index))
        .map(|entry| entry.line_count)
        .unwrap_or(1);
    let popup_height = content_area.height.max(6);
    let popup_area = Rect::new(content_area.x, content_area.y, popup_width, popup_height);
    frame.render_widget(
        Block::default()
            .title("History Search")
            .borders(Borders::ALL),
        popup_area,
    );

    let inner = popup_area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    if inner.is_empty() {
        return;
    }

    let (results_height, preview_height) = history_search_layout_for_popup(
        popup_area.height,
        history_search.results.len(),
        selected_preview_rows,
    );
    let query_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let results_area = Rect::new(inner.x, query_area.bottom(), inner.width, results_height);
    let preview_label_area = Rect::new(inner.x, results_area.bottom(), inner.width, 1);
    let preview_area = Rect::new(
        inner.x,
        preview_label_area.bottom(),
        inner.width,
        preview_height,
    );

    frame.render_widget(ClearWidget, inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("query: ", Style::default().fg(Color::Cyan)),
            Span::raw(history_search.query.clone()),
        ])),
        query_area,
    );

    let results_width = usize::from(results_area.width);
    let buffer = frame.buffer_mut();
    for row in 0..results_area.height as usize {
        let result_index = history_search.scroll.saturating_add(row);
        let (text, style) = if let Some(&entry_index) = history_search.results.get(result_index) {
            if let Some(entry) = history_entries.get(entry_index) {
                let is_selected = result_index == history_search.selected;
                let style = if is_selected {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default()
                };
                let marker = if is_selected { "> " } else { "  " };
                let summary_width = results_width.saturating_sub(marker.chars().count());
                (format!("{marker}{}", entry.summary(summary_width)), style)
            } else {
                (" ".repeat(results_width), Style::default())
            }
        } else if history_search.results.is_empty() && row == 0 {
            (
                format!("{:<width$}", "no history matches", width = results_width),
                Style::default().fg(Color::DarkGray),
            )
        } else {
            (" ".repeat(results_width), Style::default())
        };
        buffer.set_stringn(
            results_area.x,
            results_area.y + row as u16,
            text,
            results_width,
            style,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "preview",
            Style::default().fg(Color::DarkGray),
        ))),
        preview_label_area,
    );

    let preview_lines = history_search
        .results
        .get(history_search.selected)
        .and_then(|&entry_index| history_entries.get(entry_index))
        .map(|entry| syntax_highlighted_history_preview(&entry.code))
        .unwrap_or_else(|| vec![Line::default()]);
    frame.render_widget(Paragraph::new(preview_lines), preview_area);
}

fn syntax_highlighted_history_preview(code: &str) -> Vec<Line<'static>> {
    let syntax = HISTORY_SEARCH_SYNTAX_SET
        .find_syntax_by_extension("py")
        .unwrap_or_else(|| HISTORY_SEARCH_SYNTAX_SET.find_syntax_plain_text());
    let Some(theme) = HISTORY_SEARCH_THEME_SET
        .themes
        .get(HISTORY_SEARCH_THEME_NAME)
    else {
        return plain_history_preview(code);
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    let lines = code.split('\n');
    let mut highlighted = Vec::new();

    for line in lines {
        match highlighter.highlight_line(line, &HISTORY_SEARCH_SYNTAX_SET) {
            Ok(ranges) => {
                let spans = ranges
                    .into_iter()
                    .map(|(style, text)| {
                        Span::styled(
                            text.to_string(),
                            Style::default().fg(Color::Rgb(
                                style.foreground.r,
                                style.foreground.g,
                                style.foreground.b,
                            )),
                        )
                    })
                    .collect::<Vec<_>>();
                highlighted.push(Line::from(spans));
            }
            Err(_) => highlighted.push(Line::raw(line.to_string())),
        }
    }

    if highlighted.is_empty() {
        highlighted.push(Line::default());
    }
    highlighted
}

fn plain_history_preview(code: &str) -> Vec<Line<'static>> {
    let mut lines = code
        .split('\n')
        .map(|line| Line::raw(line.to_string()))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

fn render_history_search_status(frame: &mut crate::custom_terminal::Frame<'_>, status_area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " search ",
                Style::default().fg(Color::Black).bg(Color::Green),
            ),
            Span::raw("  Ctrl-R next  Enter load  Esc cancel"),
        ])),
        status_area,
    );
}

fn history_search_desired_popup_height(result_count: usize, preview_rows: usize) -> u16 {
    let desired_results = result_count.clamp(1, HISTORY_SEARCH_RESULT_LIMIT) as u16;
    let desired_preview = preview_rows.max(1) as u16;
    4u16.saturating_add(desired_results)
        .saturating_add(desired_preview)
}

fn history_search_desired_pane_height(result_count: usize, preview_rows: usize) -> u16 {
    1u16.saturating_add(history_search_desired_popup_height(
        result_count,
        preview_rows,
    ))
}

fn history_search_layout_for_popup(
    popup_height: u16,
    result_count: usize,
    preview_rows: usize,
) -> (u16, u16) {
    let available = popup_height.saturating_sub(4).max(2);
    let desired_results = result_count.clamp(1, HISTORY_SEARCH_RESULT_LIMIT) as u16;
    let desired_preview = preview_rows.max(1) as u16;

    if desired_results.saturating_add(desired_preview) <= available {
        return (desired_results, desired_preview);
    }

    let results_height = desired_results.min(available.saturating_sub(desired_preview).max(1));
    let preview_height = available.saturating_sub(results_height).max(1);
    (results_height, preview_height)
}

fn normalize_history_search_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

fn format_duration_ns(duration_ns: u64) -> String {
    let duration = Duration::from_nanos(duration_ns);
    let elapsed = duration.as_secs_f64();
    if elapsed < 0.001 {
        let micros = elapsed * 1e6;
        let decimals = if micros >= 100.0 {
            0
        } else if micros >= 10.0 {
            1
        } else {
            2
        };
        format!("{micros:.decimals$}µs")
    } else if elapsed < 1.0 {
        let millis = elapsed * 1e3;
        let decimals = if millis >= 100.0 {
            0
        } else if millis >= 10.0 {
            1
        } else {
            2
        };
        format!("{millis:.decimals$}ms")
    } else if elapsed < 60.0 {
        let decimals = if elapsed >= 10.0 { 1 } else { 2 };
        format!("{elapsed:.decimals$}s")
    } else {
        let total_seconds = duration.as_secs();
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{minutes}m{seconds:02}s")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_search_preview_uses_python_syntax_highlighting() {
        let lines = syntax_highlighted_history_preview("x = 1\nprint(x)");

        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "x = 1print(x)"
        );
        assert!(
            lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .any(|span| span.style.fg.is_some())
        );
    }
}
