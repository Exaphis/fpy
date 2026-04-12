mod diagnostics;
mod messages;
mod runtime;

use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use crate::connection::ConnectionFile;
use anyhow::{Context, Result, anyhow, bail};
use diagnostics::{local_exit_message, startup_failure_message, startup_timeout_message};
use tempfile::{NamedTempFile, TempDir};
use tokio::{
    process::{Child, Command},
    sync::mpsc,
    task::JoinHandle,
    time::sleep,
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
    stderr_log: NamedTempFile,
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
            Some(status) => Ok(Some(local_exit_message(status, local))),
            None => Ok(None),
        }
    }

    async fn stop_runtime(&mut self) {
        for task in self.runtime.tasks.drain(..) {
            task.abort();
        }
    }

    async fn shutdown_local_child(&mut self) -> Result<()> {
        if let Some(mut local) = self.local.take()
            && local.child.try_wait()?.is_none()
        {
            let _ = local.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), local.child.wait()).await;
        }
        Ok(())
    }
}

async fn spawn_kernel(launch: &LaunchConfig) -> Result<LocalKernel> {
    let connection_dir = tempfile::tempdir().context("failed to create temp connection dir")?;
    let stderr_log = NamedTempFile::new().context("failed to create kernel startup log")?;
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
            stderr_log
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
        stderr_log,
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

    use super::{LaunchConfig, LocalKernel, local_exit_message};
    use crate::kernel::{diagnostics::append_startup_context, messages::pick_text_payload};

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
        let stderr_log = NamedTempFile::new().expect("startup log");
        std::fs::write(stderr_log.path(), "No module named ipykernel").expect("write log");

        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("spawn");

        let local = LocalKernel {
            child,
            _connection_dir: TempDir::new().expect("connection dir"),
            connection_file: PathBuf::from("/tmp/kernel-connection.json"),
            stderr_log,
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

    #[tokio::test]
    async fn includes_stderr_in_local_exit_message() {
        let stderr_log = NamedTempFile::new().expect("startup log");
        std::fs::write(stderr_log.path(), "Segmentation fault: 11").expect("write log");

        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("exit 139")
            .spawn()
            .expect("spawn");
        let status = child.wait().await.expect("wait");

        let local = LocalKernel {
            child,
            _connection_dir: TempDir::new().expect("connection dir"),
            connection_file: PathBuf::from("/tmp/kernel-connection.json"),
            stderr_log,
            launch: LaunchConfig {
                python: "python3".to_string(),
                kernel_args: Vec::new(),
            },
        };

        let message = local_exit_message(status, &local);
        assert!(message.contains("Kernel exited unexpectedly"));
        assert!(message.contains("Status:"));
        assert!(message.contains("Segmentation fault: 11"));
    }
}
