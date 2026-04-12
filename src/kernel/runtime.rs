use anyhow::Result;
use serde_json::json;
use tokio::{sync::mpsc, task::JoinHandle};
use zeromq::{DealerSocket, Socket, SocketRecv, SocketSend, SubSocket};

use crate::{
    connection::{Channel, ConnectionFile},
    jupyter::{MessageCodec, WireMessage},
    kernel::{
        KernelCommand, KernelEvent, KernelStatus, Runtime,
        messages::{iopub_message_to_events, shell_message_to_events, stdin_message_to_events},
    },
};

impl Runtime {
    pub(super) async fn connect(
        connection: ConnectionFile,
        event_tx: mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Self> {
        let codec = MessageCodec::new(connection.key.clone());

        let mut shell = DealerSocket::new();
        shell.connect(&connection.endpoint(Channel::Shell)).await?;
        let (mut shell_send, shell_recv) = shell.split();

        let mut stdin_socket = DealerSocket::new();
        stdin_socket
            .connect(&connection.endpoint(Channel::Stdin))
            .await?;
        let (mut stdin_send, stdin_recv) = stdin_socket.split();

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

        let shell_task = spawn_recv_loop(
            shell_recv,
            codec.clone(),
            event_tx.clone(),
            shell_message_to_events,
        );
        let stdin_task = spawn_recv_loop(
            stdin_recv,
            codec.clone(),
            event_tx.clone(),
            stdin_message_to_events,
        );
        let iopub_task = spawn_recv_loop(iopub, codec, event_tx.clone(), iopub_message_to_events);

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

fn spawn_recv_loop<R, F>(
    mut receiver: R,
    codec: MessageCodec,
    event_tx: mpsc::UnboundedSender<KernelEvent>,
    map_message: F,
) -> JoinHandle<()>
where
    R: SocketRecv + Send + 'static,
    F: Fn(WireMessage) -> Vec<KernelEvent> + Send + Copy + 'static,
{
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(message) => match codec.decode(message) {
                    Ok(decoded) => {
                        for event in map_message(decoded) {
                            let _ = event_tx.send(event);
                        }
                    }
                    Err(error) => {
                        let _ = event_tx.send(KernelEvent::Warning(error.to_string()));
                    }
                },
                Err(error) => {
                    let _ = event_tx.send(KernelEvent::Fatal(error.to_string()));
                    break;
                }
            }
        }
    })
}

async fn send_message(
    socket: &mut impl SocketSend,
    codec: &MessageCodec,
    message: &WireMessage,
) -> Result<()> {
    socket.send(codec.encode_zmq(message)?).await?;
    Ok(())
}
