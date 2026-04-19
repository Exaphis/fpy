use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryOutcome {
    Ok,
    Error,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub session_id: Uuid,
    pub entry_seq: u64,
    pub ts_unix_ns: u64,
    pub host: String,
    pub pid: u32,
    pub code: String,
    pub duration_ns: Option<u64>,
    pub outcome: Option<HistoryOutcome>,
}

pub struct HistorySession {
    path: PathBuf,
    host: String,
    session_id: Uuid,
    pid: u32,
    next_entry_seq: u64,
    writer: BufWriter<File>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum HistoryRecord {
    #[serde(rename = "cell")]
    Cell {
        v: u8,
        session_id: Uuid,
        entry_seq: u64,
        ts_unix_ns: u64,
        host: String,
        pid: u32,
        code: String,
    },
    #[serde(rename = "cell_done")]
    CellDone {
        v: u8,
        session_id: Uuid,
        entry_seq: u64,
        ts_unix_ns: u64,
        duration_ns: u64,
        outcome: HistoryOutcome,
    },
}

impl HistorySession {
    pub fn open(root: &Path) -> Result<Self> {
        let host = current_host_name();
        let pid = std::process::id();
        Self::open_with(root, host, pid)
    }

    fn open_with(root: &Path, host: String, pid: u32) -> Result<Self> {
        let host_dir = root.join(&host);
        fs::create_dir_all(&host_dir)
            .with_context(|| format!("failed to create history directory {}", host_dir.display()))?;

        for _ in 0..16 {
            let session_id = Uuid::now_v7();
            let path = host_dir.join(format!("{session_id}-{pid}.jsonl"));
            match OpenOptions::new().create_new(true).append(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        host,
                        session_id,
                        pid,
                        next_entry_seq: 1,
                        writer: BufWriter::new(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("failed to open history file {}", path.display()));
                }
            }
        }

        Err(anyhow!("failed to allocate a unique history session file"))
    }

    pub fn append_cell(&mut self, code: &str) -> Result<u64> {
        let entry_seq = self.next_entry_seq;
        self.next_entry_seq += 1;
        self.append_record(&HistoryRecord::Cell {
            v: 1,
            session_id: self.session_id,
            entry_seq,
            ts_unix_ns: unix_timestamp_ns()?,
            host: self.host.clone(),
            pid: self.pid,
            code: code.to_string(),
        })?;
        Ok(entry_seq)
    }

    pub fn append_done(
        &mut self,
        entry_seq: u64,
        duration: Duration,
        outcome: HistoryOutcome,
    ) -> Result<()> {
        self.append_record(&HistoryRecord::CellDone {
            v: 1,
            session_id: self.session_id,
            entry_seq,
            ts_unix_ns: unix_timestamp_ns()?,
            duration_ns: duration_ns_u64(duration),
            outcome,
        })
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append_record(&mut self, record: &HistoryRecord) -> Result<()> {
        serde_json::to_writer(&mut self.writer, record)
            .with_context(|| format!("failed to serialize history record to {}", self.path.display()))?;
        self.writer
            .write_all(b"\n")
            .with_context(|| format!("failed to append history newline to {}", self.path.display()))?;
        self.writer
            .flush()
            .with_context(|| format!("failed to flush history file {}", self.path.display()))?;
        Ok(())
    }
}

pub fn default_root_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("FPY_HISTORY_DIR") {
        return Ok(PathBuf::from(path));
    }

    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path).join("fpy/history"));
    }

    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".local/share/fpy/history"))
}

pub fn load_entries(root: &Path) -> Result<Vec<HistoryEntry>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut cells = Vec::new();
    let mut cell_indexes = HashMap::<(Uuid, u64), usize>::new();
    let mut pending_done = HashMap::<(Uuid, u64), (u64, HistoryOutcome)>::new();

    let mut host_dirs = fs::read_dir(root)
        .with_context(|| format!("failed to read history root {}", root.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("failed to enumerate history root {}", root.display()))?;
    host_dirs.sort_by_key(|entry| entry.path());

    for host_dir in host_dirs {
        let file_type = host_dir.file_type().with_context(|| {
            format!("failed to read file type for {}", host_dir.path().display())
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let mut files = fs::read_dir(host_dir.path())
            .with_context(|| format!("failed to read history host dir {}", host_dir.path().display()))?
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("failed to enumerate history host dir {}", host_dir.path().display()))?;
        files.sort_by_key(|entry| entry.path());

        for file in files {
            let path = file.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }

            for record in read_records(&path)? {
                match record {
                    HistoryRecord::Cell {
                        session_id,
                        entry_seq,
                        ts_unix_ns,
                        host,
                        pid,
                        code,
                        ..
                    } => {
                        let key = (session_id, entry_seq);
                        let mut entry = HistoryEntry {
                            session_id,
                            entry_seq,
                            ts_unix_ns,
                            host,
                            pid,
                            code,
                            duration_ns: None,
                            outcome: None,
                        };
                        if let Some((duration_ns, outcome)) = pending_done.remove(&key) {
                            entry.duration_ns = Some(duration_ns);
                            entry.outcome = Some(outcome);
                        }
                        cell_indexes.insert(key, cells.len());
                        cells.push(entry);
                    }
                    HistoryRecord::CellDone {
                        session_id,
                        entry_seq,
                        duration_ns,
                        outcome,
                        ..
                    } => {
                        let key = (session_id, entry_seq);
                        if let Some(index) = cell_indexes.get(&key).copied() {
                            cells[index].duration_ns = Some(duration_ns);
                            cells[index].outcome = Some(outcome);
                        } else {
                            pending_done.insert(key, (duration_ns, outcome));
                        }
                    }
                }
            }
        }
    }

    cells.sort_by(|left, right| {
        left.ts_unix_ns
            .cmp(&right.ts_unix_ns)
            .then_with(|| left.host.cmp(&right.host))
            .then_with(|| left.session_id.cmp(&right.session_id))
            .then_with(|| left.entry_seq.cmp(&right.entry_seq))
    });
    Ok(cells)
}

fn read_records(path: &Path) -> Result<Vec<HistoryRecord>> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read history file {}", path.display()))?;
    let mut records = Vec::new();

    for chunk in contents.split_inclusive('\n') {
        let Some(line) = chunk.strip_suffix('\n') else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<HistoryRecord>(trimmed) {
            records.push(record);
        }
    }

    Ok(records)
}

fn unix_timestamp_ns() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| anyhow!("system time before unix epoch"))?
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64)
}

fn duration_ns_u64(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn current_host_name() -> String {
    if let Some(name) = std::env::var_os("HOSTNAME")
        && !name.is_empty()
    {
        return sanitize_host_name(name.to_string_lossy().as_ref());
    }

    let mut buffer = [0u8; 256];
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result == 0 {
        let length = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
        if length > 0 {
            return sanitize_host_name(&String::from_utf8_lossy(&buffer[..length]));
        }
    }

    "unknown-host".to_string()
}

fn sanitize_host_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown-host".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{
        HistoryOutcome, HistorySession, load_entries, read_records, sanitize_host_name,
    };

    #[test]
    fn writes_session_history_under_host_directory() {
        let root = TempDir::new().expect("history root");
        let mut session = HistorySession::open_with(root.path(), "test-host".to_string(), 42)
            .expect("open history session");

        let entry_seq = session.append_cell("1+1").expect("append cell");
        session
            .append_done(entry_seq, Duration::from_millis(12), HistoryOutcome::Ok)
            .expect("append done");

        let relative = session
            .path()
            .strip_prefix(root.path())
            .expect("session file under root")
            .to_path_buf();
        assert_eq!(relative.parent().unwrap(), Path::new("test-host"));
        assert!(relative
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-42.jsonl")));

        let records = read_records(session.path()).expect("read session records");
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn loads_cells_and_merges_runtime_metadata() {
        let root = TempDir::new().expect("history root");
        let host_dir = root.path().join("host-a");
        std::fs::create_dir_all(&host_dir).expect("host dir");
        let session_id = Uuid::now_v7();
        let path = host_dir.join(format!("{session_id}-7.jsonl"));
        std::fs::write(
            &path,
            format!(
                concat!(
                    "{{\"v\":1,\"type\":\"cell\",\"session_id\":\"{}\",\"entry_seq\":1,\"ts_unix_ns\":10,\"host\":\"host-a\",\"pid\":7,\"code\":\"1+1\"}}\n",
                    "{{\"v\":1,\"type\":\"cell_done\",\"session_id\":\"{}\",\"entry_seq\":1,\"ts_unix_ns\":20,\"duration_ns\":3000,\"outcome\":\"ok\"}}\n",
                    "{{\"v\":1,\"type\":\"cell\",\"session_id\":\"{}\",\"entry_seq\":2,\"ts_unix_ns\":30,\"host\":\"host-a\",\"pid\":7,\"code\":\"2+2\"}}\n"
                ),
                session_id,
                session_id,
                session_id,
            ),
        )
        .expect("write history");

        let entries = load_entries(root.path()).expect("load entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].code, "1+1");
        assert_eq!(entries[0].duration_ns, Some(3000));
        assert_eq!(entries[0].outcome, Some(HistoryOutcome::Ok));
        assert_eq!(entries[1].code, "2+2");
        assert_eq!(entries[1].duration_ns, None);
    }

    #[test]
    fn ignores_truncated_trailing_records() {
        let root = TempDir::new().expect("history root");
        let host_dir = root.path().join("host-a");
        std::fs::create_dir_all(&host_dir).expect("host dir");
        let session_id = Uuid::now_v7();
        let path = host_dir.join(format!("{session_id}-7.jsonl"));
        std::fs::write(
            &path,
            format!(
                concat!(
                    "{{\"v\":1,\"type\":\"cell\",\"session_id\":\"{}\",\"entry_seq\":1,\"ts_unix_ns\":10,\"host\":\"host-a\",\"pid\":7,\"code\":\"1+1\"}}\n",
                    "{{\"v\":1,\"type\":\"cell"
                ),
                session_id,
            ),
        )
        .expect("write history");

        let entries = load_entries(root.path()).expect("load entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, "1+1");
    }

    #[test]
    fn sanitizes_host_names_for_directory_use() {
        assert_eq!(sanitize_host_name("host/name"), "host_name");
        assert_eq!(sanitize_host_name(""), "unknown-host");
    }
}
