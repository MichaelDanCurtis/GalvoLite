//! Session lifecycle manager.
//!
//! Extracted from the `cmd_start_session` monolith in main.rs.
//! Owns the Db, tracks running sessions in-memory, provides spawn/list/get/
//! kill/handoff/send/subscribe methods for the API and CLI.

use crate::claude_md::{implementation_profile, InjectionPrompt};
use crate::db::{Db, NewSession, Session, SessionState};
use crate::error::{Error, Result};
use crate::handoff::HandoffDir;
use crate::pty::{CycleSlot, PtyHandle, PtyHandleShared, PtyOpts, SignalPump};
use crate::ritual::{PtySideEffects, RitualDriver};
use crate::signals::Signal;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::broadcast;
use uuid::Uuid;

struct RunningSession {
    driver: Arc<RitualDriver>,
    handle: PtyHandleShared,
    output_tx: broadcast::Sender<String>,
    /// Current cycle id for this running session. Shared with the
    /// [`SignalPump`] recording task so rotation on STATUS:COMPLETE
    /// immediately re-attributes signals to the new cycle.
    cycle_id: CycleSlot,
}

pub struct SessionManager {
    db: Db,
    data_dir: PathBuf,
    running: StdMutex<HashMap<Uuid, RunningSession>>,
}

impl SessionManager {
    pub fn new(db: Db, data_dir: PathBuf) -> Self {
        Self {
            db,
            data_dir,
            running: StdMutex::new(HashMap::new()),
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
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
        std::fs::create_dir_all(&galvolite_dir)
            .map_err(|e| Error::io(&galvolite_dir, e))?;
        let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
        std::fs::write(&claude_md_path, implementation_profile())
            .map_err(|e| Error::io(&claude_md_path, e))?;
        let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
        crate::handoff::ensure_session_dir(&handoff_dir, session.id)?;

        let cycle_id = self.db.start_cycle(session.id)?;

        let pty = PtyHandle::spawn(PtyOpts {
            cwd: worktree,
            command: claude_cmd,
            args: vec![],
            env: vec![],
        })?;
        self.db.update_session_pid(session.id, pty.pid as i64)?;

        let cycle_id_slot: CycleSlot = Arc::new(StdMutex::new(Some(cycle_id)));
        let pump = SignalPump::start(pty, self.db.clone(), session.id, cycle_id_slot.clone());
        let handle = pump.handle();
        let output_tx = pump.output_sender();

        let fx = Arc::new(PtySideEffects::new(handle.clone()));
        let driver = Arc::new(RitualDriver::new(fx));
        self.db.update_session_state(session.id, SessionState::Working)?;

        let injection = InjectionPrompt {
            task_summary: &task,
            session_id: &session.id.to_string(),
            claude_md_path: &claude_md_path.to_string_lossy(),
        };
        handle
            .write_str(&format!("{}\n", injection.render()))
            .await?;

        // Store in running map
        {
            let mut running = self.running.lock().unwrap();
            running.insert(
                session.id,
                RunningSession {
                    driver: driver.clone(),
                    handle: handle.clone(),
                    output_tx,
                    cycle_id: cycle_id_slot.clone(),
                },
            );
        }

        // Spawn background task to drive signals and clean up on exit
        let mgr = Arc::clone(self);
        let session_id = session.id;
        let driver_for_sig = driver.clone();
        let handoff_dir_bg = handoff_dir.clone();
        let cycle_id_slot_bg = cycle_id_slot.clone();
        tokio::spawn(async move {
            let mut pump = pump;
            while let Some(sig) = pump.next_signal().await {
                if matches!(sig, Signal::HandoffReady) {
                    if let Ok(Some(p)) =
                        crate::handoff::latest_handoff_for(&handoff_dir_bg, session_id)
                    {
                        driver_for_sig
                            .set_latest_handoff_path(p.to_string_lossy().into())
                            .await;
                    }
                }
                let is_status_complete = matches!(sig, Signal::StatusComplete);
                driver_for_sig.on_signal(sig).await;

                if is_status_complete {
                    // End current cycle with latest handoff (if any), then start a new one.
                    // Conditional end avoids clobbering a kill-path cycle close.
                    let prev_cycle = cycle_id_slot_bg.lock().unwrap().take();
                    if let Some(prev) = prev_cycle {
                        let handoff_path =
                            crate::handoff::latest_handoff_for(&handoff_dir_bg, session_id)
                                .ok()
                                .flatten()
                                .map(|p| p.to_string_lossy().into_owned());
                        let _ = mgr.db.end_cycle_if_unended(prev, handoff_path.as_deref());
                    }
                    if let Ok(new_cycle) = mgr.db.start_cycle(session_id) {
                        *cycle_id_slot_bg.lock().unwrap() = Some(new_cycle);
                    }
                }
            }

            // Session process exited — end current cycle, then end session.
            // Conditional writes guarantee we never overwrite a kill-path
            // Killed state with a Complete state (TOCTOU-free).
            let prev_cycle = cycle_id_slot_bg.lock().unwrap().take();
            if let Some(prev) = prev_cycle {
                let handoff_path = crate::handoff::latest_handoff_for(&handoff_dir_bg, session_id)
                    .ok()
                    .flatten()
                    .map(|p| p.to_string_lossy().into_owned());
                let _ = mgr.db.end_cycle_if_unended(prev, handoff_path.as_deref());
            }
            let _ = mgr.db.end_session_if_unended(session_id, SessionState::Complete);
            let mut running = mgr.running.lock().unwrap();
            running.remove(&session_id);
        });

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

    pub fn add_project(&self, new: crate::db::NewProject) -> Result<crate::db::Project> {
        self.db.insert_project(new)
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
                let _ = self.db.end_cycle_if_unended(cycle, None);
            }
        }
        // Conditional write: if the session already ended naturally
        // (e.g. Complete), we must not clobber that terminal state.
        let killed = self.db.end_session_if_unended(id, SessionState::Killed)?;
        if !was_running && !killed {
            // Not in the running map, and the DB says it was already ended.
            // Report this rather than silently succeeding.
            return Err(Error::SessionNotRunning(id));
        }
        Ok(())
    }

    pub fn subscribe_output(&self, id: Uuid) -> Result<broadcast::Receiver<String>> {
        let running = self.running.lock().unwrap();
        let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
        Ok(rs.output_tx.subscribe())
    }

    pub fn get_signals(&self, id: Uuid) -> Result<Vec<crate::db::SignalRow>> {
        self.db.list_signals_for_session(id)
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
}
