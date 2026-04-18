use std::time::{Duration as ElapsedDuration, Instant};

use anyhow::Result;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{self, Duration, MissedTickBehavior},
};

use crate::{
    cli::{Cli, Command},
    connection::ConnectionFile,
    kernel::{KernelEvent, KernelSession, KernelStatus, LaunchConfig},
    ui::{AppUi, UiAction},
};

pub async fn run(cli: Cli) -> Result<()> {
    let (startup_message, bootstrap_task, mut bootstrap_rx) = start_bootstrap(cli)?;
    let mut ui = AppUi::new("starting".to_string())?;
    ui.insert_transcript(startup_message)?;
    let mut active_session: Option<ActiveSession> = None;
    let mut ui_tick = time::interval(Duration::from_millis(120));
    ui_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut liveness_check = time::interval(Duration::from_millis(100));
    liveness_check.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let result: Result<()> = async {
        loop {
            ui.redraw()?;

            if let Some(session) = active_session.as_mut() {
                if run_active_iteration(&mut ui, session, &mut liveness_check, &mut ui_tick).await?
                {
                    break;
                }
            } else {
                match run_bootstrap_iteration(&mut ui, &mut bootstrap_rx, &mut ui_tick).await? {
                    BootstrapOutcome::Continue => {}
                    BootstrapOutcome::Exit => break,
                    BootstrapOutcome::Activated(session) => active_session = Some(*session),
                }
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
        self.started_at.take().map(|started_at| started_at.elapsed())
    }

    fn clear(&mut self) {
        self.started_at = None;
    }
}

fn start_bootstrap(cli: Cli) -> Result<(String, JoinHandle<()>, BootstrapReceiver)> {
    let startup_message = match &cli.command {
        Command::Run(_) => "starting local kernel...".to_string(),
        Command::Attach(args) => format!("connecting to {}...", args.connection_file.display()),
    };

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

    Ok((startup_message, task, rx))
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
                handle_kernel_event_stream(ui, &mut session.execution_timer, event)
            }
            _ = liveness_check.tick() => {
                handle_local_kernel_liveness(ui, &mut session.kernel, &mut session.execution_timer)
            }
            _ = ui_tick.tick() => Ok(false),
            input = ui.next_action() => {
                handle_ready_input(ui, &mut session.kernel, &mut session.execution_timer, input).await
            }
        }
    } else {
        tokio::select! {
            event = session.kernel_events.recv() => {
                handle_kernel_event_stream(ui, &mut session.execution_timer, event)
            }
            _ = liveness_check.tick() => {
                handle_local_kernel_liveness(ui, &mut session.kernel, &mut session.execution_timer)
            }
            input = ui.next_action() => {
                handle_ready_input(ui, &mut session.kernel, &mut session.execution_timer, input).await
            }
        }
    }
}

async fn run_bootstrap_iteration(
    ui: &mut AppUi,
    bootstrap_rx: &mut BootstrapReceiver,
    ui_tick: &mut time::Interval,
) -> Result<BootstrapOutcome> {
    if ui.needs_animation() {
        tokio::select! {
            bootstrap_result = bootstrap_rx => {
                activate_bootstrap_result(ui, bootstrap_result)
            }
            _ = ui_tick.tick() => Ok(BootstrapOutcome::Continue),
            input = ui.next_action() => {
                handle_pending_input(ui, input)
            }
        }
    } else {
        tokio::select! {
            bootstrap_result = bootstrap_rx => {
                activate_bootstrap_result(ui, bootstrap_result)
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
) -> Result<BootstrapOutcome> {
    let bootstrap_result = bootstrap_result
        .map_err(|_| anyhow::anyhow!("kernel startup task terminated unexpectedly"))?;
    let (kernel, kernel_events) = bootstrap_result?;
    ui.set_connection_summary(kernel.connection_summary());
    ui.mark_session_ready();
    Ok(BootstrapOutcome::Activated(Box::new(ActiveSession {
        kernel,
        kernel_events,
        execution_timer: ExecutionTimer::default(),
    })))
}

fn handle_kernel_event_stream(
    ui: &mut AppUi,
    execution_timer: &mut ExecutionTimer,
    event: Option<KernelEvent>,
) -> Result<bool> {
    match event {
        Some(event) => handle_kernel_event(ui, execution_timer, event),
        None => Ok(true),
    }
}

fn handle_local_kernel_liveness(
    ui: &mut AppUi,
    kernel: &mut KernelSession,
    execution_timer: &mut ExecutionTimer,
) -> Result<bool> {
    if let Some(message) = kernel.poll_local_exit()? {
        execution_timer.clear();
        ui.set_status(KernelStatus::Disconnected);
        ui.insert_transcript(message)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn handle_ready_input(
    ui: &mut AppUi,
    kernel: &mut KernelSession,
    execution_timer: &mut ExecutionTimer,
    input: Result<Option<UiAction>>,
) -> Result<bool> {
    if let Some(action) = input? {
        handle_ready_ui_action(ui, kernel, execution_timer, action).await
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
                    if let Some(duration) = execution_timer.finish() {
                        ui.insert_runtime(duration)?;
                    }
                }
                KernelStatus::Disconnected => {
                    execution_timer.clear();
                    return Ok(true);
                }
                _ => {}
            }
        }
        KernelEvent::ExecuteInput {
            execution_count,
            code,
        } => {
            execution_timer.start();
            ui.set_last_execution_count(execution_count);
            ui.insert_execute_input(execution_count, &code)?;
        }
        KernelEvent::ExecuteResult {
            execution_count,
            text,
        } => {
            ui.set_last_execution_count(execution_count);
            let prompt = execution_count
                .map(|count| format!("Out[{count}]"))
                .unwrap_or_else(|| "Out[?]".to_string());
            ui.insert_transcript(format!("{prompt}: {text}"))?;
        }
        KernelEvent::Stream { name, text } => {
            if name == "stderr" {
                ui.insert_transcript(format!("stderr: {text}"))?;
            } else {
                ui.insert_transcript(text)?;
            }
        }
        KernelEvent::Error { traceback } => {
            ui.clear_input_request();
            ui.insert_transcript(traceback.join("\n"))?;
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
            execution_timer.clear();
            ui.set_status(KernelStatus::Disconnected);
            ui.insert_transcript(format!("fatal: {text}"))?;
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
    action: UiAction,
) -> Result<bool> {
    match action {
        UiAction::Submit(code) => {
            kernel.execute(code)?;
            ui.set_status(KernelStatus::Busy);
            Ok(false)
        }
        UiAction::ReplyInput {
            value,
            prompt,
            password,
        } => {
            if let Some(prompt) = prompt.filter(|_| !password) {
                ui.insert_transcript(format!("{prompt}{value}"))?;
            }
            kernel.send_input_reply(value)?;
            ui.set_status(KernelStatus::Busy);
            Ok(false)
        }
        UiAction::Interrupt => {
            match kernel.interrupt() {
                Ok(()) => ui.insert_transcript("interrupt sent")?,
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
                    ui.set_connection_summary(kernel.connection_summary());
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
