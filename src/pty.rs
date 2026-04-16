//! Pty wrapper. Each session owns one long-lived child process.
//!
//! Architecture:
//!   - `portable_pty` spawns a child under a master pty.
//!   - A dedicated blocking thread reads bytes from the pty master and pushes
//!     them through a `LineBuffer`, emitting complete lines into an mpsc channel.
//!   - A second blocking thread receives byte chunks on an mpsc channel and
//!     writes them to the pty master (stdin).
//!   - The async world interacts solely via those channels + a kill flag.

use crate::error::{Error, Result};
use crate::signals::LineBuffer;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct PtyOpts {
    pub cwd: std::path::PathBuf,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub struct PtyHandle {
    pub pid: u32,
    stdout_rx: mpsc::Receiver<String>,
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_flag: Arc<AtomicBool>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    pub fn spawn(opts: PtyOpts) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Pty(format!("openpty: {e}")))?;

        let mut cmd = CommandBuilder::new(&opts.command);
        for a in &opts.args {
            cmd.arg(a);
        }
        cmd.cwd(&opts.cwd);
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::Pty(format!("spawn: {e}")))?;
        let pid = child.process_id().unwrap_or(0);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Pty(format!("clone_reader: {e}")))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Pty(format!("take_writer: {e}")))?;

        let kill_flag = Arc::new(AtomicBool::new(false));

        // Reader thread
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(256);
        let kill_reader = kill_flag.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut line_buf = LineBuffer::new();
            while !kill_reader.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        let lines = line_buf.feed(&chunk);
                        for line in lines {
                            if stdout_tx.blocking_send(line).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        // Writer thread
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
        std::thread::spawn(move || {
            while let Some(bytes) = stdin_rx.blocking_recv() {
                if writer.write_all(&bytes).is_err() {
                    return;
                }
                let _ = writer.flush();
            }
        });

        Ok(Self {
            pid,
            stdout_rx,
            stdin_tx,
            kill_flag,
            child,
        })
    }

    /// Next complete line of stdout, or `None` if the stream has ended.
    pub async fn next_line(&mut self) -> Option<String> {
        self.stdout_rx.recv().await
    }

    /// Write bytes to the child's stdin (via pty master).
    pub async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.stdin_tx
            .send(bytes.to_vec())
            .await
            .map_err(|_| Error::Pty("stdin channel closed".into()))
    }

    pub async fn write_str(&self, s: &str) -> Result<()> {
        self.write(s.as_bytes()).await
    }

    pub async fn kill(&mut self) -> Result<()> {
        self.kill_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        self.kill_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
    }
}
