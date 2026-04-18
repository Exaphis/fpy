use serde_json::Value;

use crate::{
    jupyter::WireMessage,
    kernel::{KernelEvent, KernelStatus},
};

pub(super) fn shell_message_to_events(message: WireMessage) -> Vec<KernelEvent> {
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

pub(super) fn iopub_message_to_events(message: WireMessage) -> Vec<KernelEvent> {
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
        _ => Vec::new(),
    }
}

pub(super) fn stdin_message_to_events(message: WireMessage) -> Vec<KernelEvent> {
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

pub(super) fn pick_text_payload(content: &Value) -> Option<String> {
    let data = content.get("data")?;
    for key in ["text/plain", "text/markdown"] {
        if let Some(text) = data.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde_json::{Value, json};

    use super::{iopub_message_to_events, shell_message_to_events, stdin_message_to_events};
    use crate::{
        jupyter::{Header, WireMessage},
        kernel::{KernelEvent, KernelStatus},
    };

    fn wire_message(msg_type: &str, content: Value) -> WireMessage {
        WireMessage {
            ids: vec![Bytes::from_static(b"id")],
            header: Header {
                msg_id: "msg-1".to_string(),
                username: "user".to_string(),
                session: "session".to_string(),
                date: "2024-01-01T00:00:00Z".to_string(),
                msg_type: msg_type.to_string(),
                version: "5.3".to_string(),
            },
            parent_header: Value::Null,
            metadata: json!({}),
            content,
            buffers: Vec::new(),
        }
    }

    #[test]
    fn maps_status_messages_to_kernel_status() {
        let events = iopub_message_to_events(wire_message(
            "status",
            json!({ "execution_state": "busy" }),
        ));
        assert!(matches!(events.as_slice(), [KernelEvent::Status(KernelStatus::Busy)]));

        let events = iopub_message_to_events(wire_message(
            "status",
            json!({ "execution_state": "idle" }),
        ));
        assert!(matches!(events.as_slice(), [KernelEvent::Status(KernelStatus::Idle)]));
    }

    #[test]
    fn maps_markdown_display_payloads_to_execute_results() {
        let events = iopub_message_to_events(wire_message(
            "display_data",
            json!({
                "data": {"text/markdown": "**hi**"},
                "execution_count": 7
            }),
        ));

        match events.as_slice() {
            [KernelEvent::ExecuteResult {
                execution_count,
                text,
            }] => {
                assert_eq!(*execution_count, Some(7));
                assert_eq!(text, "**hi**");
            }
            _ => panic!("unexpected events: {events:?}"),
        }
    }

    #[test]
    fn maps_stdin_requests_to_input_events() {
        let events = stdin_message_to_events(wire_message(
            "input_request",
            json!({"prompt": "Name: ", "password": true}),
        ));

        match events.as_slice() {
            [KernelEvent::InputRequest { prompt, password }] => {
                assert_eq!(prompt, "Name: ");
                assert!(*password);
            }
            _ => panic!("unexpected events: {events:?}"),
        }
    }

    #[test]
    fn maps_shutdown_reply_to_disconnected_status() {
        let events = shell_message_to_events(wire_message("shutdown_reply", json!({})));
        assert!(matches!(
            events.as_slice(),
            [KernelEvent::Status(KernelStatus::Disconnected)]
        ));
    }
}
