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
        Command::AddProject { path, name } => cmd_add_project(&db, path, name),
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
fn cmd_add_project(db: &galvolite::db::Db, path: std::path::PathBuf, name: Option<String>) -> Result<()> {
    let abs = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("resolve {}: {e}", path.display()))?;
    let name = name.unwrap_or_else(|| {
        abs.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into())
    });
    let p = db.insert_project(galvolite::db::NewProject {
        name: name.clone(),
        path: abs.to_string_lossy().into_owned(),
        remote_url: None,
    })?;
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

async fn cmd_start_session(_db: &galvolite::db::Db, _project: uuid::Uuid, _task: String, _role: String, _claude_cmd: String) -> Result<()> {
    anyhow::bail!("not yet implemented — task 19")
}
fn cmd_list_sessions(_db: &galvolite::db::Db) -> Result<()> {
    anyhow::bail!("not yet implemented — task 19")
}
fn cmd_status(_db: &galvolite::db::Db, _session: uuid::Uuid) -> Result<()> {
    anyhow::bail!("not yet implemented — task 19")
}
async fn cmd_send(_db: &galvolite::db::Db, _session: uuid::Uuid, _text: String, _newline: bool) -> Result<()> {
    anyhow::bail!("not yet implemented — task 20")
}
async fn cmd_handoff(_db: &galvolite::db::Db, _session: uuid::Uuid) -> Result<()> {
    anyhow::bail!("not yet implemented — task 21")
}
async fn cmd_kill(_db: &galvolite::db::Db, _session: uuid::Uuid) -> Result<()> {
    anyhow::bail!("not yet implemented — task 21")
}
