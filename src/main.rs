use anyhow::Result;
use clap::Parser;
use galvolite::cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("galvolite=info")),
        )
        .init();

    let cli = Cli::parse();
    let data_dir = resolve_data_dir(cli.data_dir.clone())?;
    std::fs::create_dir_all(&data_dir)?;

    let db = galvolite::db::Db::open(data_dir.join("galvolite.sqlite"))?;

    match cli.cmd {
        Command::AddProject { path, name } => cmd_add_project(&db, data_dir, path, name),
        Command::Projects => cmd_list_projects(&db),
        Command::StartSession { project, task, role, claude_cmd } => {
            cmd_start_session(&db, project, task, role, claude_cmd).await
        }
        Command::Sessions => cmd_list_sessions(&db),
        Command::Status { session } => cmd_status(&db, session),
        Command::Send { session, text, newline } => {
            cmd_send(&db, session, text, newline).await
        }
        Command::Handoff { session } => cmd_handoff(&db, session).await,
        Command::Kill { session } => cmd_kill(&db, session).await,
        Command::Serve { port, host } => cmd_serve(&db, data_dir, host, port).await,
    }
}

fn resolve_data_dir(override_dir: Option<std::path::PathBuf>) -> Result<std::path::PathBuf> {
    if let Some(d) = override_dir {
        return Ok(d);
    }
    let base = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine OS data dir"))?;
    Ok(base.join("GalvoLite"))
}

// Task 18 implements these two; rest are stubs for tasks 19-21.
fn cmd_add_project(
    db: &galvolite::db::Db,
    data_dir: std::path::PathBuf,
    path: std::path::PathBuf,
    name: Option<String>,
) -> Result<()> {
    use galvolite::session_manager::SessionManager;
    // Default the project name to the final path component (best effort —
    // canonicalization happens inside SessionManager::add_project, which is
    // the source of truth for path validation). If the path is unusable, we
    // still let SessionManager reject it with a precise error.
    let name = name.unwrap_or_else(|| {
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into())
    });
    let mgr = SessionManager::new(db.clone(), data_dir);
    let p = mgr
        .add_project(galvolite::db::NewProject {
            name,
            path: path.to_string_lossy().into_owned(),
            remote_url: None,
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("Added project {} ({}) at {}", p.name, p.id, p.path);
    Ok(())
}

fn cmd_list_projects(db: &galvolite::db::Db) -> Result<()> {
    let projects = db.list_projects()?;
    if projects.is_empty() {
        println!("(no projects registered)");
        return Ok(());
    }
    for p in projects {
        println!("{}  {}  {}", p.id, p.name, p.path);
    }
    Ok(())
}

async fn cmd_start_session(
    db: &galvolite::db::Db,
    project_id: uuid::Uuid,
    task: String,
    role: String,
    claude_cmd: String,
) -> Result<()> {
    use galvolite::claude_md::{implementation_profile, InjectionPrompt};
    use galvolite::db::NewSession;
    use galvolite::handoff::HandoffDir;
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
    use galvolite::ritual::{PtySideEffects, RitualDriver};
    use galvolite::session_registry::{listen, socket_path, ControlMessage, ControlResponse};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let project = db.get_project(project_id)?;
    let session = db.insert_session(NewSession {
        project_id: project.id,
        role: role.clone(),
        branch: None,
        worktree_path: project.path.clone(),
        task_summary: task.clone(),
        parent_session_id: None,
    })?;

    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session.id);
    let listener = listen(&sock).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let worktree = std::path::PathBuf::from(&project.path);
    let galvolite_dir = worktree.join(".galvolite");
    std::fs::create_dir_all(&galvolite_dir)?;
    let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
    std::fs::write(&claude_md_path, implementation_profile())?;
    let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
    galvolite::handoff::ensure_session_dir(&handoff_dir, session.id)?;

    tracing::info!(%session.id, "spawning claude");
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: worktree.clone(),
        command: claude_cmd,
        args: vec![],
        env: vec![],
    }).map_err(|e| anyhow::anyhow!("{e}"))?;
    db.update_session_pid(session.id, pty.pid as i64)?;
    let cycle_id = db.start_cycle(session.id)?;
    // main.rs legacy path: no rotation, so the slot is fixed at the initial cycle_id.
    let cycle_slot: galvolite::pty::CycleSlot =
        std::sync::Arc::new(std::sync::Mutex::new(Some(cycle_id)));
    let mut pump = SignalPump::start(pty, db.clone(), session.id, cycle_slot);

    let fx = Arc::new(PtySideEffects::new(pump.handle()));
    let driver = Arc::new(RitualDriver::new(fx));
    db.update_session_state(session.id, galvolite::db::SessionState::Working)?;

    let injection = InjectionPrompt {
        task_summary: &task,
        session_id: &session.id.to_string(),
        claude_md_path: &claude_md_path.to_string_lossy(),
    };
    pump.handle().write_str(&format!("{}\n", injection.render())).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Session {} started. Socket: {}", session.id, sock.display());

    let session_id = session.id;
    let driver_for_sig = driver.clone();
    let db_for_sig = db.clone();
    let handoff_dir_for_sig = handoff_dir.clone();
    let signal_task = tokio::spawn(async move {
        while let Some(sig) = pump.next_signal().await {
            if matches!(sig, galvolite::signals::Signal::HandoffReady) {
                if let Ok(Some(p)) =
                    galvolite::handoff::latest_handoff_for(&handoff_dir_for_sig, session_id)
                {
                    driver_for_sig.set_latest_handoff_path(p.to_string_lossy().into()).await;
                }
            }
            driver_for_sig.on_signal(sig).await;
        }
        tracing::info!("signal stream ended");
        let _ = db_for_sig.end_session(session_id, galvolite::db::SessionState::Complete);
    });

    let driver_for_ctrl = driver.clone();
    let db_for_ctrl = db.clone();
    let control_task = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => { tracing::error!(?e, "accept"); continue; }
            };
            let driver = driver_for_ctrl.clone();
            let db = db_for_ctrl.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut br = BufReader::new(reader);
                let mut line = String::new();
                if br.read_line(&mut line).await.is_err() { return; }
                let msg: ControlMessage = match serde_json::from_str(line.trim()) {
                    Ok(m) => m,
                    Err(e) => {
                        let resp = serde_json::to_string(&ControlResponse::Err(format!("{e}"))).unwrap_or_default();
                        let _ = writer.write_all(resp.as_bytes()).await;
                        let _ = writer.write_all(b"\n").await;
                        return;
                    }
                };
                let resp = handle_control(&db, &driver, session_id, msg).await;
                let s = serde_json::to_string(&resp).unwrap_or_default();
                let _ = writer.write_all(s.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
            });
        }
    });

    tokio::select! {
        _ = signal_task => {},
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received ctrl-c, shutting down");
            let _ = db.end_session(session.id, galvolite::db::SessionState::Killed);
        }
    }
    control_task.abort();
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

fn cmd_list_sessions(db: &galvolite::db::Db) -> Result<()> {
    let ss = db.list_sessions()?;
    if ss.is_empty() {
        println!("(no active sessions)");
        return Ok(());
    }
    for s in ss {
        println!("{}  state={}  task={}", s.id, s.state, s.task_summary);
    }
    Ok(())
}

fn cmd_status(db: &galvolite::db::Db, session: uuid::Uuid) -> Result<()> {
    let s = db.get_session(session)?;
    println!("Session {}", s.id);
    println!("  Project:       {}", s.project_id);
    println!("  Role:          {}", s.role);
    println!("  State:         {}", s.state);
    println!("  Task:          {}", s.task_summary);
    println!("  Worktree:      {}", s.worktree_path);
    println!("  PID:           {:?}", s.pty_pid);
    println!("  Autopilot:     {}", s.autopilot);
    let sigs = db.list_signals_for_session(session)?;
    println!("  Signals ({}):", sigs.len());
    for sig in sigs.iter().rev().take(10).rev() {
        println!("    [{}] {}{}", sig.at.format("%H:%M:%S"), sig.kind,
            sig.payload.as_deref().map(|p| format!(" reason=\"{p}\"")).unwrap_or_default());
    }
    Ok(())
}

async fn cmd_send(
    _db: &galvolite::db::Db,
    session_id: uuid::Uuid,
    text: String,
    newline: bool,
) -> Result<()> {
    use galvolite::session_registry::{send_command, socket_path, ControlMessage, ControlResponse};
    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session_id);
    let resp = send_command(&sock, ControlMessage::Send { text, newline }).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    match resp {
        ControlResponse::Ok(msg) => println!("{msg}"),
        ControlResponse::Err(msg) => anyhow::bail!("{msg}"),
    }
    Ok(())
}

async fn cmd_handoff(_db: &galvolite::db::Db, session_id: uuid::Uuid) -> Result<()> {
    use galvolite::session_registry::{send_command, socket_path, ControlMessage, ControlResponse};
    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session_id);
    let resp = send_command(&sock, ControlMessage::Handoff).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    match resp {
        ControlResponse::Ok(msg) => println!("{msg}"),
        ControlResponse::Err(msg) => anyhow::bail!("{msg}"),
    }
    Ok(())
}

async fn cmd_kill(db: &galvolite::db::Db, session_id: uuid::Uuid) -> Result<()> {
    use galvolite::session_registry::{send_command, socket_path, ControlMessage, ControlResponse};
    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session_id);
    match send_command(&sock, ControlMessage::Kill).await {
        Ok(ControlResponse::Ok(msg)) => println!("{msg}"),
        Ok(ControlResponse::Err(msg)) => anyhow::bail!("{msg}"),
        Err(_) => {
            db.end_session(session_id, galvolite::db::SessionState::Killed)?;
            println!("session marked killed (daemon was not running)");
        }
    }
    Ok(())
}

/// Wait for SIGINT (Ctrl-C), or — on Unix — SIGTERM. Whichever fires first
/// resolves the future. Used to drive `axum::serve(...).with_graceful_shutdown`.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut s) = signal(SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn cmd_serve(
    db: &galvolite::db::Db,
    data_dir: std::path::PathBuf,
    host: String,
    port: u16,
) -> Result<()> {
    use galvolite::session_manager::SessionManager;
    use std::sync::Arc;

    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let app = galvolite::api::router(mgr.clone());

    let addr = format!("{host}:{port}");
    tracing::info!("starting API server on {addr}");
    println!("GalvoLite API server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;

    // Graceful shutdown: on SIGINT/SIGTERM, stop accepting new connections,
    // then iterate the running session map and kill each session so PTY
    // children aren't orphaned. `running_ids()` snapshots keys under the
    // mutex so we don't hold the lock across the async kill_session calls
    // (which re-lock the same map).
    let mgr_for_shutdown = mgr.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            tracing::info!("shutdown signal received, killing running sessions");
            for id in mgr_for_shutdown.running_ids() {
                if let Err(e) = mgr_for_shutdown.kill_session(id).await {
                    tracing::warn!(%id, error = ?e, "failed to kill session during shutdown");
                }
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("serve: {e}"))?;

    Ok(())
}

async fn handle_control(
    db: &galvolite::db::Db,
    driver: &std::sync::Arc<galvolite::ritual::RitualDriver>,
    session_id: uuid::Uuid,
    msg: galvolite::session_registry::ControlMessage,
) -> galvolite::session_registry::ControlResponse {
    use galvolite::session_registry::{ControlMessage as M, ControlResponse as R};
    match msg {
        M::Send { text, newline } => {
            let mut buf = text.into_bytes();
            if newline { buf.push(b'\n'); }
            driver.inject_raw(&buf).await;
            R::Ok("sent".into())
        }
        M::Handoff => {
            driver.request_handoff().await;
            R::Ok("handoff initiated".into())
        }
        M::Kill => {
            let _ = db.end_session(session_id, galvolite::db::SessionState::Killed);
            driver.abort("killed by user").await;
            R::Ok("killed".into())
        }
        M::Status => {
            let s = driver.state().await;
            R::Ok(format!("{s:?}"))
        }
    }
}
