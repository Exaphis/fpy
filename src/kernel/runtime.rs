use anyhow::Result;
use serde_json::json;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use zeromq::{
    DealerSocket, Socket, SocketOptions, SocketRecv, SocketSend, SubSocket, util::PeerIdentity,
};

use crate::{
    connection::{Channel, ConnectionFile},
    jupyter::{Header, MessageCodec, WireMessage},
    kernel::{
        KernelCommand, KernelEvent, KernelStatus, Runtime,
        messages::{iopub_message_to_events, shell_message_to_events, stdin_message_to_events},
    },
};
use std::sync::Arc;

#[derive(Clone)]
struct PendingInputRequest {
    ids: Vec<bytes::Bytes>,
    header: Header,
}

impl Runtime {
    pub(super) async fn connect(
        connection: ConnectionFile,
        event_tx: mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Self> {
        let codec = MessageCodec::new(connection.key.clone());
        let frontend_identity = PeerIdentity::new();
        let mut shell_options = SocketOptions::default();
        shell_options.peer_identity(frontend_identity.clone());
        let mut stdin_options = SocketOptions::default();
        stdin_options.peer_identity(frontend_identity.clone());
        let mut control_options = SocketOptions::default();
        control_options.peer_identity(frontend_identity);
        let pending_input_request = Arc::new(Mutex::new(None::<PendingInputRequest>));

        let mut shell = DealerSocket::with_options(shell_options);
        shell.connect(&connection.endpoint(Channel::Shell)).await?;
        let (mut shell_send, shell_recv) = shell.split();

        let mut stdin_socket = DealerSocket::with_options(stdin_options);
        stdin_socket
            .connect(&connection.endpoint(Channel::Stdin))
            .await?;
        let (mut stdin_send, stdin_recv) = stdin_socket.split();

        let mut control = DealerSocket::with_options(control_options);
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
        let command_pending_input = pending_input_request.clone();
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
                        let pending_request = command_pending_input.lock().await.take();
                        let Some(pending_request) = pending_request else {
                            let _ = command_events.send(KernelEvent::Warning(
                                "received stdin reply without a pending input request".to_string(),
                            ));
                            continue;
                        };
                        let mut message = command_codec.message(
                            "input_reply",
                            Some(&pending_request.header),
                            json!({ "value": value }),
                        );
                        message.ids = pending_request.ids;
                        send_message(&mut stdin_send, &command_codec, &message).await
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
        let stdin_task = spawn_stdin_recv_loop(
            stdin_recv,
            codec.clone(),
            event_tx.clone(),
            pending_input_request,
        );
        let iopub_task = spawn_recv_loop(iopub, codec, event_tx.clone(), iopub_message_to_events);

        let _ = event_tx.send(KernelEvent::Connected(connection.summary()));
        let _ = event_tx.send(KernelEvent::Status(KernelStatus::Idle));

        Ok(Self {
            connection,
            command_tx,
            tasks: vec![command_task, shell_task, stdin_task, iopub_task],
        })
    }
}

fn spawn_stdin_recv_loop<R>(
    mut receiver: R,
    codec: MessageCodec,
    event_tx: mpsc::UnboundedSender<KernelEvent>,
    pending_input_request: Arc<Mutex<Option<PendingInputRequest>>>,
) -> JoinHandle<()>
where
    R: SocketRecv + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(message) => match codec.decode(message) {
                    Ok(decoded) => {
                        if decoded.header.msg_type == "input_request" {
                            let mut pending = pending_input_request.lock().await;
                            *pending = Some(PendingInputRequest {
                                ids: decoded.ids.clone(),
                                header: decoded.header.clone(),
                            });
                        }
                        for event in stdin_message_to_events(decoded) {
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
