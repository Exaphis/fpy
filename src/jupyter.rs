use anyhow::{Result, anyhow, bail, ensure};
use bytes::Bytes;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;
use zeromq::ZmqMessage;

const DELIMITER: &[u8] = b"<IDS|MSG>";
const PROTOCOL_VERSION: &str = "5.3";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub msg_id: String,
    pub username: String,
    pub session: String,
    pub date: String,
    pub msg_type: String,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct WireMessage {
    pub ids: Vec<Bytes>,
    pub header: Header,
    pub parent_header: Value,
    pub metadata: Value,
    pub content: Value,
    pub buffers: Vec<Bytes>,
}

#[derive(Debug, Clone)]
pub struct MessageCodec {
    key: Vec<u8>,
    username: String,
    session_id: String,
}

impl MessageCodec {
    pub fn new(key: String) -> Self {
        Self {
            key: key.into_bytes(),
            username: std::env::var("USER").unwrap_or_else(|_| "fpy".to_string()),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn message(
        &self,
        msg_type: &str,
        parent_header: Option<&Header>,
        content: Value,
    ) -> WireMessage {
        let header = Header {
            msg_id: Uuid::new_v4().to_string(),
            username: self.username.clone(),
            session: self.session_id.clone(),
            date: Utc::now().to_rfc3339(),
            msg_type: msg_type.to_string(),
            version: PROTOCOL_VERSION.to_string(),
        };

        WireMessage {
            ids: Vec::new(),
            header,
            parent_header: parent_header
                .map(|header| serde_json::to_value(header).unwrap_or(Value::Null))
                .unwrap_or(Value::Null),
            metadata: json!({}),
            content,
            buffers: Vec::new(),
        }
    }

    pub fn encode_zmq(&self, message: &WireMessage) -> Result<ZmqMessage> {
        let header = serde_json::to_vec(&message.header)?;
        let parent_header = serde_json::to_vec(&message.parent_header)?;
        let metadata = serde_json::to_vec(&message.metadata)?;
        let content = serde_json::to_vec(&message.content)?;
        let signature = self.sign(&[&header, &parent_header, &metadata, &content])?;

        let mut frames = Vec::with_capacity(message.ids.len() + 5 + message.buffers.len());
        frames.extend(message.ids.iter().cloned());
        frames.push(Bytes::from_static(DELIMITER));
        frames.push(Bytes::from(signature));
        frames.push(Bytes::from(header));
        frames.push(Bytes::from(parent_header));
        frames.push(Bytes::from(metadata));
        frames.push(Bytes::from(content));
        frames.extend(message.buffers.iter().cloned());
        ZmqMessage::try_from(frames)
            .map_err(|_| anyhow!("failed to assemble Jupyter message frames"))
    }

    pub fn decode(&self, message: ZmqMessage) -> Result<WireMessage> {
        let frames = message.into_vec();
        let delimiter = frames
            .iter()
            .position(|frame| frame.as_ref() == DELIMITER)
            .ok_or_else(|| anyhow!("invalid Jupyter frame: missing delimiter"))?;

        ensure!(
            frames.len() >= delimiter + 5,
            "invalid Jupyter frame: expected header frames"
        );

        let ids = frames[..delimiter].to_vec();
        let signature = &frames[delimiter + 1];
        let header_frame = &frames[delimiter + 2];
        let parent_frame = &frames[delimiter + 3];
        let metadata_frame = &frames[delimiter + 4];
        let content_frame = &frames[delimiter + 5];
        let buffers = frames[(delimiter + 6)..].to_vec();

        self.verify(
            signature,
            &[
                header_frame.as_ref(),
                parent_frame.as_ref(),
                metadata_frame.as_ref(),
                content_frame.as_ref(),
            ],
        )?;

        Ok(WireMessage {
            ids,
            header: serde_json::from_slice(header_frame)?,
            parent_header: serde_json::from_slice(parent_frame)?,
            metadata: serde_json::from_slice(metadata_frame)?,
            content: serde_json::from_slice(content_frame)?,
            buffers,
        })
    }

    fn sign(&self, parts: &[&[u8]]) -> Result<String> {
        if self.key.is_empty() {
            return Ok(String::new());
        }

        let mut mac = HmacSha256::new_from_slice(&self.key)?;
        for part in parts {
            mac.update(part);
        }

        let signature = mac.finalize().into_bytes();
        Ok(signature.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    fn verify(&self, signature: &Bytes, parts: &[&[u8]]) -> Result<()> {
        if self.key.is_empty() {
            return Ok(());
        }

        let expected = self.sign(parts)?;
        let actual = std::str::from_utf8(signature.as_ref())?;
        if expected == actual {
            Ok(())
        } else {
            bail!("invalid Jupyter message signature");
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde_json::json;
    use zeromq::ZmqMessage;

    use super::MessageCodec;

    #[test]
    fn round_trips_jupyter_frames_with_ids_and_buffers() {
        let codec = MessageCodec::new("secret".into());
        let mut message = codec.message("execute_request", None, json!({ "code": "1 + 1" }));
        message.ids = vec![Bytes::from_static(b"client-a"), Bytes::from_static(b"client-b")];
        message.buffers = vec![Bytes::from_static(b"buffer")];
        let zmq = codec.encode_zmq(&message).expect("encode");
        let decoded = codec.decode(zmq).expect("decode");

        assert_eq!(decoded.ids, message.ids);
        assert_eq!(decoded.buffers, message.buffers);
        assert_eq!(decoded.header.msg_type, "execute_request");
        assert_eq!(decoded.content["code"], "1 + 1");
    }

    #[test]
    fn rejects_messages_with_invalid_signatures() {
        let codec = MessageCodec::new("secret".into());
        let message = codec.message("execute_request", None, json!({ "code": "1 + 1" }));
        let zmq = codec.encode_zmq(&message).expect("encode");
        let mut frames = zmq.into_vec();
        frames[5] = Bytes::from_static(br#"{"code":"1 + 2"}"#);
        let tampered = ZmqMessage::try_from(frames).expect("rebuild tampered message");

        let error = codec.decode(tampered).expect_err("tampered signature should fail");
        assert!(error.to_string().contains("invalid Jupyter message signature"));
    }
}
