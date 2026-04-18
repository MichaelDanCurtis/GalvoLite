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
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{broadcast, mpsc};

/// Bounded ring buffer of recent PTY bytes paired with a tokio broadcast
/// channel. The PTY reader thread calls [`Scrollback::push`]; new terminal
/// subscribers call [`Scrollback::snapshot_and_subscribe`] to atomically
/// grab the current scrollback AND a receiver for everything afterwards.
///
/// Solves the "blank terminal when reattaching" problem: a fresh
/// `broadcast::Receiver` only sees future bytes, so reopening an expanded
/// session view used to show nothing until the child wrote more — for an
/// idle TUI that may be never. With replay, the new view repaints to the
/// child's current screen state immediately.
///
/// Atomicity contract: `push` holds the lock across (append → broadcast),
/// and `snapshot_and_subscribe` holds the lock across (clone → subscribe).
/// This guarantees every byte appears in the snapshot OR the receiver,
/// never both, never neither.
pub struct Scrollback {
    inner: StdMutex<VecDeque<u8>>,
    cap: usize,
    tx: broadcast::Sender<Vec<u8>>,
}

impl Scrollback {
    pub fn new(cap: usize) -> Self {
        let (tx, _) = broadcast::channel::<Vec<u8>>(1024);
        Self {
            inner: StdMutex::new(VecDeque::with_capacity(cap.min(64 * 1024))),
            cap,
            tx,
        }
    }

    pub fn push(&self, bytes: &[u8]) {
        let mut buf = self.inner.lock().unwrap();
        buf.extend(bytes);
        if buf.len() > self.cap {
            let drop_n = buf.len() - self.cap;
            buf.drain(0..drop_n);
        }
        let _ = self.tx.send(bytes.to_vec());
    }

    pub fn snapshot_and_subscribe(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>) {
        let buf = self.inner.lock().unwrap();
        let snapshot: Vec<u8> = buf.iter().copied().collect();
        let rx = self.tx.subscribe();
        drop(buf);
        (snapshot, rx)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }
}

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
    pub(crate) scrollback: Arc<Scrollback>,
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
        //   * `scrollback` (broadcast + ring buffer): every byte verbatim,
        //     for the terminal WebSocket — preserves ANSI/cursor sequences
        //     that line-based reassembly would destroy. The ring buffer
        //     lets a reattaching client see the child's current screen
        //     state instead of a blank terminal.
        //   * `stdout_tx` (mpsc): complete lines only, for the signal
        //     parser in `SignalPump`.
        // Signals still appear in the raw byte stream; the terminal sees
        // them alongside other output, which matches PTY semantics.
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(256);
        // 256 KiB scrollback: comfortably covers a full TUI screen at
        // ~220x100 cells (Claude Code) plus ANSI overhead, with margin.
        let scrollback = Arc::new(Scrollback::new(256 * 1024));
        let scrollback_reader = scrollback.clone();
        let kill_reader = kill_flag.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut line_buf = LineBuffer::new();
            while !kill_reader.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        // Push to scrollback first so UI rendering is not
                        // blocked by slow line-channel consumers.
                        scrollback_reader.push(&buf[..n]);
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
            scrollback,
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

    /// Subscribe to the raw byte stream from the pty master, returning the
    /// current scrollback snapshot AND a receiver for everything afterwards.
    /// Multiple subscribers are allowed; slow consumers see `Lagged`.
    pub fn snapshot_and_subscribe(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>) {
        self.scrollback.snapshot_and_subscribe()
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
/// Terminal output is NOT re-broadcast here — the [`Scrollback`] owned by
/// `PtyHandle` is what the WebSocket serves. Signals therefore also appear
/// verbatim in the terminal view (matching PTY semantics).
pub struct SignalPump {
    signal_rx: mpsc::Receiver<Signal>,
    scrollback: Arc<Scrollback>,
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
        let scrollback = pty.scrollback.clone();
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
                    if let Signal::RoleSet { role } = &sig {
                        let _ = db.set_session_role(session_id, role);
                    }
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                }
            }
        });

        Self {
            signal_rx,
            scrollback,
            handle: shared,
            _task: task,
        }
    }

    pub async fn next_signal(&mut self) -> Option<Signal> {
        self.signal_rx.recv().await
    }

    /// Subscribe for future bytes only. Used by tests and callers that
    /// don't need scrollback replay.
    pub fn subscribe_output(&self) -> broadcast::Receiver<Vec<u8>> {
        self.scrollback.subscribe()
    }

    /// The session-local scrollback handle. Callers use this to atomically
    /// snapshot the current screen state and subscribe for future bytes.
    pub fn scrollback(&self) -> Arc<Scrollback> {
        self.scrollback.clone()
    }

    pub fn handle(&self) -> PtyHandleShared {
        self.handle.clone()
    }
}

#[cfg(test)]
mod scrollback_tests {
    use super::Scrollback;

    #[test]
    fn snapshot_replays_bytes_pushed_before_subscribe() {
        let sb = Scrollback::new(1024);
        sb.push(b"hello ");
        sb.push(b"world");
        let (snap, _rx) = sb.snapshot_and_subscribe();
        assert_eq!(snap, b"hello world");
    }

    #[test]
    fn evicts_oldest_bytes_when_over_cap() {
        let sb = Scrollback::new(4);
        sb.push(b"abc");
        sb.push(b"defg"); // total 7, cap 4 → keep last 4: "defg"
        let (snap, _rx) = sb.snapshot_and_subscribe();
        assert_eq!(snap, b"defg");
    }

    #[tokio::test]
    async fn subscribe_receives_future_pushes_after_snapshot() {
        let sb = Scrollback::new(1024);
        let (snap, mut rx) = sb.snapshot_and_subscribe();
        assert!(snap.is_empty());
        sb.push(b"future");
        let bytes = rx.recv().await.unwrap();
        assert_eq!(bytes, b"future");
    }

    #[tokio::test]
    async fn subscribe_does_not_double_deliver_pre_snapshot_bytes() {
        let sb = Scrollback::new(1024);
        sb.push(b"old");
        let (snap, mut rx) = sb.snapshot_and_subscribe();
        assert_eq!(snap, b"old");
        // Receiver must not also yield "old" — it was subscribed AFTER the
        // broadcast happened. Polling immediately should see no message.
        let res =
            tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(res.is_err(), "receiver yielded a stale message: {res:?}");
    }
}
