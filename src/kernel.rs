use std::{
    fs,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tempfile::{NamedTempFile, TempDir};
use tokio::{
    process::{Child, Command},
    sync::mpsc,
    task::JoinHandle,
    time::sleep,
};
use zeromq::{DealerSocket, Socket, SocketRecv, SocketSend, SubSocket};

use crate::{
    connection::{Channel, ConnectionFile},
    jupyter::{MessageCodec, WireMessage},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelStatus {
    Connecting,
    Idle,
    Busy,
    AwaitingInput,
    Disconnected,
}

#[derive(Debug)]
pub enum KernelEvent {
    Connected(String),
    Status(KernelStatus),
    ExecuteInput {
        execution_count: Option<u32>,
        code: String,
    },
    ExecuteResult {
        execution_count: Option<u32>,
        text: String,
    },
    Stream {
        name: String,
        text: String,
    },
    Error {
        traceback: Vec<String>,
    },
    InputRequest {
        prompt: String,
        password: bool,
    },
    Info(String),
    Warning(String),
    Fatal(String),
}

enum KernelCommand {
    Execute { code: String },
    InputReply { value: String },
    KernelInfo,
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct LaunchConfig {
    pub python: String,
    pub kernel_args: Vec<String>,
}

struct LocalKernel {
    child: Child,
    _connection_dir: TempDir,
    connection_file: PathBuf,
    startup_log: NamedTempFile,
    launch: LaunchConfig,
}

struct Runtime {
    connection: ConnectionFile,
    command_tx: mpsc::UnboundedSender<KernelCommand>,
    tasks: Vec<JoinHandle<()>>,
}

pub struct KernelSession {
    runtime: Runtime,
    event_tx: mpsc::UnboundedSender<KernelEvent>,
    local: Option<LocalKernel>,
}

impl KernelSession {
    pub async fn launch(
        launch: LaunchConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<KernelEvent>)> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let mut local = spawn_kernel(&launch).await?;
        let connection = wait_for_connection(&mut local).await?;
        let runtime = Runtime::connect(connection, event_tx.clone()).await?;

        Ok((
            Self {
                runtime,
                event_tx,
                local: Some(local),
            },
            event_rx,
        ))
    }

    pub async fn attach(
        connection: ConnectionFile,
    ) -> Result<(Self, mpsc::UnboundedReceiver<KernelEvent>)> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let runtime = Runtime::connect(connection, event_tx.clone()).await?;
        Ok((
            Self {
                runtime,
                event_tx,
                local: None,
            },
            event_rx,
        ))
    }

    pub fn connection_summary(&self) -> String {
        self.runtime.connection.summary()
    }

    pub fn execute(&self, code: String) -> Result<()> {
        self.runtime
            .command_tx
            .send(KernelCommand::Execute { code })
            .map_err(|_| anyhow!("kernel command loop is not running"))
    }

    pub fn send_input_reply(&self, value: String) -> Result<()> {
        self.runtime
            .command_tx
            .send(KernelCommand::InputReply { value })
            .map_err(|_| anyhow!("kernel stdin loop is not running"))
    }

    pub fn interrupt(&mut self) -> Result<()> {
        match self.local.as_mut() {
            Some(local) => send_sigint(local.child.id()),
            None => bail!("interrupt is only supported for locally launched kernels"),
        }
    }

    pub async fn restart(&mut self) -> Result<()> {
        let launch = match self.local.as_ref() {
            Some(local) => local.launch.clone(),
            None => bail!("restart is only supported for locally launched kernels"),
        };

        self.stop_runtime().await;
        self.shutdown_local_child().await?;
        let mut local = spawn_kernel(&launch).await?;
        let connection = wait_for_connection(&mut local).await?;
        let runtime = Runtime::connect(connection, self.event_tx.clone()).await?;
        self.local = Some(local);
        self.runtime = runtime;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if self.local.is_some() {
            let _ = self.runtime.command_tx.send(KernelCommand::Shutdown);
        }
        self.stop_runtime().await;
        self.shutdown_local_child().await?;
        Ok(())
    }

    pub fn poll_local_exit(&mut self) -> Result<Option<String>> {
        let Some(local) = self.local.as_mut() else {
            return Ok(None);
        };

        match local.child.try_wait()? {
            Some(status) => Ok(Some(format!("kernel exited with status {status}"))),
            None => Ok(None),
        }
    }

    async fn stop_runtime(&mut self) {
        for task in self.runtime.tasks.drain(..) {
            task.abort();
        }
    }

    async fn shutdown_local_child(&mut self) -> Result<()> {
        if let Some(mut local) = self.local.take() {
            if local.child.try_wait()?.is_none() {
                let _ = local.child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(2), local.child.wait()).await;
            }
        }
        Ok(())
    }
}

impl Runtime {
    async fn connect(
        connection: ConnectionFile,
        event_tx: mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Self> {
        let codec = MessageCodec::new(connection.key.clone());

        let mut shell = DealerSocket::new();
        shell.connect(&connection.endpoint(Channel::Shell)).await?;
        let (mut shell_send, mut shell_recv) = shell.split();

        let mut stdin_socket = DealerSocket::new();
        stdin_socket
            .connect(&connection.endpoint(Channel::Stdin))
            .await?;
        let (mut stdin_send, mut stdin_recv) = stdin_socket.split();

        let mut control = DealerSocket::new();
        control
            .connect(&connection.endpoint(Channel::Control))
            .await?;
        let (mut control_send, _control_recv) = control.split();

        let mut iopub = SubSocket::new();
        iopub.connect(&connection.endpoint(Channel::Iopub)).await?;
        iopub.subscribe("").await?;

        let (command_tx, mut command_rx) = mpsc::unbounded_channel();

        let command_codec = codec.clone();
        let command_events = event_tx.clone();
        let command_task = tokio::spawn(async move {
            while let Some(command) = command_rx.recv().await {
                let result = match command {
                    KernelCommand::Execute { code } => {
                        let message = command_codec.message(
                            "execute_request",
                            None,
                            json!({
                                "code": code,
                                "silent": false,
                                "store_history": true,
                                "user_expressions": {},
                                "allow_stdin": true,
                                "stop_on_error": true
                            }),
                        );
                        send_message(&mut shell_send, &command_codec, &message).await
                    }
                    KernelCommand::InputReply { value } => {
                        let message =
                            command_codec.message("input_reply", None, json!({ "value": value }));
                        send_message(&mut stdin_send, &command_codec, &message).await
                    }
                    KernelCommand::KernelInfo => {
                        let message = command_codec.message("kernel_info_request", None, json!({}));
                        send_message(&mut shell_send, &command_codec, &message).await
                    }
                    KernelCommand::Shutdown => {
                        let message = command_codec.message(
                            "shutdown_request",
                            None,
                            json!({ "restart": false }),
                        );
                        send_message(&mut control_send, &command_codec, &message).await
                    }
                };

                if let Err(error) = result {
                    let _ = command_events.send(KernelEvent::Fatal(error.to_string()));
                    break;
                }
            }
        });

        let shell_codec = codec.clone();
        let shell_events = event_tx.clone();
        let shell_task = tokio::spawn(async move {
            loop {
                match shell_recv.recv().await {
                    Ok(message) => match shell_codec.decode(message) {
                        Ok(decoded) => {
                            for event in shell_message_to_events(decoded) {
                                let _ = shell_events.send(event);
                            }
                        }
                        Err(error) => {
                            let _ = shell_events.send(KernelEvent::Warning(error.to_string()));
                        }
                    },
                    Err(error) => {
                        let _ = shell_events.send(KernelEvent::Fatal(error.to_string()));
                        break;
                    }
                }
            }
        });

        let stdin_codec = codec.clone();
        let stdin_events = event_tx.clone();
        let stdin_task = tokio::spawn(async move {
            loop {
                match stdin_recv.recv().await {
                    Ok(message) => match stdin_codec.decode(message) {
                        Ok(decoded) => {
                            for event in stdin_message_to_events(decoded) {
                                let _ = stdin_events.send(event);
                            }
                        }
                        Err(error) => {
                            let _ = stdin_events.send(KernelEvent::Warning(error.to_string()));
                        }
                    },
                    Err(error) => {
                        let _ = stdin_events.send(KernelEvent::Fatal(error.to_string()));
                        break;
                    }
                }
            }
        });

        let iopub_codec = codec;
        let iopub_events = event_tx.clone();
        let iopub_task = tokio::spawn(async move {
            loop {
                match iopub.recv().await {
                    Ok(message) => match iopub_codec.decode(message) {
                        Ok(decoded) => {
                            for event in iopub_message_to_events(decoded) {
                                let _ = iopub_events.send(event);
                            }
                        }
                        Err(error) => {
                            let _ = iopub_events.send(KernelEvent::Warning(error.to_string()));
                        }
                    },
                    Err(error) => {
                        let _ = iopub_events.send(KernelEvent::Fatal(error.to_string()));
                        break;
                    }
                }
            }
        });

        let _ = event_tx.send(KernelEvent::Connected(connection.summary()));
        let _ = event_tx.send(KernelEvent::Status(KernelStatus::Connecting));
        let _ = command_tx.send(KernelCommand::KernelInfo);

        Ok(Self {
            connection,
            command_tx,
            tasks: vec![command_task, shell_task, stdin_task, iopub_task],
        })
    }
}

async fn send_message(
    socket: &mut impl SocketSend,
    codec: &MessageCodec,
    message: &WireMessage,
) -> Result<()> {
    socket.send(codec.into_zmq(message)?).await?;
    Ok(())
}

async fn spawn_kernel(launch: &LaunchConfig) -> Result<LocalKernel> {
    let connection_dir = tempfile::tempdir().context("failed to create temp connection dir")?;
    let startup_log = NamedTempFile::new().context("failed to create kernel startup log")?;
    let path = connection_dir.path().join("kernel-connection.json");

    let mut command = Command::new(&launch.python);
    command
        .arg("-m")
        .arg("ipykernel_launcher")
        .args(&launch.kernel_args)
        .arg("-f")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            startup_log
                .reopen()
                .context("failed to open kernel startup log")?,
        ))
        .kill_on_drop(true);

    let child = command
        .spawn()
        .with_context(|| format!("failed to launch {}", launch.python))?;

    Ok(LocalKernel {
        child,
        _connection_dir: connection_dir,
        connection_file: path,
        startup_log,
        launch: launch.clone(),
    })
}

async fn wait_for_connection(local: &mut LocalKernel) -> Result<ConnectionFile> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_parse_error = None;

    loop {
        let path = &local.connection_file;
        if path.exists() {
            match ConnectionFile::read(path) {
                Ok(connection) => return Ok(connection),
                Err(error) if Instant::now() < deadline => {
                    last_parse_error = Some(error);
                }
                Err(error) => last_parse_error = Some(error),
            }
        }

        if let Some(status) = local.child.try_wait()? {
            bail!(
                "{}",
                startup_failure_message(status, local, last_parse_error.as_ref())
            );
        }

        if Instant::now() >= deadline {
            let _ = local.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), local.child.wait()).await;
            bail!(
                "{}",
                startup_timeout_message(local, last_parse_error.as_ref())
            );
        }

        sleep(Duration::from_millis(10)).await;
    }
}

fn startup_failure_message(
    status: std::process::ExitStatus,
    local: &LocalKernel,
    parse_error: Option<&anyhow::Error>,
) -> String {
    let mut details = vec![
        format!("Python: {}", local.launch.python),
        format!("Status: {status}"),
    ];
    append_startup_context(&mut details, local, parse_error);
    format_diagnostic("Kernel startup failed", &details)
}

fn startup_timeout_message(local: &LocalKernel, parse_error: Option<&anyhow::Error>) -> String {
    let mut details = vec![
        format!("Python: {}", local.launch.python),
        "Reason: timed out waiting for the connection file".to_string(),
    ];
    append_startup_context(&mut details, local, parse_error);
    format_diagnostic("Kernel startup timed out", &details)
}

fn append_startup_context(
    details: &mut Vec<String>,
    local: &LocalKernel,
    parse_error: Option<&anyhow::Error>,
) {
    if let Some(error) = parse_error {
        details.push(format!("Connection file: {error}"));
    }

    if let Some(log_output) = read_startup_log(local.startup_log.path()) {
        details.push(format!("Kernel stderr:\n{log_output}"));
    }
}

fn format_diagnostic(title: &str, details: &[String]) -> String {
    let mut lines = vec![title.to_string()];
    lines.extend(details.iter().map(|detail| indent_detail(detail)));
    lines.join("\n")
}

fn indent_detail(detail: &str) -> String {
    let mut lines = detail.lines();
    let Some(first) = lines.next() else {
        return "  ".to_string();
    };

    let mut formatted = format!("  {first}");
    for line in lines {
        formatted.push('\n');
        formatted.push_str("    ");
        formatted.push_str(line);
    }
    formatted
}

fn read_startup_log(path: &std::path::Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn shell_message_to_events(message: WireMessage) -> Vec<KernelEvent> {
    match message.header.msg_type.as_str() {
        "kernel_info_reply" => {
            let banner = message
                .content
                .get("banner")
                .and_then(Value::as_str)
                .unwrap_or("connected");
            vec![KernelEvent::Info(banner.to_string())]
        }
        "execute_reply" => Vec::new(),
        "shutdown_reply" => vec![KernelEvent::Status(KernelStatus::Disconnected)],
        _ => Vec::new(),
    }
}

fn iopub_message_to_events(message: WireMessage) -> Vec<KernelEvent> {
    match message.header.msg_type.as_str() {
        "status" => {
            let status = match message
                .content
                .get("execution_state")
                .and_then(Value::as_str)
            {
                Some("busy") => KernelStatus::Busy,
                Some("idle") => KernelStatus::Idle,
                Some("starting") => KernelStatus::Connecting,
                _ => KernelStatus::Connecting,
            };
            vec![KernelEvent::Status(status)]
        }
        "execute_input" => vec![KernelEvent::ExecuteInput {
            execution_count: message
                .content
                .get("execution_count")
                .and_then(Value::as_u64)
                .and_then(|count| u32::try_from(count).ok()),
            code: message
                .content
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }],
        "stream" => vec![KernelEvent::Stream {
            name: message
                .content
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("stdout")
                .to_string(),
            text: message
                .content
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }],
        "execute_result" | "display_data" | "update_display_data" => {
            if let Some(text) = pick_text_payload(&message.content) {
                vec![KernelEvent::ExecuteResult {
                    execution_count: message
                        .content
                        .get("execution_count")
                        .and_then(Value::as_u64)
                        .and_then(|count| u32::try_from(count).ok()),
                    text,
                }]
            } else {
                Vec::new()
            }
        }
        "error" => vec![KernelEvent::Error {
            traceback: message
                .content
                .get("traceback")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        }],
        "input_request" => vec![KernelEvent::InputRequest {
            prompt: message
                .content
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            password: message
                .content
                .get("password")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }],
        _ => Vec::new(),
    }
}

fn stdin_message_to_events(message: WireMessage) -> Vec<KernelEvent> {
    match message.header.msg_type.as_str() {
        "input_request" => vec![KernelEvent::InputRequest {
            prompt: message
                .content
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            password: message
                .content
                .get("password")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }],
        _ => Vec::new(),
    }
}

fn pick_text_payload(content: &Value) -> Option<String> {
    let data = content.get("data")?;
    for key in ["text/plain", "text/markdown"] {
        if let Some(text) = data.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

fn send_sigint(pid: Option<u32>) -> Result<()> {
    let pid = pid.ok_or_else(|| anyhow!("kernel process has no pid"))?;

    #[cfg(unix)]
    unsafe {
        if libc::kill(pid as i32, libc::SIGINT) == 0 {
            Ok(())
        } else {
            Err(anyhow!("failed to send SIGINT to kernel process"))
        }
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        bail!("interrupt is only implemented on unix-like systems")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use tempfile::{NamedTempFile, TempDir};

    use super::{LaunchConfig, LocalKernel, append_startup_context, pick_text_payload};

    #[test]
    fn prefers_plain_text_payloads() {
        let content = json!({
            "data": {
                "text/plain": "hello",
                "text/html": "<b>hello</b>"
            }
        });

        assert_eq!(pick_text_payload(&content).as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn appends_startup_log_output_to_errors() {
        let startup_log = NamedTempFile::new().expect("startup log");
        std::fs::write(startup_log.path(), "No module named ipykernel").expect("write log");

        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("spawn");

        let local = LocalKernel {
            child,
            _connection_dir: TempDir::new().expect("connection dir"),
            connection_file: PathBuf::from("/tmp/kernel-connection.json"),
            startup_log,
            launch: LaunchConfig {
                python: "python3".to_string(),
                kernel_args: Vec::new(),
            },
        };

        let mut details = Vec::new();
        append_startup_context(&mut details, &local, None);
        let message = details.join("\n");

        assert!(message.contains("No module named ipykernel"));
    }
}
