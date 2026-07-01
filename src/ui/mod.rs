mod backend;
mod components;
pub(crate) mod display;
mod duration_format;
mod editor;
mod editor_controller;
mod history_search;
mod input;
mod palette;
mod prompt;
mod render;
mod session;
mod style;
mod syntax;
mod transcript;

use anyhow::Result;
use crossterm::{
    event::{Event, EventStream, KeyEvent, KeyEventKind},
    terminal,
};
use edtui::EditorMode;
use futures::StreamExt;
use ratatui::layout::{Rect, Size};
use std::{
    io::{Stdout, stdout},
    time::{Duration, Instant},
};

use self::{
    backend::{PiStyleMainScreenBackend, TerminalBackend},
    display::{
        DisplayKernelStatus, DisplayModel, DisplayRenderer, HistorySearchOverlayModel,
        HistorySearchResultModel, MimeBundle, OverlayModel, PaletteOverlayModel,
        StreamName as DisplayStreamName,
    },
    duration_format::format_duration_ns,
    editor::{PendingStdin, status_label},
    editor_controller::EditorController,
    history_search::{
        HistorySearchEntry, HistorySearchState, history_search_layout_for_popup,
        history_search_scroll_for_selection, syntax_highlighted_history_preview,
    },
    input::{
        GlobalKey, GlobalKeyContext, HistorySearchKey, PaletteKey, classify_global_key,
        classify_history_search_key, classify_palette_key, history_search_paste_text,
        is_exit_confirmation_key, normalize_paste_text,
    },
    palette::{PaletteAction, palette_items},
    prompt::input_prompt_label,
    render::transient_status_label,
    session::TerminalSession,
    transcript::runtime_line,
};
use crate::history::{HistoryEntry, HistoryOutcome};
use crate::kernel::{KernelStatus, StreamName as KernelStreamName};

const HISTORY_SEARCH_MAX_PREVIEW_ROWS: usize = 8;

#[derive(Debug)]
pub enum UiAction {
    Submit(String),
    ReplyInput { value: String },
    Interrupt,
    ClearScreen,
    Exit,
    Restart,
    ShowConnectionInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayKind {
    None,
    Palette,
    HistorySearch,
}

pub struct AppUi {
    frame_backend: PiStyleMainScreenBackend<Stdout>,
    display_renderer: DisplayRenderer,
    terminal_session: TerminalSession,
    events: EventStream,
    current_pane: Rect,
    display_model: DisplayModel,
    pending_stdin: Option<PendingStdin>,
    editor: EditorController,
    palette_open: bool,
    palette_index: usize,
    history_entries: Vec<HistorySearchEntry>,
    history_search: HistorySearchState,
    last_execution_count: Option<u32>,
    busy_started_at: Option<Instant>,
    busy_code: Option<String>,
    optimistic_submit_code: Option<String>,
    status: KernelStatus,
    connection_summary: String,
    session_ready: bool,
    exit_confirmation_pending: bool,
    dirty: bool,
}

impl AppUi {
    pub fn new(connection_summary: String) -> Result<Self> {
        let terminal_session = TerminalSession::start()?;
        let (width, height) = terminal::size()?;
        let stdout = stdout();
        let pane = Rect::new(0, 0, width, height);
        let frame_backend = PiStyleMainScreenBackend::new(stdout, Size::new(width, height));

        Ok(Self {
            frame_backend,
            display_renderer: DisplayRenderer,
            terminal_session,
            events: EventStream::new(),
            current_pane: pane,
            display_model: DisplayModel::new(),
            pending_stdin: None,
            editor: EditorController::new(),
            palette_open: false,
            palette_index: 0,
            history_entries: Vec::new(),
            history_search: HistorySearchState::new(),
            last_execution_count: None,
            busy_started_at: None,
            busy_code: None,
            optimistic_submit_code: None,
            status: KernelStatus::Connecting,
            connection_summary,
            session_ready: false,
            exit_confirmation_pending: false,
            dirty: true,
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

    pub fn discard_editor_history_submission(&mut self, code: &str) {
        self.editor.pop_history_if_last(code);
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
        self.display_model.kernel_status = DisplayKernelStatus::from(status);
        if status == KernelStatus::Disconnected {
            self.session_ready = false;
            self.clear_busy_runtime();
        } else if status == KernelStatus::Idle {
            self.clear_busy_runtime();
        }
        self.dirty = true;
    }

    pub fn set_last_execution_count(&mut self, count: Option<u32>) {
        if let Some(count) = count {
            self.last_execution_count = Some(count);
            self.dirty = true;
        }
    }

    pub fn reset_last_execution_count(&mut self) {
        self.last_execution_count = None;
        self.dirty = true;
    }

    fn begin_busy_runtime(&mut self, code: String) {
        self.busy_started_at = Some(Instant::now());
        self.busy_code = Some(code);
        self.dirty = true;
    }

    fn clear_busy_runtime(&mut self) {
        self.busy_started_at = None;
        self.busy_code = None;
        self.optimistic_submit_code = None;
    }

    fn last_runtime_for_code(&self, code: &str) -> Option<u64> {
        self.history_entries
            .iter()
            .rev()
            .find(|entry| entry.code == code && entry.duration_ns.is_some())
            .and_then(|entry| entry.duration_ns)
    }

    fn busy_runtime_label(&self) -> Option<String> {
        let elapsed = self.busy_started_at?.elapsed();
        let elapsed_ns = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        let current = format_duration_ns(elapsed_ns);
        let last = self
            .busy_code
            .as_deref()
            .and_then(|code| self.last_runtime_for_code(code));
        Some(match last {
            Some(last_ns) => format!("{current} ({} last)", format_duration_ns(last_ns)),
            None => current,
        })
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

    fn sync_display_model_live_state(&mut self) {
        self.display_model.editor.text = self.editor.text();
        self.display_model.editor.render_state = Some(self.editor.render_state());
        self.display_model.editor.prompt = if let Some(stdin) = self.editor.pending_stdin() {
            stdin
                .visible_prompt()
                .map_or_else(String::new, ToString::to_string)
        } else {
            format!("{}: ", self.prompt_label())
        };
        self.display_model.kernel_status = DisplayKernelStatus::from(self.status);
        let status_detail = if self.exit_confirmation_pending {
            Some("press Ctrl-D again to exit".to_string())
        } else {
            status_label(
                self.editor.pending_stdin(),
                &self.prompt_label(),
                self.editor.history_position(),
            )
        };
        let transient = if self.session_ready && self.status == KernelStatus::Connecting {
            None
        } else {
            transient_status_label(self.status).map(ToString::to_string)
        };
        let activity = if self.status == KernelStatus::Busy {
            self.busy_runtime_label()
                .map(|label| format!("{label} Kernel busy. Ctrl-C to interrupt"))
                .or_else(|| Some("Kernel busy. Ctrl-C to interrupt".to_string()))
        } else if self.status == KernelStatus::Disconnected {
            Some("Kernel disconnected".to_string())
        } else {
            transient
        };
        self.display_model.footer.text = Some(
            [
                Some(match self.editor.mode() {
                    EditorMode::Insert => "INS".to_string(),
                    EditorMode::Normal => "NAV".to_string(),
                    EditorMode::Visual => "VIS".to_string(),
                    EditorMode::Search => "SRCH".to_string(),
                }),
                status_detail,
                activity,
                Some("Ctrl-P palette".to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" "),
        );
        self.display_model.overlay = match self.overlay_kind() {
            OverlayKind::None => OverlayModel::None,
            OverlayKind::Palette => OverlayModel::Palette(PaletteOverlayModel {
                items: palette_items()
                    .iter()
                    .map(|action| action.label().to_string())
                    .collect(),
                selected: self.palette_index,
            }),
            OverlayKind::HistorySearch => {
                let visible_result_rows = self.history_search_visible_rows();
                let visible_preview_rows = self.history_search_preview_rows();
                let result_summary_width =
                    (self.current_pane.width as usize).saturating_sub(2).max(1);
                let result_start = self.history_search.scroll;
                let result_end = result_start.saturating_add(visible_result_rows);
                let selected_preview = self
                    .history_search
                    .results
                    .get(self.history_search.selected)
                    .and_then(|&entry_index| self.history_entries.get(entry_index))
                    .map(|entry| {
                        syntax_highlighted_history_preview(&entry.code)
                            .into_iter()
                            .flat_map(|line| {
                                crate::ui::transcript::wrap_ansi_to_width(
                                    &line,
                                    self.current_pane.width,
                                )
                            })
                            .take(visible_preview_rows)
                            .collect()
                    })
                    .unwrap_or_default();
                OverlayModel::HistorySearch(HistorySearchOverlayModel {
                    query: self.history_search.query.clone(),
                    results: self
                        .history_search
                        .results
                        .iter()
                        .enumerate()
                        .skip(result_start)
                        .take(result_end.saturating_sub(result_start))
                        .filter_map(|(result_index, &entry_index)| {
                            self.history_entries.get(entry_index).map(|entry| {
                                HistorySearchResultModel {
                                    summary: entry.highlighted_summary(result_summary_width),
                                    selected: result_index == self.history_search.selected,
                                }
                            })
                        })
                        .collect(),
                    selected: self.history_search.selected,
                    preview_lines: selected_preview,
                })
            }
        };
    }

    pub fn begin_input_request(&mut self, prompt: String, password: bool) {
        self.pending_stdin = Some(PendingStdin::new(prompt.clone(), password));
        self.editor.begin_input_request(prompt, password);
        self.status = KernelStatus::AwaitingInput;
        self.display_model.kernel_status = DisplayKernelStatus::AwaitingInput;
        self.dirty = true;
    }

    pub fn clear_input_request(&mut self) {
        self.editor.clear_input_request();
        self.pending_stdin = None;
        self.dirty = true;
    }

    pub fn record_input_reply(&mut self, value: &str) {
        if let Some(stdin) = self.pending_stdin.take()
            && !stdin.password()
        {
            let index = self
                .display_model
                .transcript
                .push_stdin(stdin.prompt().to_string(), false);
            self.display_model
                .transcript
                .fill_stdin_value(index, value.to_string());
        }
        self.dirty = true;
    }

    pub fn mark_session_ready(&mut self) {
        self.session_ready = true;
        self.status = KernelStatus::Idle;
        self.display_model.kernel_status = DisplayKernelStatus::Idle;
        self.dirty = true;
    }

    pub fn insert_transcript(&mut self, text: impl Into<String>) -> Result<()> {
        let text = text.into();
        self.display_model.transcript.push_system(text);
        self.dirty = true;
        Ok(())
    }

    pub fn show_submitted_code(&mut self, code: &str) -> Result<()> {
        self.begin_busy_runtime(code.to_string());
        self.optimistic_submit_code = Some(code.to_string());
        self.set_status(KernelStatus::Busy);
        let optimistic_count = self
            .last_execution_count
            .map(|count| count.saturating_add(1));
        self.display_model
            .transcript
            .push_input(optimistic_count, code);
        self.dirty = true;
        Ok(())
    }

    pub fn insert_execute_input(&mut self, execution_count: Option<u32>, code: &str) -> Result<()> {
        let reconciled_optimistic_input = self.optimistic_submit_code.as_deref() == Some(code)
            && self
                .display_model
                .transcript
                .update_most_recent_input_if_code_matches(execution_count, code);
        if reconciled_optimistic_input {
            self.optimistic_submit_code = None;
        } else {
            self.begin_busy_runtime(code.to_string());
            self.display_model
                .transcript
                .push_input(execution_count, code);
        }
        self.dirty = true;
        Ok(())
    }

    pub fn insert_execute_result(
        &mut self,
        execution_count: Option<u32>,
        text: &str,
    ) -> Result<()> {
        self.display_model
            .transcript
            .push_execute_result(execution_count, MimeBundle::plain(text));
        self.dirty = true;
        Ok(())
    }

    pub fn insert_stream(&mut self, name: KernelStreamName, text: &str) -> Result<()> {
        let name = match name {
            KernelStreamName::Stdout => DisplayStreamName::Stdout,
            KernelStreamName::Stderr => DisplayStreamName::Stderr,
        };
        self.display_model.transcript.push_stream(name, text);
        self.dirty = true;
        Ok(())
    }

    pub fn insert_error(&mut self, traceback: &[String]) -> Result<()> {
        self.display_model.transcript.push_error(traceback.to_vec());
        self.dirty = true;
        Ok(())
    }

    pub fn insert_runtime(&mut self, duration: Duration) -> Result<()> {
        self.insert_transcript(runtime_line(duration))
    }

    pub fn shutdown(&mut self) -> Result<()> {
        if self.terminal_session.is_restored() {
            return Ok(());
        }

        self.frame_backend.prepare_shutdown()?;
        self.terminal_session.shutdown_at_current_cursor()
    }

    pub fn clear_screen(&mut self) -> Result<()> {
        self.display_model.transcript.clear_visible();
        self.frame_backend.clear_screen()?;
        self.dirty = true;
        Ok(())
    }

    pub fn redraw(&mut self) -> Result<()> {
        let size = self.frame_backend.refresh_size()?;
        self.current_pane = Rect::new(0, 0, size.width, size.height);
        self.sync_display_model_live_state();
        let frame = self.display_renderer.render(&self.display_model, size);
        self.frame_backend.draw_frame(frame)?;
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
                    let text = normalize_paste_text(&text);
                    if self.history_search.open {
                        self.history_search
                            .query
                            .push_str(&history_search_paste_text(&text));
                        self.refresh_history_search_results();
                    } else if self.editor_enabled() {
                        self.editor.on_paste(text);
                    }
                    self.request_redraw();
                    return Ok(None);
                }
                Event::Resize(_, _) => {
                    self.frame_backend.refresh_size()?;
                    self.request_redraw();
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        let context = self.global_key_context();
        if !is_exit_confirmation_key(key, context) {
            self.exit_confirmation_pending = false;
        }

        if self.history_search.open {
            return self.handle_history_search_key(key);
        }
        if self.palette_open {
            return self.handle_palette_key(key);
        }

        match classify_global_key(key, context) {
            GlobalKey::OpenHistorySearch => {
                self.open_history_search();
                None
            }
            GlobalKey::OpenPalette => {
                self.palette_open = true;
                None
            }
            GlobalKey::InterruptOrClear => {
                if self.editor.awaiting_input() {
                    Some(UiAction::Interrupt)
                } else if self.editor_enabled() && !self.editor.is_empty() {
                    let _ = self.clear_editor_view();
                    None
                } else if self.status == KernelStatus::Busy {
                    Some(UiAction::Interrupt)
                } else {
                    None
                }
            }
            GlobalKey::ExitOrConfirm => {
                if self.exit_confirmation_pending {
                    Some(UiAction::Exit)
                } else {
                    self.exit_confirmation_pending = true;
                    None
                }
            }
            GlobalKey::ClearScreen => Some(UiAction::ClearScreen),
            GlobalKey::HistoryUp => {
                self.editor.history_up();
                None
            }
            GlobalKey::HistoryDown => {
                self.editor.history_down();
                None
            }
            GlobalKey::InsertIndent => {
                self.editor.insert_indent();
                None
            }
            GlobalKey::AcceptGhostSuggestion => {
                self.editor.accept_ghost_suggestion();
                None
            }
            GlobalKey::InsertLineBreak => {
                self.editor.insert_line_break();
                None
            }
            GlobalKey::Submit => {
                let text = self.editor.take_text();
                if self.editor.take_pending_stdin().is_some() {
                    Some(UiAction::ReplyInput { value: text })
                } else {
                    if text.trim().is_empty() {
                        return None;
                    }
                    self.editor.push_history(text.clone());
                    Some(UiAction::Submit(text))
                }
            }
            GlobalKey::Editor => {
                if !self.editor.handle_extension_key(key) {
                    self.editor.on_key(key);
                }
                None
            }
            GlobalKey::Ignored => None,
        }
    }

    fn global_key_context(&self) -> GlobalKeyContext {
        GlobalKeyContext {
            editor_enabled: self.editor_enabled(),
            awaiting_input: self.editor.awaiting_input(),
            editor_empty: self.editor.is_empty(),
            editor_single_line: self.editor.is_single_line(),
            editor_has_history: self.editor.has_history(),
            editor_insert_mode: self.editor.mode() == EditorMode::Insert,
            submit_ready: self.submit_ready(),
            ghost_suggestion_available: if self.editor_enabled() {
                self.editor.ghost_suggestion_suffix().is_some()
            } else {
                false
            },
        }
    }

    fn handle_history_search_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        match classify_history_search_key(key) {
            HistorySearchKey::Close => {
                self.history_search.open = false;
                None
            }
            HistorySearchKey::Up => {
                self.history_search.selected = self.history_search.selected.saturating_sub(1);
                self.adjust_history_search_scroll();
                None
            }
            HistorySearchKey::Down => {
                let max = self.history_search.results.len().saturating_sub(1);
                self.history_search.selected = (self.history_search.selected + 1).min(max);
                self.adjust_history_search_scroll();
                None
            }
            HistorySearchKey::Load => {
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
            HistorySearchKey::Backspace => {
                self.history_search.query.pop();
                self.refresh_history_search_results();
                None
            }
            HistorySearchKey::Cycle => {
                let max = self.history_search.results.len().saturating_sub(1);
                self.history_search.selected = (self.history_search.selected + 1).min(max);
                self.adjust_history_search_scroll();
                None
            }
            HistorySearchKey::Insert(ch) => {
                self.history_search.query.push(ch);
                self.refresh_history_search_results();
                None
            }
            HistorySearchKey::Ignored => None,
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) -> Option<UiAction> {
        match classify_palette_key(key) {
            PaletteKey::Close => {
                self.palette_open = false;
                None
            }
            PaletteKey::Up => {
                self.palette_index = self.palette_index.saturating_sub(1);
                None
            }
            PaletteKey::Down => {
                self.palette_index = (self.palette_index + 1).min(palette_items().len() - 1);
                None
            }
            PaletteKey::Select => {
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
            PaletteKey::Ignored => None,
        }
    }

    fn open_history_search(&mut self) {
        self.palette_open = false;
        self.history_search.open = true;
        self.history_search.query.clear();
        self.refresh_history_search_results();
    }

    fn refresh_history_search_results(&mut self) {
        self.history_search.refresh_results(&self.history_entries);
    }

    fn adjust_history_search_scroll(&mut self) {
        let visible_rows = self.history_search_visible_rows();
        if visible_rows == 0 {
            self.history_search.scroll = 0;
            return;
        }

        self.history_search.scroll = history_search_scroll_for_selection(
            self.history_search.results.len(),
            visible_rows,
            self.history_search.selected,
        );
    }

    fn history_search_visible_rows(&self) -> usize {
        self.history_search_layout().0
    }

    fn history_search_preview_rows(&self) -> usize {
        self.history_search_layout().1
    }

    fn history_search_layout(&self) -> (usize, usize) {
        let (result_rows, preview_rows) = history_search_layout_for_popup(
            self.current_pane.height.saturating_sub(1),
            self.history_search.results.len(),
            self.history_search_selected_preview_rows(),
        );
        (result_rows as usize, preview_rows as usize)
    }

    fn history_search_selected_preview_rows(&self) -> usize {
        self.history_search
            .results
            .get(self.history_search.selected)
            .and_then(|&entry_index| self.history_entries.get(entry_index))
            .map(|entry| entry.line_count)
            .unwrap_or(1)
            .min(HISTORY_SEARCH_MAX_PREVIEW_ROWS)
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

    fn editor_enabled(&self) -> bool {
        !matches!(self.status, KernelStatus::Disconnected)
    }

    fn submit_ready(&self) -> bool {
        self.session_ready
            && !matches!(self.status, KernelStatus::Busy | KernelStatus::Disconnected)
    }

    fn prompt_label(&self) -> String {
        input_prompt_label(self.last_execution_count)
    }

    fn status_spins(&self) -> bool {
        matches!(self.status, KernelStatus::Busy)
            || (self.status == KernelStatus::Connecting && !self.session_ready)
    }

    fn clear_editor_view(&mut self) -> Result<()> {
        self.editor.reset();
        self.dirty = true;
        Ok(())
    }
}

impl Drop for AppUi {
    fn drop(&mut self) {
        if !self.terminal_session.is_restored() {
            let _ = self.shutdown();
        }
    }
}
