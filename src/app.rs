use std::{
    path::PathBuf,
    time::{Duration as ElapsedDuration, Instant},
};

use anyhow::Result;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{self, Duration, MissedTickBehavior},
};

use crate::{
    cli::{Cli, Command},
    connection::ConnectionFile,
    frontend_magic::{
        FrontendMagic, FrontendMagicParse, OutputCaptureStore, copy_to_clipboard,
        parse_frontend_magic, resolve_error_message,
    },
    history::{self, HistoryOutcome, HistorySession},
    kernel::{KernelEvent, KernelSession, KernelStatus, LaunchConfig},
    ui::{AppUi, UiAction},
};

pub async fn run(cli: Cli) -> Result<()> {
    let history_root = history::default_root_dir().ok();
    let mut history_warnings = Vec::new();
    let loaded_history = if let Some(root) = history_root.as_deref() {
        match history::load_entries(root) {
            Ok(entries) => entries,
            Err(error) => {
                history_warnings.push(format!("persistent history load failed: {error}"));
                Vec::new()
            }
        }
    } else {
        history_warnings.push("persistent history disabled: HOME is not set".to_string());
        Vec::new()
    };

    let (bootstrap_task, mut bootstrap_rx) = start_bootstrap(cli)?;
    let mut ui = AppUi::new("starting".to_string())?;
    ui.load_history(loaded_history);
    ui.insert_transcript(format!("fpy {}", env!("CARGO_PKG_VERSION")))?;
    for warning in history_warnings {
        ui.insert_transcript(format!("warning: {warning}"))?;
    }
    let mut active_session: Option<ActiveSession> = None;
    let mut ui_tick = time::interval(Duration::from_millis(120));
    ui_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut liveness_check = time::interval(Duration::from_millis(100));
    liveness_check.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let result: Result<()> = async {
        ui.redraw()?;

        loop {
            let should_exit = if let Some(session) = active_session.as_mut() {
                run_active_iteration(&mut ui, session, &mut liveness_check, &mut ui_tick).await?
            } else {
                match run_bootstrap_iteration(
                    &mut ui,
                    &mut bootstrap_rx,
                    &mut ui_tick,
                    history_root.as_ref(),
                )
                .await?
                {
                    BootstrapOutcome::Continue => false,
                    BootstrapOutcome::Exit => true,
                    BootstrapOutcome::Activated(session) => {
                        active_session = Some(*session);
                        false
                    }
                }
            };

            if ui.needs_redraw() {
                ui.redraw()?;
            }

            if should_exit {
                break;
            }
        }

        Ok(())
    }
    .await;

    bootstrap_task.abort();
    let shutdown_result = match active_session.as_mut() {
        Some(session) => session.kernel.shutdown().await,
        None => Ok(()),
    };
    let ui_result = ui.shutdown();
    result?;
    shutdown_result?;
    ui_result?;
    Ok(())
}

struct ActiveSession {
    kernel: KernelSession,
    kernel_events: tokio::sync::mpsc::UnboundedReceiver<KernelEvent>,
    execution_timer: ExecutionTimer,
    history: Option<HistorySession>,
    pending_history: Option<PendingHistoryEntry>,
    captures: OutputCaptureStore,
}

struct PendingHistoryEntry {
    entry_seq: Option<u64>,
    ui_history_index: usize,
    outcome: HistoryOutcome,
}

type BootstrapReceiver = oneshot::Receiver<Result<BootstrappedSession>>;
type BootstrappedSession = (
    KernelSession,
    tokio::sync::mpsc::UnboundedReceiver<KernelEvent>,
);
type BootstrapTaskResult = Result<BootstrappedSession>;

enum BootstrapOutcome {
    Continue,
    Exit,
    Activated(Box<ActiveSession>),
}

#[derive(Default)]
struct ExecutionTimer {
    started_at: Option<Instant>,
}

impl ExecutionTimer {
    fn start(&mut self) {
        self.started_at = Some(Instant::now());
    }

    fn finish(&mut self) -> Option<ElapsedDuration> {
        self.started_at
            .take()
            .map(|started_at| started_at.elapsed())
    }

    fn clear(&mut self) {
        self.started_at = None;
    }
}

fn start_bootstrap(cli: Cli) -> Result<(JoinHandle<()>, BootstrapReceiver)> {
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let result = match cli.command {
            Command::Run(args) => {
                let launch = LaunchConfig {
                    python: args.python,
                    kernel_args: args.kernel_args,
                };
                KernelSession::launch(launch).await
            }
            Command::Attach(args) => match ConnectionFile::read(&args.connection_file) {
                Ok(connection) => KernelSession::attach(connection).await,
                Err(error) => Err(error),
            },
        };
        let _ = tx.send(result);
    });

    Ok((task, rx))
}

async fn run_active_iteration(
    ui: &mut AppUi,
    session: &mut ActiveSession,
    liveness_check: &mut time::Interval,
    ui_tick: &mut time::Interval,
) -> Result<bool> {
    if ui.needs_animation() {
        tokio::select! {
            event = session.kernel_events.recv() => {
                handle_kernel_event_stream(
                    ui,
                    &mut session.execution_timer,
                    &mut session.history,
                    &mut session.pending_history,
                    &mut session.captures,
                    event,
                )
            }
            _ = liveness_check.tick() => {
                handle_local_kernel_liveness(
                    ui,
                    &mut session.kernel,
                    &mut session.execution_timer,
                    &mut session.pending_history,
                    &mut session.captures,
                )
            }
            _ = ui_tick.tick() => {
                ui.request_redraw();
                Ok(false)
            },
            input = ui.next_action() => {
                handle_ready_input(
                    ui,
                    &mut session.kernel,
                    &mut session.execution_timer,
                    &mut session.history,
                    &mut session.pending_history,
                    &mut session.captures,
                    input,
                ).await
            }
        }
    } else {
        tokio::select! {
            event = session.kernel_events.recv() => {
                handle_kernel_event_stream(
                    ui,
                    &mut session.execution_timer,
                    &mut session.history,
                    &mut session.pending_history,
                    &mut session.captures,
                    event,
                )
            }
            _ = liveness_check.tick() => {
                handle_local_kernel_liveness(
                    ui,
                    &mut session.kernel,
                    &mut session.execution_timer,
                    &mut session.pending_history,
                    &mut session.captures,
                )
            }
            input = ui.next_action() => {
                handle_ready_input(
                    ui,
                    &mut session.kernel,
                    &mut session.execution_timer,
                    &mut session.history,
                    &mut session.pending_history,
                    &mut session.captures,
                    input,
                ).await
            }
        }
    }
}

async fn run_bootstrap_iteration(
    ui: &mut AppUi,
    bootstrap_rx: &mut BootstrapReceiver,
    ui_tick: &mut time::Interval,
    history_root: Option<&PathBuf>,
) -> Result<BootstrapOutcome> {
    if ui.needs_animation() {
        tokio::select! {
            bootstrap_result = bootstrap_rx => {
                activate_bootstrap_result(ui, bootstrap_result, history_root)
            }
            _ = ui_tick.tick() => {
                ui.request_redraw();
                Ok(BootstrapOutcome::Continue)
            },
            input = ui.next_action() => {
                handle_pending_input(ui, input)
            }
        }
    } else {
        tokio::select! {
            bootstrap_result = bootstrap_rx => {
                activate_bootstrap_result(ui, bootstrap_result, history_root)
            }
            input = ui.next_action() => {
                handle_pending_input(ui, input)
            }
        }
    }
}

fn activate_bootstrap_result(
    ui: &mut AppUi,
    bootstrap_result: Result<BootstrapTaskResult, oneshot::error::RecvError>,
    history_root: Option<&PathBuf>,
) -> Result<BootstrapOutcome> {
    let bootstrap_result = bootstrap_result
        .map_err(|_| anyhow::anyhow!("kernel startup task terminated unexpectedly"))?;
    let (kernel, kernel_events) = bootstrap_result?;
    ui.set_connection_summary(kernel.connection_summary());
    ui.mark_session_ready();
    let history = if let Some(root) = history_root {
        match HistorySession::open(root) {
            Ok(history) => Some(history),
            Err(error) => {
                ui.insert_transcript(format!("warning: persistent history disabled: {error}"))?;
                None
            }
        }
    } else {
        None
    };
    Ok(BootstrapOutcome::Activated(Box::new(ActiveSession {
        kernel,
        kernel_events,
        execution_timer: ExecutionTimer::default(),
        history,
        pending_history: None,
        captures: OutputCaptureStore::default(),
    })))
}

fn handle_kernel_event_stream(
    ui: &mut AppUi,
    execution_timer: &mut ExecutionTimer,
    history: &mut Option<HistorySession>,
    pending_history: &mut Option<PendingHistoryEntry>,
    captures: &mut OutputCaptureStore,
    event: Option<KernelEvent>,
) -> Result<bool> {
    match event {
        Some(event) => handle_kernel_event(
            ui,
            execution_timer,
            history,
            pending_history,
            captures,
            event,
        ),
        None => {
            captures.finish_active();
            execution_timer.clear();
            pending_history.take();
            ui.set_status(KernelStatus::Disconnected);
            ui.insert_transcript("Kernel exited unexpectedly")?;
            ui.redraw()?;
            Ok(true)
        }
    }
}

fn handle_local_kernel_liveness(
    ui: &mut AppUi,
    kernel: &mut KernelSession,
    execution_timer: &mut ExecutionTimer,
    pending_history: &mut Option<PendingHistoryEntry>,
    captures: &mut OutputCaptureStore,
) -> Result<bool> {
    if let Some(message) = kernel.poll_local_exit()? {
        captures.finish_active();
        execution_timer.clear();
        pending_history.take();
        ui.set_status(KernelStatus::Disconnected);
        ui.insert_transcript(message)?;
        ui.redraw()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn handle_ready_input(
    ui: &mut AppUi,
    kernel: &mut KernelSession,
    execution_timer: &mut ExecutionTimer,
    history: &mut Option<HistorySession>,
    pending_history: &mut Option<PendingHistoryEntry>,
    captures: &mut OutputCaptureStore,
    input: Result<Option<UiAction>>,
) -> Result<bool> {
    if let Some(action) = input? {
        handle_ready_ui_action(
            ui,
            kernel,
            execution_timer,
            history,
            pending_history,
            captures,
            action,
        )
        .await
    } else {
        Ok(false)
    }
}

fn handle_pending_input(
    ui: &mut AppUi,
    input: Result<Option<UiAction>>,
) -> Result<BootstrapOutcome> {
    if let Some(action) = input?
        && handle_pending_ui_action(ui, action)?
    {
        Ok(BootstrapOutcome::Exit)
    } else {
        Ok(BootstrapOutcome::Continue)
    }
}

fn handle_kernel_event(
    ui: &mut AppUi,
    execution_timer: &mut ExecutionTimer,
    history: &mut Option<HistorySession>,
    pending_history: &mut Option<PendingHistoryEntry>,
    captures: &mut OutputCaptureStore,
    event: KernelEvent,
) -> Result<bool> {
    match event {
        KernelEvent::Connected(summary) => {
            ui.set_connection_summary(summary);
        }
        KernelEvent::Status(status) => {
            ui.set_status(status);
            match status {
                KernelStatus::Idle => {
                    captures.finish_active();
                    if let Some(duration) = execution_timer.finish() {
                        if let Some(entry) = pending_history.take() {
                            persist_history_done(ui, history, &entry, duration)?;
                            ui.record_history_completion(
                                entry.ui_history_index,
                                duration,
                                entry.outcome,
                            );
                        }
                        ui.insert_runtime(duration)?;
                    }
                }
                KernelStatus::Disconnected => {
                    captures.finish_active();
                    execution_timer.clear();
                    pending_history.take();
                    ui.insert_transcript("Kernel exited unexpectedly")?;
                    ui.redraw()?;
                    return Ok(true);
                }
                _ => {}
            }
        }
        KernelEvent::ExecuteInput {
            execution_count,
            code,
        } => {
            captures.begin_cell(execution_count, &code);
            execution_timer.start();
            ui.set_last_execution_count(execution_count);
            ui.insert_execute_input(execution_count, &code)?;
        }
        KernelEvent::ExecuteResult {
            execution_count,
            text,
        } => {
            captures.append_execute_result(execution_count, &text);
            ui.set_last_execution_count(execution_count);
            ui.insert_execute_result(execution_count, &text)?;
        }
        KernelEvent::Pager { text } => {
            ui.insert_transcript(text)?;
        }
        KernelEvent::Stream { name, text } => {
            captures.append_stream(name, &text);
            ui.insert_stream(name, &text)?;
        }
        KernelEvent::Error { traceback } => {
            if let Some(entry) = pending_history.as_mut() {
                entry.outcome = if traceback
                    .iter()
                    .any(|line| line.contains("KeyboardInterrupt"))
                {
                    HistoryOutcome::Interrupted
                } else {
                    HistoryOutcome::Error
                };
            }
            ui.clear_input_request();
            ui.insert_error(&traceback)?;
        }
        KernelEvent::InputRequest { prompt, password } => {
            ui.begin_input_request(prompt, password);
        }
        KernelEvent::Info(text) => {
            ui.clear_input_request();
            ui.insert_transcript(text)?;
        }
        KernelEvent::Warning(text) => {
            ui.insert_transcript(format!("warning: {text}"))?;
        }
        KernelEvent::Fatal(text) => {
            captures.finish_active();
            execution_timer.clear();
            pending_history.take();
            ui.set_status(KernelStatus::Disconnected);
            ui.insert_transcript(format!("fatal: {text}"))?;
            ui.redraw()?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn handle_pending_ui_action(ui: &mut AppUi, action: UiAction) -> Result<bool> {
    match action {
        UiAction::Exit => Ok(true),
        UiAction::ClearScreen => {
            ui.clear_screen()?;
            Ok(false)
        }
        UiAction::Submit(_) | UiAction::ReplyInput { .. } => {
            ui.insert_transcript("kernel is still starting")?;
            Ok(false)
        }
        UiAction::Interrupt | UiAction::Restart => {
            ui.insert_transcript("kernel is still starting")?;
            Ok(false)
        }
        UiAction::ShowConnectionInfo => {
            ui.insert_transcript("kernel is still starting")?;
            Ok(false)
        }
    }
}

async fn handle_ready_ui_action(
    ui: &mut AppUi,
    kernel: &mut KernelSession,
    execution_timer: &mut ExecutionTimer,
    history: &mut Option<HistorySession>,
    pending_history: &mut Option<PendingHistoryEntry>,
    captures: &mut OutputCaptureStore,
    action: UiAction,
) -> Result<bool> {
    match action {
        UiAction::Submit(code) => {
            match parse_frontend_magic(&code) {
                FrontendMagicParse::Magic(magic) => {
                    ui.discard_editor_history_submission(&code);
                    handle_frontend_magic(ui, captures, magic)?;
                    return Ok(false);
                }
                FrontendMagicParse::Error(message) => {
                    ui.discard_editor_history_submission(&code);
                    ui.insert_transcript(format!("fpy: {message}"))?;
                    return Ok(false);
                }
                FrontendMagicParse::NotFrontendMagic => {}
            }

            let ui_history_index = ui.record_history_submission(&code);
            let mut entry_seq = None;
            if let Some(history_session) = history.as_mut() {
                match history_session.append_cell(&code) {
                    Ok(seq) => entry_seq = Some(seq),
                    Err(error) => {
                        *history = None;
                        ui.insert_transcript(format!(
                            "warning: persistent history disabled: {error}"
                        ))?;
                    }
                }
            }
            *pending_history = Some(PendingHistoryEntry {
                entry_seq,
                ui_history_index,
                outcome: HistoryOutcome::Ok,
            });
            ui.show_submitted_code(&code)?;
            ui.redraw()?;
            kernel.execute(code)?;
            Ok(false)
        }
        UiAction::ReplyInput { value } => {
            ui.record_input_reply(&value);
            kernel.send_input_reply(value)?;
            ui.set_status(KernelStatus::Busy);
            Ok(false)
        }
        UiAction::Interrupt => {
            match kernel.interrupt() {
                Ok(()) => {
                    execution_timer.clear();
                    pending_history.take();
                    ui.insert_transcript("^C")?;
                }
                Err(error) => ui.insert_transcript(format!("interrupt unavailable: {error}"))?,
            }
            Ok(false)
        }
        UiAction::ClearScreen => {
            ui.clear_screen()?;
            Ok(false)
        }
        UiAction::Exit => Ok(true),
        UiAction::Restart => {
            execution_timer.clear();
            match kernel.restart().await {
                Ok(()) => {
                    captures.finish_active();
                    pending_history.take();
                    ui.set_connection_summary(kernel.connection_summary());
                    ui.reset_last_execution_count();
                    ui.mark_session_ready();
                    ui.insert_transcript("kernel restarted")?;
                }
                Err(error) => ui.insert_transcript(format!("restart unavailable: {error}"))?,
            }
            Ok(false)
        }
        UiAction::ShowConnectionInfo => {
            ui.insert_transcript(ui.connection_summary().to_string())?;
            Ok(false)
        }
    }
}

fn handle_frontend_magic(
    ui: &mut AppUi,
    captures: &OutputCaptureStore,
    magic: FrontendMagic,
) -> Result<()> {
    match magic {
        FrontendMagic::PrintOutput { target } => match captures.resolve_output(target) {
            Ok(resolved) => ui.insert_transcript(resolved.output),
            Err(error) => ui.insert_transcript(resolve_error_message(error)),
        },
        FrontendMagic::ClipOutput { target } => match captures.resolve_output(target) {
            Ok(resolved) => match copy_to_clipboard(resolved.output) {
                Ok(()) => ui.insert_transcript(format!(
                    "fpy: copied output from In [{}] to clipboard",
                    resolved.execution_count
                )),
                Err(error) => ui.insert_transcript(format!("fpy: {error}")),
            },
            Err(error) => ui.insert_transcript(resolve_error_message(error)),
        },
    }
}

fn persist_history_done(
    ui: &mut AppUi,
    history: &mut Option<HistorySession>,
    entry: &PendingHistoryEntry,
    duration: ElapsedDuration,
) -> Result<()> {
    if let (Some(history_session), Some(entry_seq)) = (history.as_mut(), entry.entry_seq)
        && let Err(error) = history_session.append_done(entry_seq, duration, entry.outcome)
    {
        *history = None;
        ui.insert_transcript(format!("warning: persistent history disabled: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ExecutionTimer;

    #[test]
    fn execution_timer_finishes_only_once() {
        let mut timer = ExecutionTimer::default();
        timer.start();
        assert!(timer.finish().is_some());
        assert!(timer.finish().is_none());
    }

    #[test]
    fn execution_timer_clear_discards_pending_runtime() {
        let mut timer = ExecutionTimer::default();
        timer.start();
        timer.clear();
        assert!(timer.finish().is_none());
    }
}
