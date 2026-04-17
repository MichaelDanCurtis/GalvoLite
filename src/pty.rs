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
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{broadcast, mpsc};

#[derive(Debug, Clone)]
pub struct PtyOpts {
    pub cwd: std::path::PathBuf,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Shared handle to the pty master, used for resize from the async world.
/// Wrapped in a blocking mutex because `MasterPty` is not `Sync` in general.
pub(crate) type SharedMaster = Arc<StdMutex<Box<dyn MasterPty + Send>>>;

pub struct PtyHandle {
    pub pid: u32,
    stdout_rx: mpsc::Receiver<String>,
    pub(crate) raw_output_tx: broadcast::Sender<Vec<u8>>,
    pub(crate) stdin_tx: mpsc::Sender<Vec<u8>>,
    pub(crate) kill_flag: Arc<AtomicBool>,
    pub(crate) master: SharedMaster,
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

        // Reader thread. Splits a single byte stream into two consumers:
        //   * `raw_output_tx` (broadcast): every byte verbatim, for the
        //     terminal WebSocket — preserves ANSI/cursor sequences that
        //     line-based reassembly would destroy.
        //   * `stdout_tx` (mpsc): complete lines only, for the signal
        //     parser in `SignalPump`.
        // Signals still appear in the raw byte stream; the terminal sees
        // them alongside other output, which matches PTY semantics.
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(256);
        let (raw_output_tx, _) = broadcast::channel::<Vec<u8>>(1024);
        let raw_tx_reader = raw_output_tx.clone();
        let kill_reader = kill_flag.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut line_buf = LineBuffer::new();
            while !kill_reader.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        // Broadcast raw bytes first so UI rendering is not
                        // blocked by slow line-channel consumers.
                        let _ = raw_tx_reader.send(buf[..n].to_vec());
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

        let master = Arc::new(StdMutex::new(pair.master));

        Ok(Self {
            pid,
            stdout_rx,
            raw_output_tx,
            stdin_tx,
            kill_flag,
            master,
            child,
        })
    }

    /// Next complete line of stdout, or `None` if the stream has ended.
    pub async fn next_line(&mut self) -> Option<String> {
        self.stdout_rx.recv().await
    }

    /// Subscribe to the raw byte stream from the pty master.
    /// Multiple subscribers are allowed; slow consumers see `Lagged`.
    pub fn subscribe_raw_output(&self) -> broadcast::Receiver<Vec<u8>> {
        self.raw_output_tx.subscribe()
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

    /// Resize the pty. Forwarded to the OS so the child process receives
    /// SIGWINCH and can redraw at the new dimensions.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .lock()
            .unwrap()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Pty(format!("resize: {e}")))
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

use crate::db::Db;
use crate::signals::{parse_signal, Signal};
use uuid::Uuid;

/// Shared, mutable cycle-id slot threaded into [`SignalPump::start`].
/// The recording task reads this on every signal so rotating the cycle
/// (e.g. on STATUS:COMPLETE) correctly re-attributes subsequent signals.
pub type CycleSlot = Arc<StdMutex<Option<Uuid>>>;

/// Wraps a `PtyHandle`. Runs a background task that reads complete lines from
/// the pty, parses them for signals, persists signals to the db, and forwards
/// parsed signals to the ritual driver via an mpsc channel.
///
/// Terminal output is NOT re-broadcast here — the raw byte stream on
/// `PtyHandle::raw_output_tx` is what the WebSocket serves. Signals therefore
/// also appear verbatim in the terminal view (matching PTY semantics).
pub struct SignalPump {
    signal_rx: mpsc::Receiver<Signal>,
    raw_output_tx: broadcast::Sender<Vec<u8>>,
    handle: PtyHandleShared,
    _task: tokio::task::JoinHandle<()>,
}

/// A clonable "write + kill + resize" view of the underlying pty.
#[derive(Clone)]
pub struct PtyHandleShared {
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_flag: Arc<AtomicBool>,
    pid: u32,
    master: SharedMaster,
}

impl PtyHandleShared {
    pub async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.stdin_tx
            .send(bytes.to_vec())
            .await
            .map_err(|_| Error::Pty("stdin channel closed".into()))
    }

    pub async fn write_str(&self, s: &str) -> Result<()> {
        self.write(s.as_bytes()).await
    }

    pub fn kill(&self) {
        self.kill_flag.store(true, Ordering::Relaxed);
        if self.pid > 0 {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &self.pid.to_string()])
                .status();
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .lock()
            .unwrap()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Pty(format!("resize: {e}")))
    }
}

impl SignalPump {
    pub fn start(mut pty: PtyHandle, db: Db, session_id: Uuid, cycle_id: CycleSlot) -> Self {
        let (signal_tx, signal_rx) = mpsc::channel::<Signal>(64);
        let raw_output_tx = pty.raw_output_tx.clone();
        let shared = PtyHandleShared {
            stdin_tx: pty.stdin_tx.clone(),
            kill_flag: pty.kill_flag.clone(),
            pid: pty.pid,
            master: pty.master.clone(),
        };

        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    // Read the current cycle id on each signal so rotations
                    // (STATUS:COMPLETE → new cycle) are reflected immediately.
                    let cur_cycle = *cycle_id.lock().unwrap();
                    let _ = db.record_signal(
                        session_id,
                        cur_cycle,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                }
            }
        });

        Self {
            signal_rx,
            raw_output_tx,
            handle: shared,
            _task: task,
        }
    }

    pub async fn next_signal(&mut self) -> Option<Signal> {
        self.signal_rx.recv().await
    }

    pub fn subscribe_output(&self) -> broadcast::Receiver<Vec<u8>> {
        self.raw_output_tx.subscribe()
    }

    pub fn output_sender(&self) -> broadcast::Sender<Vec<u8>> {
        self.raw_output_tx.clone()
    }

    pub fn handle(&self) -> PtyHandleShared {
        self.handle.clone()
    }
}
