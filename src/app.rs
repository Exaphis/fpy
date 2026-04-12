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
                if ui.needs_animation() {
                    tokio::select! {
                        event = session.kernel_events.recv() => {
                            match event {
                                Some(event) => {
                                    if handle_kernel_event(&mut ui, event)? {
                                        break;
                                    }
                                }
                                None => break,
                            }
                        }
                        _ = liveness_check.tick() => {
                            if let Some(message) = session.kernel.poll_local_exit()? {
                                ui.set_status(KernelStatus::Disconnected);
                                ui.insert_transcript(message)?;
                                break;
                            }
                        }
                        _ = ui_tick.tick() => {}
                        input = ui.next_action() => {
                            if let Some(action) = input? {
                                if handle_ready_ui_action(&mut ui, &mut session.kernel, action).await? {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    tokio::select! {
                        event = session.kernel_events.recv() => {
                            match event {
                                Some(event) => {
                                    if handle_kernel_event(&mut ui, event)? {
                                        break;
                                    }
                                }
                                None => break,
                            }
                        }
                        _ = liveness_check.tick() => {
                            if let Some(message) = session.kernel.poll_local_exit()? {
                                ui.set_status(KernelStatus::Disconnected);
                                ui.insert_transcript(message)?;
                                break;
                            }
                        }
                        input = ui.next_action() => {
                            if let Some(action) = input? {
                                if handle_ready_ui_action(&mut ui, &mut session.kernel, action).await? {
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                if ui.needs_animation() {
                    tokio::select! {
                        bootstrap_result = &mut bootstrap_rx => {
                            let (kernel, kernel_events) = bootstrap_result
                                .map_err(|_| anyhow::anyhow!("kernel startup task terminated unexpectedly"))??;
                            ui.set_connection_summary(kernel.connection_summary());
                            active_session = Some(ActiveSession { kernel, kernel_events });
                        }
                        _ = ui_tick.tick() => {}
                        input = ui.next_action() => {
                            if let Some(action) = input? {
                                if handle_pending_ui_action(&mut ui, action)? {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    tokio::select! {
                        bootstrap_result = &mut bootstrap_rx => {
                            let (kernel, kernel_events) = bootstrap_result
                                .map_err(|_| anyhow::anyhow!("kernel startup task terminated unexpectedly"))??;
                            ui.set_connection_summary(kernel.connection_summary());
                            active_session = Some(ActiveSession { kernel, kernel_events });
                        }
                        input = ui.next_action() => {
                            if let Some(action) = input? {
                                if handle_pending_ui_action(&mut ui, action)? {
                                    break;
                                }
                            }
                        }
                    }
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
}

fn start_bootstrap(
    cli: Cli,
) -> Result<(
    String,
    JoinHandle<()>,
    oneshot::Receiver<
        Result<(
            KernelSession,
            tokio::sync::mpsc::UnboundedReceiver<KernelEvent>,
        )>,
    >,
)> {
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

fn handle_kernel_event(ui: &mut AppUi, event: KernelEvent) -> Result<bool> {
    match event {
        KernelEvent::Connected(summary) => {
            ui.set_connection_summary(summary);
        }
        KernelEvent::Status(status) => {
            ui.set_status(status);
            if status == KernelStatus::Disconnected {
                return Ok(true);
            }
        }
        KernelEvent::ExecuteInput {
            execution_count,
            code,
        } => {
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
        UiAction::Submit(_) | UiAction::ReplyInput(_) => {
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
    action: UiAction,
) -> Result<bool> {
    match action {
        UiAction::Submit(code) => {
            kernel.execute(code)?;
            ui.set_status(KernelStatus::Busy);
            Ok(false)
        }
        UiAction::ReplyInput(value) => {
            ui.insert_transcript(format!("stdin> {value}"))?;
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
            match kernel.restart().await {
                Ok(()) => {
                    ui.set_connection_summary(kernel.connection_summary());
                    ui.set_status(KernelStatus::Connecting);
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
