use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "arbor",
    about = "Arbor CLI — manage worktrees, terminals, processes, and tasks"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,

    /// Output as JSON instead of human-readable text
    #[arg(long, global = true)]
    pub(crate) json: bool,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Show daemon health and version
    Health,

    /// Repository operations
    #[command(subcommand)]
    Repos(ReposCommand),

    /// Worktree operations
    #[command(subcommand)]
    Worktrees(WorktreesCommand),

    /// Terminal session operations
    #[command(subcommand)]
    Terminals(TerminalsCommand),

    /// AI agent activity
    #[command(subcommand)]
    Agents(AgentsCommand),

    /// Managed process operations
    #[command(subcommand)]
    Processes(ProcessesCommand),

    /// Scheduled task operations
    #[command(subcommand)]
    Tasks(TasksCommand),
}

// ── Repository commands ──────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum ReposCommand {
    /// List all known repositories
    List,
}

// ── Worktree commands ────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum WorktreesCommand {
    /// List worktrees
    List {
        /// Filter by repository root path
        #[arg(long)]
        repo_root: Option<String>,
    },
    /// Create a new worktree
    Create {
        /// Repository root path
        #[arg(long)]
        repo_root: String,
        /// Worktree path
        #[arg(long)]
        path: String,
        /// Branch name
        #[arg(long)]
        branch: Option<String>,
        /// Create in detached HEAD mode
        #[arg(long)]
        detach: bool,
        /// Force creation
        #[arg(long)]
        force: bool,
    },
    /// Delete a worktree
    Delete {
        /// Repository root path
        #[arg(long)]
        repo_root: String,
        /// Worktree path
        #[arg(long)]
        path: String,
        /// Also delete the branch
        #[arg(long)]
        delete_branch: bool,
        /// Force deletion
        #[arg(long)]
        force: bool,
    },
    /// List changed files in a worktree
    Changes {
        /// Worktree path
        #[arg(long)]
        path: String,
    },
    /// Create a commit in a worktree
    Commit {
        /// Worktree path
        #[arg(long)]
        path: String,
        /// Commit message (auto-generated if omitted)
        #[arg(long, short)]
        message: Option<String>,
    },
    /// Push the current branch to origin
    Push {
        /// Worktree path
        #[arg(long)]
        path: String,
    },
}

// ── Terminal commands ────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum TerminalsCommand {
    /// List all terminal sessions
    List,
    /// Create or attach to a terminal session
    Create {
        /// Working directory
        #[arg(long)]
        cwd: String,
        /// Session ID
        #[arg(long)]
        session_id: Option<String>,
        /// Shell to use
        #[arg(long)]
        shell: Option<String>,
        /// Terminal columns
        #[arg(long)]
        cols: Option<u16>,
        /// Terminal rows
        #[arg(long)]
        rows: Option<u16>,
        /// Session title
        #[arg(long)]
        title: Option<String>,
        /// Command to run instead of shell
        #[arg(long)]
        command: Option<String>,
    },
    /// Read terminal output
    Read {
        /// Session ID
        session_id: String,
        /// Maximum lines to return
        #[arg(long)]
        max_lines: Option<usize>,
    },
    /// Write input to a terminal
    Write {
        /// Session ID
        session_id: String,
        /// Data to write
        data: String,
    },
    /// Resize a terminal
    Resize {
        /// Session ID
        session_id: String,
        /// Columns
        #[arg(long)]
        cols: u16,
        /// Rows
        #[arg(long)]
        rows: u16,
    },
    /// Send a signal to a terminal
    Signal {
        /// Session ID
        session_id: String,
        /// Signal name (e.g. SIGINT, SIGTERM, SIGKILL)
        signal: String,
    },
    /// Detach from a terminal without killing it
    Detach {
        /// Session ID
        session_id: String,
    },
    /// Kill a terminal session
    Kill {
        /// Session ID
        session_id: String,
    },
}

// ── Agent commands ───────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum AgentsCommand {
    /// List active agent sessions
    List,
}

// ── Process commands ─────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum ProcessesCommand {
    /// List all managed processes
    List,
    /// Start all auto-start processes
    StartAll,
    /// Stop all running processes
    StopAll,
    /// Start a process by name
    Start {
        /// Process name
        name: String,
    },
    /// Stop a process by name
    Stop {
        /// Process name
        name: String,
    },
    /// Restart a process by name
    Restart {
        /// Process name
        name: String,
    },
}

// ── Task commands ────────────────────────────────────────────────────

#[derive(Subcommand)]
pub(crate) enum TasksCommand {
    /// List all scheduled tasks
    List,
    /// Manually trigger a task
    Run {
        /// Task name
        name: String,
    },
    /// Show execution history for a task
    History {
        /// Task name
        name: String,
    },
}
