use clap::{Parser, Subcommand};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "galvolite", version, about = "Solo continuity engine for Claude Code")]
pub struct Cli {
    /// Override the app data directory (default: OS-standard).
    #[arg(long, global = true)]
    pub data_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Register a project directory.
    AddProject {
        /// Path to the project's git repo.
        path: std::path::PathBuf,
        /// Optional human-readable name (defaults to directory name).
        #[arg(long)]
        name: Option<String>,
    },
    /// List registered projects.
    Projects,
    /// Start a new session in a project.
    StartSession {
        /// Project UUID.
        #[arg(long)]
        project: Uuid,
        /// One-line task summary.
        #[arg(long)]
        task: String,
        /// Role (implementation | brainstorm | scout | fix).
        #[arg(long, default_value = "implementation")]
        role: String,
        /// Command to spawn (default: `claude`).
        #[arg(long, default_value = "claude")]
        claude_cmd: String,
    },
    /// List active sessions.
    Sessions,
    /// Show a session's state + recent signals.
    Status {
        session: Uuid,
    },
    /// Send text to a session's stdin (no trailing newline added).
    Send {
        session: Uuid,
        /// Text to inject (use --newline to append \n).
        text: String,
        #[arg(long)]
        newline: bool,
    },
    /// Trigger the handoff ritual for a session.
    Handoff {
        session: Uuid,
    },
    /// Kill a session (sends SIGTERM and marks it killed in DB).
    Kill {
        session: Uuid,
    },
}
