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
