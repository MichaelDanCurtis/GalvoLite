use anyhow::Result;
use clap::Parser;
use galvolite::cli::{Cli, Command};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;

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
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    mgr.start_terminal_reactor();
    galvolite::plan_executor::start_phase_reactor(Arc::clone(&mgr));
    mgr.start_directive_watchers();
    let _ = mgr.recover_pending_rejoins().await;

    match cli.cmd {
        Command::AddProject { path, name } => cmd_add_project(&mgr, path, name),
        Command::Projects => cmd_list_projects(&mgr),
        Command::StartSession {
            project,
            task,
            role,
            claude_cmd,
        } => cmd_start_session(&mgr, project, task, role, claude_cmd).await,
        Command::Sessions => cmd_list_sessions(&mgr),
        Command::Status { session } => cmd_status(&db, session),
        Command::Send {
            session,
            text,
            newline,
        } => cmd_send(&mgr, session, text, newline).await,
        Command::Handoff { session } => cmd_handoff(&mgr, session).await,
        Command::Kill { session } => cmd_kill(&mgr, session).await,
        Command::Serve { port, host } => cmd_serve(mgr, host, port).await,
    }
}

fn resolve_data_dir(override_dir: Option<std::path::PathBuf>) -> Result<std::path::PathBuf> {
    if let Some(d) = override_dir {
        return Ok(d);
    }
    let base =
        dirs::data_dir().ok_or_else(|| anyhow::anyhow!("could not determine OS data dir"))?;
    Ok(base.join("GalvoLite"))
}

fn cmd_add_project(
    mgr: &SessionManager,
    path: std::path::PathBuf,
    name: Option<String>,
) -> Result<()> {
    let name = name.unwrap_or_else(|| {
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into())
    });
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

fn cmd_list_projects(mgr: &SessionManager) -> Result<()> {
    let projects = mgr.list_projects().map_err(|e| anyhow::anyhow!("{e}"))?;
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
    mgr: &Arc<SessionManager>,
    project_id: uuid::Uuid,
    task: String,
    role: String,
    claude_cmd: String,
) -> Result<()> {
    let session = mgr
        .spawn_session(project_id, task, role, claude_cmd, vec![])
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("Session {} started", session.id);
    println!("Press Ctrl-C to detach. (Send/handoff/kill across processes require `serve`.)");

    tokio::signal::ctrl_c().await?;
    println!("\nDetached from session {}.", session.id);
    Ok(())
}

fn cmd_list_sessions(mgr: &SessionManager) -> Result<()> {
    let ss = mgr.list_sessions().map_err(|e| anyhow::anyhow!("{e}"))?;
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
    Ok(())
}

async fn cmd_send(
    mgr: &SessionManager,
    session_id: uuid::Uuid,
    text: String,
    newline: bool,
) -> Result<()> {
    mgr.send_to_session(session_id, text, newline)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("sent");
    Ok(())
}

async fn cmd_handoff(mgr: &SessionManager, session_id: uuid::Uuid) -> Result<()> {
    mgr.trigger_handoff(session_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("handoff initiated");
    Ok(())
}

async fn cmd_kill(mgr: &SessionManager, session_id: uuid::Uuid) -> Result<()> {
    mgr.kill_session(session_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("killed");
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
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::warn!(error = ?e, "failed to install SIGTERM handler; relying on SIGINT only");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn cmd_serve(mgr: Arc<SessionManager>, host: String, port: u16) -> Result<()> {
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
    // mutex so we don't hold the lock across the async kill_session calls.
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
