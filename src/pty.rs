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

/// Server-side terminal screen model paired with a tokio broadcast channel
/// of raw PTY bytes. The PTY reader thread calls [`Scrollback::push`]; new
/// subscribers call [`Scrollback::snapshot_and_subscribe`] to atomically
/// grab a full repaint of the current screen state AND a receiver for
/// every byte afterwards.
///
/// Why a screen emulator instead of a raw byte ring: alt-screen TUIs
/// (claude, vim, htop, less) emit incremental updates that assume a prior
/// frame is already painted. Replaying raw bytes recreates alt-screen
/// enter/exit sequences and partial cursor moves out of context, leaving
/// the terminal showing fragments. A `vt100::Parser` fed the same bytes
/// maintains the cell grid; on reattach we ask it for a freshly-formatted
/// repaint of the *current* screen, which is what tmux attach does.
///
/// Atomicity contract: `push` holds the lock across (parse → broadcast),
/// and `snapshot_and_subscribe` holds the lock across (format → subscribe).
/// This guarantees every byte either appears in the formatted snapshot OR
/// arrives via the receiver, never both, never neither.
pub struct Scrollback {
    parser: StdMutex<vt100::Parser>,
    tx: broadcast::Sender<Vec<u8>>,
}

impl Scrollback {
    pub fn new(rows: u16, cols: u16) -> Self {
        let (tx, _) = broadcast::channel::<Vec<u8>>(1024);
        Self {
            // scrollback_len=0: TUIs manage their own history; we only
            // care about reproducing the visible screen on reattach.
            parser: StdMutex::new(vt100::Parser::new(rows, cols, 0)),
            tx,
        }
    }

    pub fn push(&self, bytes: &[u8]) {
        let mut p = self.parser.lock().unwrap();
        p.process(bytes);
        let _ = self.tx.send(bytes.to_vec());
    }

    pub fn snapshot_and_subscribe(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>) {
        let p = self.parser.lock().unwrap();
        let snapshot = p.screen().contents_formatted();
        let rx = self.tx.subscribe();
        drop(p);
        (snapshot, rx)
    }

    /// Subscribe for future bytes only — does NOT include scrollback.
    /// The terminal WebSocket uses this paired with [`Scrollback::snapshot`]
    /// taken AFTER a client-driven resize, because a snapshot taken before
    /// the parser knows the client's grid dims paints absolute cursor
    /// positions onto the wrong cells (alt-screen TUIs render scrambled).
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tx.subscribe()
    }

    /// Current screen state as a self-contained ANSI repaint sequence.
    /// Call after resizing the parser so absolute cursor positions match
    /// the dims the client's terminal emulator is actually using.
    pub fn snapshot(&self) -> Vec<u8> {
        self.parser.lock().unwrap().screen().contents_formatted()
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        self.parser.lock().unwrap().screen_mut().set_size(rows, cols);
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
        const INIT_ROWS: u16 = 40;
        const INIT_COLS: u16 = 120;
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: INIT_ROWS,
                cols: INIT_COLS,
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
        let scrollback = Arc::new(Scrollback::new(INIT_ROWS, INIT_COLS));
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
    /// SIGWINCH and can redraw at the new dimensions; also resizes the
    /// internal vt100 parser so a subsequent reattach snapshot matches.
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
            .map_err(|e| Error::Pty(format!("resize: {e}")))?;
        self.scrollback.resize(rows, cols);
        Ok(())
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
    scrollback: Arc<Scrollback>,
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
            .map_err(|e| Error::Pty(format!("resize: {e}")))?;
        self.scrollback.resize(rows, cols);
        Ok(())
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
            scrollback: scrollback.clone(),
        };

        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    // Legacy: write to the `signals` table for the old API path.
                    // Sub-plan 5 removes this. The new pipeline below funnels the
                    // same data through state::apply_event so the events log is
                    // populated even though hooks/MCP haven't shipped yet.
                    let cur_cycle = *cycle_id.lock().unwrap();
                    let _ = db.record_signal(
                        session_id,
                        cur_cycle,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );

                    // New funnel path. Source::System because the legacy parser
                    // is an internal bridge, not a model-chosen MCP tool.
                    // TODO(sub-plan-5): migrate to apply_event_with_bus once the
                    // SignalPump has access to an EventBroadcaster. The signal
                    // pipeline is replaced wholesale in Sub-plan 5, so plumbing
                    // the bus here now would be wasted churn.
                    let _ = crate::state::apply_event(
                        &db,
                        session_id,
                        crate::state::EventInput {
                            source: crate::state::Source::System,
                            kind: format!("signal.{}", sig.kind()),
                            payload: serde_json::json!({
                                "raw_kind": sig.kind(),
                                "payload": sig.payload(),
                            }),
                            // Allow replays (no dedup) — the signals table
                            // already has its own append history.
                            dedup_key: None,
                        },
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

    /// Render the formatted snapshot through a fresh parser to get the
    /// visible cell text, so tests can assert on what a reattaching client
    /// would actually see (not raw ANSI bytes).
    fn rendered(snap: &[u8], rows: u16, cols: u16) -> String {
        let mut p = vt100::Parser::new(rows, cols, 0);
        p.process(snap);
        p.screen().contents()
    }

    #[test]
    fn snapshot_repaints_visible_text() {
        let sb = Scrollback::new(24, 80);
        sb.push(b"hello ");
        sb.push(b"world");
        let (snap, _rx) = sb.snapshot_and_subscribe();
        assert!(
            rendered(&snap, 24, 80).contains("hello world"),
            "expected snapshot to repaint 'hello world'"
        );
    }

    #[test]
    fn snapshot_reflects_alt_screen_state_not_byte_history() {
        // Enter alt-screen, draw "FRAME_A", clear and draw "FRAME_B".
        // A raw byte ring would replay both frames; a screen emulator
        // only repaints the current visible state ("FRAME_B").
        let sb = Scrollback::new(24, 80);
        sb.push(b"\x1b[?1049h"); // alt-screen enter
        sb.push(b"FRAME_A");
        sb.push(b"\x1b[2J\x1b[H"); // clear + home
        sb.push(b"FRAME_B");
        let (snap, _rx) = sb.snapshot_and_subscribe();
        let text = rendered(&snap, 24, 80);
        assert!(text.contains("FRAME_B"), "snapshot must contain FRAME_B");
        assert!(
            !text.contains("FRAME_A"),
            "snapshot must NOT contain superseded FRAME_A; got: {text:?}"
        );
    }

    #[tokio::test]
    async fn subscribe_receives_future_pushes_after_snapshot() {
        let sb = Scrollback::new(24, 80);
        let (_snap, mut rx) = sb.snapshot_and_subscribe();
        sb.push(b"future");
        let bytes = rx.recv().await.unwrap();
        assert_eq!(bytes, b"future");
    }

    #[tokio::test]
    async fn subscribe_does_not_double_deliver_pre_snapshot_bytes() {
        let sb = Scrollback::new(24, 80);
        sb.push(b"old");
        let (_snap, mut rx) = sb.snapshot_and_subscribe();
        // Receiver must not yield "old" — it was subscribed AFTER the
        // broadcast happened. Polling immediately should see no message.
        let res =
            tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(res.is_err(), "receiver yielded a stale message: {res:?}");
    }
}
