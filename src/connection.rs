use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Shell,
    Iopub,
    Stdin,
    Control,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionFile {
    pub ip: String,
    pub transport: String,
    pub shell_port: u16,
    pub iopub_port: u16,
    pub stdin_port: u16,
    pub control_port: u16,
    pub hb_port: u16,
    #[serde(default)]
    pub key: String,
    #[serde(default = "default_signature_scheme")]
    pub signature_scheme: String,
    #[serde(default)]
    pub kernel_name: String,
}

fn default_signature_scheme() -> String {
    "hmac-sha256".to_string()
}

impl ConnectionFile {
    pub fn read(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read connection file {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("failed to parse connection file {}", path.display()))
    }

    pub fn endpoint(&self, channel: Channel) -> String {
        let port = match channel {
            Channel::Shell => self.shell_port,
            Channel::Iopub => self.iopub_port,
            Channel::Stdin => self.stdin_port,
            Channel::Control => self.control_port,
            Channel::Heartbeat => self.hb_port,
        };

        format!("{}://{}:{port}", self.transport, self.ip)
    }

    pub fn summary(&self) -> String {
        format!(
            concat!(
                "transport={transport} ip={ip} ",
                "shell={shell} iopub={iopub} stdin={stdin} control={control} hb={hb}"
            ),
            transport = self.transport,
            ip = self.ip,
            shell = self.shell_port,
            iopub = self.iopub_port,
            stdin = self.stdin_port,
            control = self.control_port,
            hb = self.hb_port,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Channel, ConnectionFile};

    #[test]
    fn builds_channel_endpoints() {
        let connection = ConnectionFile {
            ip: "127.0.0.1".into(),
            transport: "tcp".into(),
            shell_port: 1,
            iopub_port: 2,
            stdin_port: 3,
            control_port: 4,
            hb_port: 5,
            key: "secret".into(),
            signature_scheme: "hmac-sha256".into(),
            kernel_name: String::new(),
        };

        assert_eq!(connection.endpoint(Channel::Shell), "tcp://127.0.0.1:1");
        assert_eq!(connection.endpoint(Channel::Heartbeat), "tcp://127.0.0.1:5");
    }
}
