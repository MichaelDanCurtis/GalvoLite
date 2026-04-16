//! Per-session control socket.
//!
//! `start-session` listens on `<data-dir>/sockets/<session-id>.sock`.
//! Other CLI commands (`send`, `handoff`, `kill`) connect and send a single
//! JSON-line command.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlMessage {
    Send { text: String, newline: bool },
    Handoff,
    Kill,
    Status,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlResponse {
    Ok(String),
    Err(String),
}

pub fn socket_path(data_dir: &std::path::Path, session_id: uuid::Uuid) -> PathBuf {
    data_dir.join("sockets").join(format!("{session_id}.sock"))
}

pub async fn listen(
    path: &std::path::Path,
) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::error::Error::io(parent, e))?;
    }
    // macOS sun_path is 104 bytes; Linux is 108. Check before bind.
    let path_len = path.as_os_str().len();
    if path_len > 100 {
        return Err(crate::error::Error::Config(format!(
            "socket path too long ({path_len} bytes, max ~100): {}. \
             Use --data-dir with a shorter path.",
            path.display()
        )));
    }
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path).map_err(|e| crate::error::Error::io(path, e))
}

pub async fn send_command(
    sock_path: &std::path::Path,
    msg: ControlMessage,
) -> Result<ControlResponse> {
    let stream = UnixStream::connect(sock_path)
        .await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    let (reader, mut writer) = stream.into_split();
    let line = serde_json::to_string(&msg)
        .map_err(|e| crate::error::Error::Config(format!("serialize: {e}")))?;
    writer.write_all(line.as_bytes()).await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    writer.write_all(b"\n").await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    writer.shutdown().await.ok();

    let mut br = BufReader::new(reader);
    let mut buf = String::new();
    br.read_line(&mut buf).await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    let resp: ControlResponse = serde_json::from_str(buf.trim())
        .map_err(|e| crate::error::Error::Config(format!("deserialize: {e}")))?;
    Ok(resp)
}
