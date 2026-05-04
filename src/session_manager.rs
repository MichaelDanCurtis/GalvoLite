//! Session lifecycle manager.
//!
//! Extracted from the `cmd_start_session` monolith in main.rs.
//! Owns the Db, tracks running sessions in-memory, provides spawn/list/get/
//! kill/handoff/send/subscribe methods for the API and CLI.

use crate::api::events::{DomainEvent, EventBroadcaster};
use crate::api::operator::OperatorPromptRegistry;
use crate::claude_md::{implementation_profile, InjectionPrompt};
use crate::db::{Db, NewSession, Session};
use crate::error::{Error, Result};
use crate::handoff::HandoffDir;
use crate::pty::{PtyHandle, PtyHandleShared, PtyOpts, Scrollback};
use crate::ritual::{PtySideEffects, RitualDriver};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::broadcast;
use uuid::Uuid;
use rusqlite::OptionalExtension;

struct RunningSession {
    driver: Arc<RitualDriver>,
    handle: PtyHandleShared,
    scrollback: Arc<Scrollback>,
    /// Current cycle id for this running session. Shared with the
    /// PTY-exit watch task so the cycle can be closed cleanly when the
    /// child exits naturally or via `kill_session`.
    cycle_id: Arc<StdMutex<Option<Uuid>>>,
    /// Handoff directory for this session. Stashed so `kill_session` can
    /// look up the latest handoff file when closing the current cycle,
    /// mirroring the natural-exit bg task's behavior.
    handoff_dir: HandoffDir,
}

pub struct SessionManager {
    db: Db,
    data_dir: PathBuf,
    running: StdMutex<HashMap<Uuid, RunningSession>>,
    mcp_registry: crate::mcp::registry::ToolRegistry,
    event_bus: EventBroadcaster,
    /// In-memory map of pending `ask_operator` prompts. The MCP handler
    /// parks on a oneshot keyed by a fresh `prompt_id`; the
    /// `POST /api/operator/prompt/:id/answer` route resolves it. Kept on the
    /// SessionManager so both callers share the same pending-map.
    operator_registry: Arc<OperatorPromptRegistry>,
    /// Per-project directive watchers. Keyed by project_id; each entry holds
    /// a notify::RecommendedWatcher whose Drop stops the watcher. Populated
    /// by `start_directive_watchers` (startup) and `add_project` (live add),
    /// drained by `delete_project`.
    project_watchers: StdMutex<HashMap<Uuid, crate::directive_watcher::WatcherHandle>>,
    /// Coarse serialisation lock for phase advancement. Held by
    /// `advance_next_phase` for the full peek→pop→spawn→UPDATE→publish
    /// sequence. Prevents TOCTOU races between the phase reactor and the
    /// operator `approve_phase` path. One lock per `SessionManager` (≈ per
    /// running process); phase advancement is low-frequency so contention is
    /// negligible.
    phase_advancement_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SessionManager {
    /// Backwards-compatible constructor — creates an internal broadcaster
    /// with a 256-frame buffer. Tests that don't observe events use this.
    pub fn new(db: Db, data_dir: PathBuf) -> Self {
        // 256-frame buffer is enough headroom for normal dashboard load.
        // Lagged subscribers drop frames and re-fetch on reconnect (see
        // EventBroadcaster::publish doc). OperatorPrompt durability is
        // guaranteed by OperatorPromptRegistry (Task 6), not the broadcast
        // ring — a dropped frame just delays modal display until a HTTP poll.
        Self::with_event_bus(db, data_dir, EventBroadcaster::new(256))
    }

    /// Construct a `SessionManager` with a caller-supplied `EventBroadcaster`.
    /// Use this when the caller (e.g. the dashboard server in `main.rs`) needs
    /// to retain a clone of the broadcaster to subscribe or publish independently.
    pub fn with_event_bus(db: Db, data_dir: PathBuf, event_bus: EventBroadcaster) -> Self {
        match db.reconcile_orphaned_sessions() {
            Ok(0) => {}
            Ok(n) => tracing::info!("reconciled {n} orphaned session(s) to Error on startup"),
            Err(e) => tracing::warn!("startup session reconciliation failed: {e}"),
        }
        if let Err(e) = recover_non_terminal_sessions(&db, &event_bus) {
            tracing::warn!("startup recovery failed: {e}");
        }
        Self {
            db,
            data_dir,
            running: StdMutex::new(HashMap::new()),
            mcp_registry: crate::mcp::registry::ToolRegistry::build(),
            event_bus,
            operator_registry: OperatorPromptRegistry::new(),
            project_watchers: StdMutex::new(HashMap::new()),
            phase_advancement_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Shared handle to the `ask_operator` pending-prompt registry. The MCP
    /// handler calls `register_and_wait`; the HTTP answer route calls
    /// `answer`. Both go through this one Arc.
    pub fn operator_registry(&self) -> &Arc<OperatorPromptRegistry> {
        &self.operator_registry
    }

    /// Subscribe to the domain-event stream. Each call returns an independent
    /// `broadcast::Receiver` — the WS handler in `api::mod` calls this once per
    /// connected dashboard socket.
    pub fn subscribe_events(&self) -> broadcast::Receiver<DomainEvent> {
        self.event_bus.subscribe()
    }

    /// Publish-side handle to the broadcaster. MCP tool handlers call this to
    /// emit `OperatorToast` / `OperatorPrompt` / `SessionRoleChanged` events.
    pub fn event_bus(&self) -> &EventBroadcaster {
        &self.event_bus
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Arc-clone of the phase-advancement lock. Both the phase reactor and the
    /// `approve_phase` handler acquire this before any peek/pop/spawn sequence
    /// to prevent TOCTOU double-advance.
    pub fn phase_advancement_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(&self.phase_advancement_lock)
    }

    /// Access the MCP tool registry built at startup.
    pub fn mcp_registry(&self) -> &crate::mcp::registry::ToolRegistry {
        &self.mcp_registry
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub async fn spawn_session(
        self: &Arc<Self>,
        project_id: Uuid,
        task: String,
        role: String,
        claude_cmd: String,
        args: Vec<String>,
    ) -> Result<Session> {
        let project = self.db.get_project(project_id)?;
        let session = self.db.insert_session(NewSession {
            project_id: project.id,
            role: role.clone(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: task.clone(),
            parent_session_id: None,
        })?;

        let worktree = PathBuf::from(&project.path);
        let galvolite_dir = worktree.join(".galvolite");
        if let Err(e) = std::fs::create_dir_all(&galvolite_dir) {
            // Filesystem setup failed before start_cycle — only the session
            // row exists. Route through apply_event so events log records
            // why this session ended in Error.
            orphan_session(&self.db, &self.event_bus, session.id, "galvolite_dir_create_failed");
            return Err(Error::io(&galvolite_dir, e));
        }
        let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
        if let Err(e) = std::fs::write(&claude_md_path, implementation_profile()) {
            orphan_session(&self.db, &self.event_bus, session.id, "claude_md_write_failed");
            return Err(Error::io(&claude_md_path, e));
        }
        let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
        if let Err(e) = crate::handoff::ensure_session_dir(&handoff_dir, session.id) {
            orphan_session(&self.db, &self.event_bus, session.id, "handoff_dir_create_failed");
            return Err(e);
        }

        // Write per-session .claude/settings.json so claude code's hook
        // dispatch table points all seven hooks at our galvolite-hook
        // binary, plus .mcp.json so claude discovers our Streamable HTTP
        // MCP server with the per-session bearer token.
        match crate::spawn_config::resolve_hook_binary() {
            Ok(hook_bin) => {
                if let Err(e) = crate::spawn_config::write_settings_json(
                    &worktree,
                    hook_bin.to_string_lossy().as_ref(),
                ) {
                    orphan_session(&self.db, &self.event_bus, session.id, "settings_json_write_failed");
                    return Err(Error::io(worktree.join(".claude/settings.json"), e));
                }
            }
            Err(e) => {
                orphan_session(&self.db, &self.event_bus, session.id, "hook_binary_resolve_failed");
                return Err(Error::io(PathBuf::from("galvolite-hook"), e));
            }
        }

        // Mint per-session bearer token, persist to sessions.api_token, and
        // write .mcp.json so the spawned claude process can authenticate to
        // our /mcp endpoint. Token format `tok_<uuid-simple>` matches the
        // pattern used in tests/dashboard_smoke_e2e.rs.
        let api_token = format!("tok_{}", Uuid::new_v4().simple());
        if let Err(e) = self.db.set_api_token(session.id, &api_token) {
            orphan_session(&self.db, &self.event_bus, session.id, "api_token_persist_failed");
            return Err(e);
        }
        let api_base = crate::spawn_config::resolve_api_base();
        if let Err(e) = crate::spawn_config::write_mcp_json(&worktree, &api_base, &api_token) {
            orphan_session(&self.db, &self.event_bus, session.id, "mcp_json_write_failed");
            return Err(e);
        }

        let cycle_id = self.db.start_cycle(session.id)?;

        // PTY spawn. If this fails, session + cycle rows exist but no
        // process — roll back the DB rows so nothing is orphaned.
        let pty = match PtyHandle::spawn(PtyOpts {
            cwd: worktree,
            command: claude_cmd,
            args,
            env: vec![],
        }) {
            Ok(p) => p,
            Err(e) => {
                let _ = self.db.end_cycle_if_unended(cycle_id, None);
                orphan_session(&self.db, &self.event_bus, session.id, "pty_spawn_failed");
                return Err(e);
            }
        };

        // From this point on, the PTY child is alive. Any fallible step
        // before the bg task owns cleanup must kill the child + roll back.
        if let Err(e) = self.db.update_session_pid(session.id, pty.pid as i64) {
            drop(pty);
            let _ = self.db.end_cycle_if_unended(cycle_id, None);
            orphan_session(&self.db, &self.event_bus, session.id, "update_pid_failed");
            return Err(e);
        }

        let cycle_id_slot: Arc<StdMutex<Option<Uuid>>> =
            Arc::new(StdMutex::new(Some(cycle_id)));
        let handle = pty.shared();
        let scrollback = pty.scrollback.clone();

        let fx = Arc::new(PtySideEffects::new(handle.clone()));
        let driver = Arc::new(RitualDriver::new(fx));
        let started = crate::state::apply_event_with_bus(
            &self.db,
            session.id,
            &self.event_bus,
            crate::state::EventInput {
                source: crate::state::Source::System,
                kind: "system.session_started".into(),
                payload: serde_json::json!({ "role": session.role.clone() }),
                dedup_key: Some(format!("session-started:{}", session.id)),
            },
        );
        if let Err(e) = started {
            handle.kill();
            let _ = self.db.end_cycle_if_unended(cycle_id, None);
            orphan_session(&self.db, &self.event_bus, session.id, "session_started_event_failed");
            return Err(e);
        }

        // Insert into running map + spawn bg task BEFORE the first fallible
        // interactive step (write_str). That way, if anything after this
        // line fails, the bg task already owns the cleanup path OR we run
        // an explicit rollback that mirrors it — nothing leaks.
        {
            let mut running = self.running.lock().unwrap();
            running.insert(
                session.id,
                RunningSession {
                    driver: driver.clone(),
                    handle: handle.clone(),
                    scrollback,
                    cycle_id: cycle_id_slot.clone(),
                    handoff_dir: handoff_dir.clone(),
                },
            );
        }

        // Spawn background task that owns the PtyHandle (so its Drop kills
        // the child on cleanup) and watches for natural exit. Cycles end
        // here on PTY exit; rotation on STATUS:COMPLETE is gone (replaced
        // by hooks/MCP).
        let mgr = Arc::clone(self);
        let session_id = session.id;
        let handoff_dir_bg = handoff_dir.clone();
        let cycle_id_slot_bg = cycle_id_slot.clone();
        tokio::spawn(async move {
            let mut pty_owner = pty;
            pty_owner.wait_exit().await;

            let prev_cycle = cycle_id_slot_bg.lock().unwrap().take();
            if let Some(prev) = prev_cycle {
                let handoff_path = crate::handoff::latest_handoff_for(&handoff_dir_bg, session_id)
                    .ok()
                    .flatten()
                    .map(|p| p.to_string_lossy().into_owned());
                let _ = mgr.db.end_cycle_if_unended(prev, handoff_path.as_deref());
            }
            let _ = crate::state::apply_event_with_bus(
                &mgr.db,
                session_id,
                &mgr.event_bus,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind: "system.session_ended".into(),
                    payload: serde_json::json!({ "reason": "pty_exit" }),
                    dedup_key: Some(format!("session-ended:{session_id}")),
                },
            );
            let mut running = mgr.running.lock().unwrap();
            running.remove(&session_id);
        });

        // Injection is the final fallible step. Bg task is already running,
        // so on failure we kill the child — the bg task will see the pty
        // close and run its full cleanup (end cycle, end session).
        let injection = InjectionPrompt {
            task_summary: &task,
            session_id: &session.id.to_string(),
            claude_md_path: &claude_md_path.to_string_lossy(),
        };
        if let Err(e) = handle
            .write_str(&format!("{}\n", injection.render()))
            .await
        {
            handle.kill();
            // Belt-and-braces: actively mark Error in case bg task's drain
            // races us. Route through apply_event; the bg task's
            // system.session_ended will be a no-op once we land Error
            // (terminal-state guard). The bg task still removes the
            // running-map entry.
            orphan_session(&self.db, &self.event_bus, session.id, "prompt_inject_failed");
            return Err(e);
        }

        Ok(session)
    }

    pub async fn spawn_child(
        self: &Arc<Self>,
        parent_id: Uuid,
        role: &str,
        task: &str,
        plan_part: Option<u32>,
    ) -> Result<crate::db::Session> {
        let parent = self.db.get_session(parent_id)?;
        let project = self.db.get_project(parent.project_id)?;
        let repo_root = std::path::PathBuf::from(&project.path);

        // Resolve parent's HEAD as base ref so the child sees parent WIP commits.
        let head_out = std::process::Command::new("git")
            .args(["-C", parent.worktree_path.as_str(), "rev-parse", "HEAD"])
            .output()
            .map_err(|e| Error::io(&repo_root, e))?;
        let base_ref = if head_out.status.success() {
            String::from_utf8_lossy(&head_out.stdout).trim().to_string()
        } else {
            "HEAD".to_string()
        };

        // Provisional UUID for branch derivation; insert may produce a different one.
        let child_id = Uuid::new_v4();
        let (wt_path, branch) = crate::worktree::add_worktree(&repo_root, child_id, &base_ref)?;

        let session = self.db.insert_session(crate::db::NewSession {
            project_id: parent.project_id,
            role: role.to_string(),
            branch: Some(branch.clone()),
            worktree_path: wt_path.to_string_lossy().into_owned(),
            task_summary: task.to_string(),
            parent_session_id: Some(parent_id),
        })?;

        // If insert produced a different id, rename the worktree dir to match.
        let _wt_path = if session.id != child_id {
            let new_wt = repo_root.join(".galvolite").join("worktrees").join(session.id.to_string());
            std::fs::rename(&wt_path, &new_wt).map_err(|e| Error::io(&new_wt, e))?;
            self.db.update_worktree_path(session.id, &new_wt.to_string_lossy())?;
            new_wt
        } else {
            wt_path
        };

        let session = self.db.get_session(session.id)?;
        let wt_pathbuf = std::path::PathBuf::from(&session.worktree_path);

        // Per-session bootstrap (mirror spawn_session)
        let galvolite_dir = wt_pathbuf.join(".galvolite");
        if let Err(e) = std::fs::create_dir_all(&galvolite_dir) {
            orphan_session(&self.db, &self.event_bus, session.id, "galvolite_dir_create_failed");
            return Err(Error::io(&galvolite_dir, e));
        }
        let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
        if let Err(e) = std::fs::write(&claude_md_path, crate::claude_md::profile_for_role(role)) {
            orphan_session(&self.db, &self.event_bus, session.id, "claude_md_write_failed");
            return Err(Error::io(&claude_md_path, e));
        }
        let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
        if let Err(e) = crate::handoff::ensure_session_dir(&handoff_dir, session.id) {
            orphan_session(&self.db, &self.event_bus, session.id, "handoff_dir_create_failed");
            return Err(e);
        }

        match crate::spawn_config::resolve_hook_binary() {
            Ok(hook_bin) => {
                if let Err(e) = crate::spawn_config::write_settings_json(
                    &wt_pathbuf,
                    hook_bin.to_string_lossy().as_ref(),
                ) {
                    orphan_session(&self.db, &self.event_bus, session.id, "settings_json_write_failed");
                    return Err(Error::io(wt_pathbuf.join(".claude/settings.json"), e));
                }
            }
            Err(e) => {
                orphan_session(&self.db, &self.event_bus, session.id, "hook_binary_resolve_failed");
                return Err(Error::io(std::path::PathBuf::from("galvolite-hook"), e));
            }
        }

        let api_token = format!("tok_{}", Uuid::new_v4().simple());
        if let Err(e) = self.db.set_api_token(session.id, &api_token) {
            orphan_session(&self.db, &self.event_bus, session.id, "api_token_persist_failed");
            return Err(e);
        }
        let api_base = crate::spawn_config::resolve_api_base();
        if let Err(e) = crate::spawn_config::write_mcp_json(&wt_pathbuf, &api_base, &api_token) {
            orphan_session(&self.db, &self.event_bus, session.id, "mcp_json_write_failed");
            return Err(e);
        }

        let claude_cmd = std::env::var("GALVOLITE_CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());
        let cycle_id = self.db.start_cycle(session.id)?;

        let pty = match PtyHandle::spawn(PtyOpts {
            cwd: wt_pathbuf.clone(),
            command: claude_cmd,
            args: vec![],
            env: vec![],
        }) {
            Ok(p) => p,
            Err(e) => {
                let _ = self.db.end_cycle_if_unended(cycle_id, None);
                orphan_session(&self.db, &self.event_bus, session.id, "pty_spawn_failed");
                return Err(e);
            }
        };

        if let Err(e) = self.db.update_session_pid(session.id, pty.pid as i64) {
            drop(pty);
            let _ = self.db.end_cycle_if_unended(cycle_id, None);
            orphan_session(&self.db, &self.event_bus, session.id, "update_pid_failed");
            return Err(e);
        }

        let cycle_id_slot: Arc<StdMutex<Option<Uuid>>> =
            Arc::new(StdMutex::new(Some(cycle_id)));
        let handle = pty.shared();
        let scrollback = pty.scrollback.clone();

        let fx = Arc::new(crate::ritual::PtySideEffects::new(handle.clone()));
        let driver = Arc::new(crate::ritual::RitualDriver::new(fx));

        // Emit parent-side spawn event (logging-only; ignore if it fails).
        let _ = crate::state::apply_event_with_bus(
            &self.db, parent_id, &self.event_bus,
            crate::state::EventInput {
                source: crate::state::Source::Mcp,
                kind: "mcp.spawn_session".into(),
                payload: serde_json::json!({ "child_id": session.id, "role": role, "branch": branch }),
                dedup_key: Some(format!("spawn:{}:{}", parent_id, session.id)),
            },
        );

        // Emit child session_started event.
        let started = crate::state::apply_event_with_bus(
            &self.db, session.id, &self.event_bus,
            crate::state::EventInput {
                source: crate::state::Source::System,
                kind: "system.session_started".into(),
                payload: serde_json::json!({ "role": role, "parent": parent_id }),
                dedup_key: Some(format!("session-started:{}", session.id)),
            },
        );
        if let Err(e) = started {
            handle.kill();
            let _ = self.db.end_cycle_if_unended(cycle_id, None);
            orphan_session(&self.db, &self.event_bus, session.id, "session_started_event_failed");
            return Err(e);
        }

        {
            let mut running = self.running.lock().unwrap();
            running.insert(
                session.id,
                RunningSession {
                    driver: driver.clone(),
                    handle: handle.clone(),
                    scrollback,
                    cycle_id: cycle_id_slot.clone(),
                    handoff_dir: handoff_dir.clone(),
                },
            );
        }

        let mgr = Arc::clone(self);
        let session_id = session.id;
        let handoff_dir_bg = handoff_dir.clone();
        let cycle_id_slot_bg = cycle_id_slot.clone();
        tokio::spawn(async move {
            let mut pty_owner = pty;
            pty_owner.wait_exit().await;

            let prev_cycle = cycle_id_slot_bg.lock().unwrap().take();
            if let Some(prev) = prev_cycle {
                let handoff_path = crate::handoff::latest_handoff_for(&handoff_dir_bg, session_id)
                    .ok()
                    .flatten()
                    .map(|p| p.to_string_lossy().into_owned());
                let _ = mgr.db.end_cycle_if_unended(prev, handoff_path.as_deref());
            }
            let _ = crate::state::apply_event_with_bus(
                &mgr.db,
                session_id,
                &mgr.event_bus,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind: "system.session_ended".into(),
                    payload: serde_json::json!({ "reason": "pty_exit" }),
                    dedup_key: Some(format!("session-ended:{session_id}")),
                },
            );
            let mut running = mgr.running.lock().unwrap();
            running.remove(&session_id);
        });

        // Injection prompt: prepend prior-context block (postmortems + decisions) if available.
        let prior = crate::claude_md::build_prior_context(
            std::path::Path::new(&project.path),
            plan_part,
        );
        let injection = InjectionPrompt {
            task_summary: task,
            session_id: &session_id.to_string(),
            claude_md_path: &claude_md_path.to_string_lossy(),
        };
        let full = if prior.is_empty() {
            format!("{}\n", injection.render())
        } else {
            format!("{}\n{}\n", prior, injection.render())
        };
        if let Err(e) = handle.write_str(&full).await {
            handle.kill();
            orphan_session(&self.db, &self.event_bus, session_id, "prompt_inject_failed");
            return Err(e);
        }

        // Re-fetch after all mutations so the returned row is fully up-to-date.
        let session = self.db.get_session(session_id)?;
        Ok(session)
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        self.db.list_sessions()
    }

    pub fn get_session(&self, id: Uuid) -> Result<Session> {
        self.db.get_session(id)
    }

    pub fn list_projects(&self) -> Result<Vec<crate::db::Project>> {
        self.db.list_projects()
    }

    pub fn get_project(&self, id: Uuid) -> Result<crate::db::Project> {
        self.db.get_project(id)
    }

    /// Register a project after validating + canonicalizing its path.
    ///
    /// Rejects:
    /// - empty paths,
    /// - non-absolute paths,
    /// - paths containing any `..` component,
    /// - paths that fail to canonicalize (don't exist / not accessible).
    ///
    /// The stored `path` is the canonical form, so callers (CLI, HTTP API)
    /// all see a consistent, resolved value. This is the security boundary
    /// for both the CLI and the HTTP API's `POST /api/projects` endpoint:
    /// without it, an HTTP client could register projects pointing at
    /// arbitrary paths on the host.
    pub fn add_project(&self, mut new: crate::db::NewProject) -> Result<crate::db::Project> {
        let canonical = validate_and_canonicalize_project_path(&new.path)?;
        new.path = canonical;
        let project = self.db.insert_project(new)?;
        self.spawn_watcher(project.id, std::path::Path::new(&project.path));
        self.event_bus().publish(
            crate::api::events::DomainEvent::ProjectAdded { project_id: project.id },
        );
        Ok(project)
    }

    pub async fn send_to_session(&self, id: Uuid, text: String, newline: bool) -> Result<()> {
        let driver = {
            let running = self.running.lock().unwrap();
            let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
            rs.driver.clone()
        };
        let mut buf = text.into_bytes();
        if newline {
            buf.push(b'\n');
        }
        driver.inject_raw(&buf).await;
        Ok(())
    }

    pub async fn trigger_handoff(&self, id: Uuid) -> Result<()> {
        let driver = {
            let running = self.running.lock().unwrap();
            let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
            rs.driver.clone()
        };
        driver.request_handoff().await;
        Ok(())
    }

    pub async fn kill_session(&self, id: Uuid) -> Result<()> {
        let rs = {
            let mut running = self.running.lock().unwrap();
            running.remove(&id)
        };
        let was_running = rs.is_some();
        if let Some(rs) = rs {
            rs.driver.abort("killed by user").await;
            rs.handle.kill();
            if let Some(cycle) = rs.cycle_id.lock().unwrap().take() {
                // Mirror the natural-exit bg task: attribute the latest
                // handoff file (if any) to the cycle we're closing. A scan
                // error here is silently treated as "no handoff" — we
                // don't want to bubble a filesystem error out of a kill
                // path.
                let handoff_path =
                    crate::handoff::latest_handoff_for(&rs.handoff_dir, id)
                        .ok()
                        .flatten()
                        .map(|p| p.to_string_lossy().into_owned());
                let _ = self
                    .db
                    .end_cycle_if_unended(cycle, handoff_path.as_deref());
            }
        }
        // Conditional write: if the session already ended naturally
        // (e.g. Complete), we must not clobber that terminal state.
        // Route through apply_event so events log + sessions.state move
        // atomically. dedup_key collapses any race against the bg-task
        // pty_exit path; one wins, one no-ops. Terminal-state guard inside
        // apply_event also returns Err if already Ended — treat as "not
        // killed by us" rather than bubbling.
        let killed = crate::state::apply_event_with_bus(
            &self.db,
            id,
            &self.event_bus,
            crate::state::EventInput {
                source: crate::state::Source::Operator,
                kind: "system.session_ended".into(),
                payload: serde_json::json!({ "reason": "operator_kill" }),
                dedup_key: Some(format!("session-ended:{id}")),
            },
        );
        let killed_ok = killed.is_ok();
        if !was_running && !killed_ok {
            // Not in the running map, and the DB says it was already ended.
            // Report this rather than silently succeeding.
            return Err(Error::SessionNotRunning(id));
        }
        Ok(())
    }

    /// Toggle the session's `autopilot` flag. State-only in Plan 2; Plan 3
    /// wires behavior off this flag.
    pub async fn set_autopilot(&self, id: Uuid, enabled: bool) -> Result<()> {
        self.db.set_session_autopilot(id, enabled)
    }

    /// Cascade-delete a project: kill any running sessions under this project,
    /// then purge rows via `Db::delete_project_cascade`.
    ///
    /// Returns `Error::ProjectNotFound` if the id is unknown.
    /// Kill errors on a per-session basis are best-effort: if a PTY is already
    /// dead the session row is still purged. If every kill succeeds but the
    /// cascade fails (e.g. DB closed), the kills are not rolled back.
    pub async fn delete_project(&self, project_id: Uuid) -> Result<()> {
        // Early 404 so we don't half-kill sessions of a nonexistent project.
        let _ = self.db.get_project(project_id).map_err(|e| match e {
            Error::Db(rusqlite::Error::QueryReturnedNoRows) => Error::ProjectNotFound(project_id),
            other => other,
        })?;

        // Snapshot sessions that belong to this project and are currently running.
        let running: Vec<Uuid> = self
            .db
            .list_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.project_id == project_id)
            .map(|s| s.id)
            .filter(|id| self.running_ids().contains(id))
            .collect();

        for id in running {
            if let Err(e) = self.kill_session(id).await {
                tracing::warn!(session_id = %id, error = ?e, "kill during project delete failed; continuing");
            }
        }

        // Drop watcher BEFORE cascade so the directory removal (if any)
        // doesn't race notify on Linux/inotify.
        self.project_watchers.lock().unwrap().remove(&project_id);

        self.db.delete_project_cascade(project_id)?;
        Ok(())
    }

    /// Number of sessions currently tracked in the running map.
    /// Useful for tests and diagnostics; not part of the stable API.
    #[doc(hidden)]
    pub fn running_count(&self) -> usize {
        self.running.lock().unwrap().len()
    }

    /// Snapshot of currently-running session ids.
    ///
    /// Collects keys under the mutex, returns a `Vec` so callers can drop
    /// the lock before doing async work per-id. This is what `serve`'s
    /// graceful-shutdown path uses to enumerate sessions to kill —
    /// `kill_session` re-locks the running map, so we must not hold the
    /// lock across that call.
    pub fn running_ids(&self) -> Vec<Uuid> {
        self.running.lock().unwrap().keys().copied().collect()
    }

    /// Returns `(scrollback_snapshot, future_bytes_receiver)`. The snapshot
    /// is the current ring-buffer contents (most recent ~256 KiB of PTY
    /// output); the receiver streams everything after the subscribe point.
    /// Together they let a fresh terminal client repaint to the child's
    /// current screen state instead of starting from blank.
    pub fn subscribe_output(
        &self,
        id: Uuid,
    ) -> Result<(Vec<u8>, broadcast::Receiver<Vec<u8>>)> {
        let running = self.running.lock().unwrap();
        let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
        Ok(rs.scrollback.snapshot_and_subscribe())
    }

    /// Subscribe for future PTY bytes only (no initial snapshot). The
    /// terminal WebSocket pairs this with [`SessionManager::snapshot_terminal`]
    /// taken AFTER the client has reported its grid dims via a resize frame.
    /// See `terminal_ws` for why the snapshot is deferred.
    pub fn subscribe_output_only(
        &self,
        id: Uuid,
    ) -> Result<broadcast::Receiver<Vec<u8>>> {
        let running = self.running.lock().unwrap();
        let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
        Ok(rs.scrollback.subscribe())
    }

    /// Capture the session's current screen state as a self-contained ANSI
    /// repaint sequence. Call after `resize_pty` so the cursor positions in
    /// the snapshot match the client's actual grid.
    pub fn snapshot_terminal(&self, id: Uuid) -> Result<Vec<u8>> {
        let running = self.running.lock().unwrap();
        let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
        Ok(rs.scrollback.snapshot())
    }

    /// Resize the underlying pty for a running session.
    ///
    /// Forwarded from the terminal WebSocket when the client reports a new
    /// cell grid. The child process receives SIGWINCH and can redraw.
    pub fn resize_pty(&self, id: Uuid, cols: u16, rows: u16) -> Result<()> {
        let running = self.running.lock().unwrap();
        let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
        rs.handle.resize(cols, rows)
    }

    /// Write bytes to the running session's pty (i.e. into the child
    /// process's stdin). Used by the terminal WebSocket for keystrokes.
    pub async fn write_pty(&self, id: Uuid, bytes: Vec<u8>) -> Result<()> {
        let handle = {
            let running = self.running.lock().unwrap();
            let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
            rs.handle.clone()
        };
        handle.write(&bytes).await
    }

    pub async fn session_ritual_state(
        &self,
        id: Uuid,
    ) -> Result<crate::ritual::RitualState> {
        let driver = {
            let running = self.running.lock().unwrap();
            let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
            rs.driver.clone()
        };
        Ok(driver.state().await)
    }

    pub async fn set_waiting_on(&self, parent_id: Uuid, children: &[Uuid]) -> Result<()> {
        let conn = self.db.pool().get()?;
        for c in children {
            let pid: Option<String> = conn.query_row(
                "SELECT parent_session_id FROM sessions WHERE id = ?1",
                [c.to_string()], |r| r.get(0),
            ).optional()?;
            match pid {
                Some(s) if s == parent_id.to_string() => {}
                _ => return Err(Error::Config(format!("child {c} not owned by parent {parent_id}"))),
            }
        }
        let json = serde_json::to_string(&children.iter().map(|u| u.to_string()).collect::<Vec<_>>())?;
        conn.execute(
            "UPDATE sessions SET waiting_on = ?1 WHERE id = ?2",
            rusqlite::params![json, parent_id.to_string()],
        )?;
        let _ = crate::state::apply_event_with_bus(
            &self.db, parent_id, &self.event_bus,
            crate::state::EventInput {
                source: crate::state::Source::Mcp,
                kind: "mcp.wait_children".into(),
                payload: serde_json::json!({ "children": children.iter().map(|u| u.to_string()).collect::<Vec<_>>() }),
                dedup_key: None,
            },
        );
        Ok(())
    }

    /// Spawn a background task that listens to the event bus and invokes
    /// `on_child_terminal` when an event indicates a session has terminated.
    ///
    /// Three trigger paths handled:
    /// - `mcp.status_report` with payload `kind=complete|blocked` → child reported done
    /// - `system.session_ended` with `reason=operator_kill` → terminal status "error"
    /// - `system.session_ended` with any other reason → terminal status "ended"
    ///
    /// Errors from `on_child_terminal` are swallowed (best-effort fan-out).
    ///
    /// Must be called from within a tokio runtime context.
    pub fn start_terminal_reactor(self: &Arc<Self>) {
        let mgr = Arc::clone(self);
        let mut rx = self.event_bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        if let DomainEvent::SessionEventAppended {
                            session_id, kind, payload, ..
                        } = ev {
                            match kind.as_str() {
                                "mcp.status_report" => {
                                    if let Some(k) = payload.get("kind").and_then(|v| v.as_str()) {
                                        if k == "complete" || k == "blocked" {
                                            let _ = mgr.on_child_terminal(session_id, k).await;
                                        }
                                    }
                                }
                                "system.session_ended" => {
                                    let reason = payload.get("reason").and_then(|v| v.as_str()).unwrap_or("ended");
                                    let status = if reason == "operator_kill" { "error" } else { "ended" };
                                    let _ = mgr.on_child_terminal(session_id, status).await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            "terminal reactor lagged by {n} events; some terminal events may be missed (recovery pass will re-fire)"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Walk every existing project and spawn a directive watcher for it.
    /// Idempotent: a project already watched is skipped. Errors are logged
    /// per-project, not propagated — one broken project must not block the
    /// rest.
    ///
    /// Call once at startup (after the runtime is up). `add_project` calls
    /// `spawn_watcher` directly so live-added projects don't need a re-scan.
    pub fn start_directive_watchers(&self) {
        let projects = match self.db.list_projects() {
            Ok(ps) => ps,
            Err(e) => {
                tracing::warn!(error = ?e, "list_projects for watcher startup failed");
                return;
            }
        };
        for p in projects {
            self.spawn_watcher(p.id, std::path::Path::new(&p.path));
        }
    }

    /// Internal: register a watcher for one project. Logs and swallows
    /// errors. Used by both startup and `add_project`.
    fn spawn_watcher(&self, project_id: Uuid, project_path: &std::path::Path) {
        {
            let watchers = self.project_watchers.lock().unwrap();
            if watchers.contains_key(&project_id) {
                return;
            }
        }
        match crate::directive_watcher::spawn(
            project_id,
            project_path,
            self.event_bus.clone(),
        ) {
            Ok(h) => {
                // Re-lock and use entry().or_insert(): if a concurrent caller
                // beat us to insert(), `h` is dropped here (stopping the
                // duplicate OS-level watcher) and the existing entry is kept.
                self.project_watchers
                    .lock()
                    .unwrap()
                    .entry(project_id)
                    .or_insert(h);
            }
            Err(e) => {
                tracing::warn!(
                    project_id = %project_id,
                    error = %e,
                    "directive watcher spawn failed"
                );
            }
        }
    }

    /// Called from three sites: mcp.status_report (kind=complete|blocked) side-effect,
    /// PTY exit handler (status=ended|error), and recovery startup pass.
    /// Idempotent: writing the same terminal_status twice is a no-op.
    pub async fn on_child_terminal(&self, child_id: Uuid, status: &str) -> Result<()> {
        // Phase 1: all synchronous DB work in a block so no non-Send types
        // (Transaction, Statement, PooledConnection) are held across any
        // `.await` point in phase 2. The block returns only Send data.
        let newly_resolved: Vec<(Uuid, Vec<Uuid>)> = {
            let mut conn = self.db.pool().get()?;
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

            // 1. Stamp child's terminal_status (first writer wins).
            tx.execute(
                "UPDATE sessions SET terminal_status = COALESCE(terminal_status, ?1) WHERE id = ?2",
                rusqlite::params![status, child_id.to_string()],
            )?;

            // 2. Find every parent observing this child.
            let mut stmt = tx.prepare(
                "SELECT id, waiting_on FROM sessions WHERE waiting_on LIKE ?1"
            )?;
            let pattern = format!("%{}%", child_id);
            let parents: Vec<(String, String)> = stmt
                .query_map([pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);

            let mut newly_resolved: Vec<(Uuid, Vec<Uuid>)> = Vec::new();

            for (parent_id_str, waiting_json) in parents {
                let parent_id = Uuid::parse_str(&parent_id_str)
                    .map_err(|e| Error::Config(format!("bad parent uuid: {e}")))?;
                let mut arr: Vec<String> = serde_json::from_str(&waiting_json).unwrap_or_default();
                let before = arr.len();
                arr.retain(|s| s != &child_id.to_string());
                if arr.len() == before { continue; }

                if arr.is_empty() {
                    // All children resolved — clear waiting_on, populate pending_rejoin.
                    // Query ALL children of this parent (not just those still in
                    // waiting_on) so earlier children removed from the array in
                    // prior on_child_terminal calls are still included in the
                    // rejoin markdown.
                    let mut child_stmt = tx.prepare(
                        "SELECT id FROM sessions WHERE parent_session_id = ?1"
                    )?;
                    let all_children: Vec<Uuid> = child_stmt
                        .query_map([&parent_id_str], |r| r.get::<_, String>(0))?
                        .filter_map(|r| r.ok())
                        .filter_map(|s| Uuid::parse_str(&s).ok())
                        .collect();
                    drop(child_stmt);
                    tx.execute(
                        "UPDATE sessions SET waiting_on = NULL WHERE id = ?1",
                        [parent_id_str],
                    )?;
                    newly_resolved.push((parent_id, all_children));
                } else {
                    tx.execute(
                        "UPDATE sessions SET waiting_on = ?1 WHERE id = ?2",
                        rusqlite::params![serde_json::to_string(&arr).unwrap(), parent_id_str],
                    )?;
                }
            }
            tx.commit()?;
            // conn, tx, stmt all drop here — before any await.
            newly_resolved
        };

        // Phase 2: async work. No non-Send DB handles in scope.
        // 3. Outside the transaction: build markdown, write pending_rejoin, fire events,
        //    nudge PTY. Each parent independent.
        for (parent_id, child_set) in newly_resolved {
            let md = self.build_rejoin_markdown(parent_id, &child_set).await?;
            {
                let conn = self.db.pool().get()?;
                conn.execute(
                    "UPDATE sessions SET pending_rejoin = ?1 WHERE id = ?2",
                    rusqlite::params![md, parent_id.to_string()],
                )?;
                // conn drops here, before the PTY await below.
            }
            let _ = crate::state::apply_event_with_bus(
                &self.db, parent_id, &self.event_bus,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind: "session.children_resolved".into(),
                    payload: serde_json::json!({
                        "children": child_set.iter().map(|u| u.to_string()).collect::<Vec<_>>()
                    }),
                    dedup_key: Some(format!("children-resolved:{parent_id}")),
                },
            );
            // Wake the parent's PTY if it is alive.
            let marker = format!("[GALVOLITE:REJOIN {}]\n", parent_id);
            let running_handle = {
                let running = self.running.lock().unwrap();
                running.get(&parent_id).map(|rs| rs.handle.clone())
            };
            if let Some(h) = running_handle {
                let _ = h.write_str(&marker).await;
            }
        }
        Ok(())
    }

    pub async fn build_rejoin_markdown(&self, parent_id: Uuid, children: &[Uuid]) -> Result<String> {
        let conn = self.db.pool().get()?;
        let mut lines = Vec::with_capacity(children.len() + 2);
        lines.push("Children resolved. Resume.".to_string());
        lines.push(String::new());
        for cid in children {
            let row: Option<(String, Option<String>, Option<String>)> = conn.query_row(
                "SELECT role, terminal_status, handoff_path FROM sessions \
                 WHERE id = ?1 AND parent_session_id = ?2",
                rusqlite::params![cid.to_string(), parent_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ).optional()?;
            let Some((role, status, handoff)) = row else { continue };
            let short = &cid.simple().to_string()[..8];
            let status_s = status.as_deref().unwrap_or("unknown");
            let handoff_s = handoff.as_deref().unwrap_or("none");
            lines.push(format!("- {short} role={role} status={status_s} handoff={handoff_s}"));
        }
        Ok(lines.join("\n"))
    }

    /// Restart-time scan: any parent with a non-NULL `waiting_on` whose listed
    /// children already have terminal_status set should resolve immediately.
    ///
    /// Idempotent: replays `on_child_terminal` for already-terminal children;
    /// `on_child_terminal` is itself idempotent so repeat calls are safe.

    /// Find the most recent non-terminal top-level session for a project.
    /// "Orchestrator" here means a session with no parent — the user-facing
    /// root that may spawn children. The DB schema does not store
    /// "orchestrator" as a role (that's a runtime profile signaled via
    /// `[ROLE:orchestrator]`); the top-level-session heuristic is what
    /// `execute_plan` needs to attach children to.
    pub fn find_orchestrator_for_project(&self, project_id: Uuid) -> Option<crate::db::Session> {
        self.db.list_sessions().ok()?.into_iter()
            .filter(|s| s.project_id == project_id
                        && s.parent_session_id.is_none()
                        && s.ended_at.is_none())
            .max_by_key(|s| s.created_at)
    }

    /// Append a block to `pending_overseer_recs` for a session. Concatenates
    /// with `\n\n---\n\n` if there is existing content (append-only — never
    /// silently overwrites). Used by the recommendations `act` endpoint when
    /// the operator chooses "Make" to inject Overseer-surfaced recs into the
    /// orchestrator's next handoff.
    pub fn append_pending_overseer_recs(&self, session_id: Uuid, block: &str) -> Result<()> {
        let conn = self.db.pool().get()?;
        let prior: Option<String> = conn.query_row(
            "SELECT pending_overseer_recs FROM sessions WHERE id = ?1",
            [session_id.to_string()], |r| r.get(0),
        ).ok().flatten();
        let merged = match prior {
            Some(p) if !p.is_empty() => format!("{p}\n\n---\n\n{block}"),
            _ => block.to_string(),
        };
        conn.execute(
            "UPDATE sessions SET pending_overseer_recs = ?1 WHERE id = ?2",
            rusqlite::params![merged, session_id.to_string()],
        )?;
        Ok(())
    }

    pub async fn recover_pending_rejoins(&self) -> Result<()> {
        let parents: Vec<(String, String)> = {
            let conn = self.db.pool().get()?;
            let mut stmt = conn.prepare(
                "SELECT id, waiting_on FROM sessions WHERE waiting_on IS NOT NULL"
            )?;
            let collected: Vec<(String, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };

        for (_pid, waiting_json) in parents {
            let arr: Vec<String> = serde_json::from_str(&waiting_json).unwrap_or_default();
            for child_str in arr {
                let cid = match Uuid::parse_str(&child_str) { Ok(u) => u, Err(_) => continue };
                let term: Option<String> = {
                    let conn = self.db.pool().get()?;
                    conn.query_row(
                        "SELECT terminal_status FROM sessions WHERE id = ?1",
                        [child_str.as_str()],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten()
                };
                if let Some(status) = term {
                    self.on_child_terminal(cid, &status).await?;
                }
            }
        }
        Ok(())
    }
}

/// Route a spawn-failure rollback through `apply_event` so it lands as a
/// `system.session_orphaned` event in the events log and an `Error`
/// terminal state on the session — atomically, in one transaction.
///
/// All errors are swallowed; the caller is already returning the original
/// failure and we don't want to mask it with a database error.
fn orphan_session(
    db: &Db,
    bus: &crate::api::events::EventBroadcaster,
    session_id: Uuid,
    reason: &'static str,
) {
    let _ = crate::state::apply_event_with_bus(
        db,
        session_id,
        bus,
        crate::state::EventInput {
            source: crate::state::Source::System,
            kind: "system.session_orphaned".into(),
            payload: serde_json::json!({ "reason": reason }),
            dedup_key: Some(format!("session-orphaned:{session_id}")),
        },
    );
}

/// Restart-recovery scan (spec §11.2). For every session in a non-terminal
/// state, check whether the recorded PTY pid is still alive. Alive: replay
/// the last 20 events as one `SessionResumed` frame and append a
/// `system.recovered` marker so consumers can see where the restart
/// boundary lies. Dead: handled by Task 11 (orphan + auto-handoff).
///
/// Errors from individual sessions are swallowed — recovery is best-effort
/// and one bad session must not block the others.
fn recover_non_terminal_sessions(db: &Db, bus: &EventBroadcaster) -> Result<()> {
    let rows = db.list_non_terminal_sessions()?;
    for (sid, pid) in rows {
        let alive = pid_is_alive(pid);
        if alive {
            // Alive PTY: replay last 20 events as one SessionResumed frame
            // (we cannot rebind the read loop — that requires the original
            // master fd). Mark a recovery boundary in the events log.
            let recent = db.read_recent_events(sid, 20).unwrap_or_default();
            bus.publish(DomainEvent::SessionResumed {
                session_id: sid,
                last_events: serde_json::Value::Array(recent),
            });
            if let Err(e) = crate::state::apply_event_with_bus(
                db,
                sid,
                bus,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind: "system.recovered".into(),
                    payload: serde_json::json!({ "pid": pid, "alive": true }),
                    // ms-precision so two restarts in the same wall-clock second
                    // don't dedup-collide and silently drop the second marker.
                    dedup_key: Some(format!(
                        "recovered:{sid}:{}",
                        chrono::Utc::now().timestamp_millis()
                    )),
                },
            ) {
                tracing::warn!("failed to write system.recovered for {sid}: {e}");
            }
        } else {
            let recent = db.read_recent_events(sid, 20).unwrap_or_default();
            // Stamp an auto-recovery handoff so the operator has a starting
            // breadcrumb when they re-open this session.
            if let Ok(s) = db.get_session(sid) {
                let dir = HandoffDir::new(
                    std::path::Path::new(&s.worktree_path).join(".galvolite/handoffs"),
                );
                match crate::handoff::write_auto_recovery_handoff(&dir, sid, &recent) {
                    Ok(p) => {
                        if let Err(e) = db.set_session_handoff_path(sid, &p.to_string_lossy()) {
                            tracing::warn!("failed to stamp handoff_path for {sid}: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("auto-recovery handoff write failed for {sid}: {e}")
                    }
                }
            }
            if let Err(e) = crate::state::apply_event_with_bus(
                db,
                sid,
                bus,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind: "system.session_orphaned".into(),
                    payload: serde_json::json!({ "reason": "pty_dead_at_restart", "pid": pid }),
                    dedup_key: Some(format!("session-orphaned:{sid}")),
                },
            ) {
                tracing::warn!("failed to write system.session_orphaned for {sid}: {e}");
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn pid_is_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    // SIGNAL 0: reachability check, no-op delivery.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
#[cfg(not(unix))]
fn pid_is_alive(_pid: i64) -> bool {
    false
}

/// Validate and canonicalize a project path.
///
/// Centralized so both the CLI (`add-project`) and the HTTP API
/// (`POST /api/projects`) apply the same rules. Returns the canonical
/// path as a `String` (matching `NewProject::path`'s type).
fn validate_and_canonicalize_project_path(raw: &str) -> Result<String> {
    if raw.is_empty() {
        return Err(Error::InvalidPath("path is empty".into()));
    }
    let p = std::path::Path::new(raw);
    if !p.is_absolute() {
        return Err(Error::InvalidPath(format!(
            "path must be absolute: {raw}"
        )));
    }
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(Error::InvalidPath(format!(
            "path must not contain `..` components: {raw}"
        )));
    }
    let canonical = p
        .canonicalize()
        .map_err(|e| Error::InvalidPath(format!("resolve {raw}: {e}")))?;
    Ok(canonical.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_mgr() -> (Arc<SessionManager>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Db::open(dir.path().join("test.sqlite")).unwrap();
        let mgr = Arc::new(SessionManager::new(db, dir.path().to_path_buf()));
        (mgr, dir)
    }

    #[test]
    fn add_project_canonicalizes_valid_absolute_path() {
        let (mgr, _data) = open_mgr();
        let proj_dir = tempfile::tempdir().unwrap();
        let raw = proj_dir.path().to_string_lossy().into_owned();

        let p = mgr
            .add_project(crate::db::NewProject {
                name: "ok".into(),
                path: raw.clone(),
                remote_url: None,
            })
            .expect("valid absolute existing path should insert");

        let expected = std::fs::canonicalize(proj_dir.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(p.path, expected);
    }

    #[test]
    fn add_project_rejects_relative_path() {
        let (mgr, _data) = open_mgr();
        let err = mgr
            .add_project(crate::db::NewProject {
                name: "rel".into(),
                path: "relative/path".into(),
                remote_url: None,
            })
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidPath(_)),
            "expected InvalidPath, got {err:?}"
        );
    }

    #[test]
    fn add_project_rejects_parent_dir_component() {
        let (mgr, _data) = open_mgr();
        let err = mgr
            .add_project(crate::db::NewProject {
                name: "dotdot".into(),
                path: "/tmp/../etc".into(),
                remote_url: None,
            })
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidPath(_)),
            "expected InvalidPath, got {err:?}"
        );
    }

    #[test]
    fn add_project_rejects_nonexistent_path() {
        let (mgr, _data) = open_mgr();
        let err = mgr
            .add_project(crate::db::NewProject {
                name: "ghost".into(),
                path: "/this/path/does/not/exist/galvolite-test".into(),
                remote_url: None,
            })
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidPath(_)),
            "expected InvalidPath, got {err:?}"
        );
    }

    #[test]
    fn add_project_rejects_empty_path() {
        let (mgr, _data) = open_mgr();
        let err = mgr
            .add_project(crate::db::NewProject {
                name: "empty".into(),
                path: String::new(),
                remote_url: None,
            })
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidPath(_)),
            "expected InvalidPath, got {err:?}"
        );
    }
}
