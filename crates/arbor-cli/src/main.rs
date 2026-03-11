use {
    arbor_daemon_client::{
        CommitWorktreeRequest, CreateTerminalRequest, CreateWorktreeRequest, DaemonClient,
        DaemonClientError, DeleteWorktreeRequest, PushWorktreeRequest, TerminalResizeRequest,
        TerminalSignalRequest,
    },
    clap::{Parser, Subcommand},
    serde::Serialize,
    std::process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "arbor",
    about = "Arbor CLI — manage worktrees, terminals, processes, and tasks"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Output as JSON instead of human-readable text
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
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
enum ReposCommand {
    /// List all known repositories
    List,
}

// ── Worktree commands ────────────────────────────────────────────────

#[derive(Subcommand)]
enum WorktreesCommand {
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
enum TerminalsCommand {
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
enum AgentsCommand {
    /// List active agent sessions
    List,
}

// ── Process commands ─────────────────────────────────────────────────

#[derive(Subcommand)]
enum ProcessesCommand {
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
enum TasksCommand {
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

// ── Execution ────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let cli = Cli::parse();
    let client = DaemonClient::from_env();

    let result = run(&cli, &client);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cli.json {
                print_json(&serde_json::json!({ "error": error.to_string() }));
            } else {
                eprintln!("error: {error}");
            }
            ExitCode::FAILURE
        },
    }
}

fn run(cli: &Cli, client: &DaemonClient) -> Result<(), DaemonClientError> {
    match &cli.command {
        Command::Health => {
            let response = client.health()?;
            if cli.json {
                print_json(&response);
            } else {
                println!("status: {}", response.status);
                println!("version: {}", response.version);
            }
        },

        Command::Repos(cmd) => match cmd {
            ReposCommand::List => {
                let repos = client.list_repositories()?;
                if cli.json {
                    print_json(&repos);
                } else {
                    for repo in &repos {
                        let slug = repo
                            .github_repo_slug
                            .as_deref()
                            .map(|s| format!(" ({s})"))
                            .unwrap_or_default();
                        println!("{}{slug}", repo.root);
                    }
                }
            },
        },

        Command::Worktrees(cmd) => run_worktrees(cli, client, cmd)?,
        Command::Terminals(cmd) => run_terminals(cli, client, cmd)?,

        Command::Agents(cmd) => match cmd {
            AgentsCommand::List => {
                let sessions = client.list_agent_activity()?;
                if cli.json {
                    print_json(&sessions);
                } else {
                    for session in &sessions {
                        println!(
                            "{}\t{}\t{}",
                            session.cwd, session.state, session.updated_at_unix_ms
                        );
                    }
                }
            },
        },

        Command::Processes(cmd) => run_processes(cli, client, cmd)?,
        Command::Tasks(cmd) => run_tasks(cli, client, cmd)?,
    }

    Ok(())
}

fn run_worktrees(
    cli: &Cli,
    client: &DaemonClient,
    cmd: &WorktreesCommand,
) -> Result<(), DaemonClientError> {
    match cmd {
        WorktreesCommand::List { repo_root } => {
            let worktrees = client.list_worktrees(repo_root.as_deref())?;
            if cli.json {
                print_json(&worktrees);
            } else {
                for wt in &worktrees {
                    let primary = if wt.is_primary_checkout {
                        " [primary]"
                    } else {
                        ""
                    };
                    let diff = match (wt.diff_additions, wt.diff_deletions) {
                        (Some(a), Some(d)) => format!(" +{a}/-{d}"),
                        _ => String::new(),
                    };
                    println!("{}\t{}{primary}{diff}", wt.branch, wt.path);
                }
            }
        },
        WorktreesCommand::Create {
            repo_root,
            path,
            branch,
            detach,
            force,
        } => {
            let response = client.create_worktree(&CreateWorktreeRequest {
                repo_root: repo_root.clone(),
                path: path.clone(),
                branch: branch.clone(),
                detach: Some(*detach),
                force: Some(*force),
            })?;
            if cli.json {
                print_json(&response);
            } else {
                println!("{}", response.message);
            }
        },
        WorktreesCommand::Delete {
            repo_root,
            path,
            delete_branch,
            force,
        } => {
            let response = client.delete_worktree(&DeleteWorktreeRequest {
                repo_root: repo_root.clone(),
                path: path.clone(),
                delete_branch: Some(*delete_branch),
                force: Some(*force),
            })?;
            if cli.json {
                print_json(&response);
            } else {
                println!("{}", response.message);
            }
        },
        WorktreesCommand::Changes { path } => {
            let files = client.list_changed_files(path)?;
            if cli.json {
                print_json(&files);
            } else {
                for file in &files {
                    println!(
                        "{}\t{}\t+{}/-{}",
                        file.kind, file.path, file.additions, file.deletions
                    );
                }
            }
        },
        WorktreesCommand::Commit { path, message } => {
            let response = client.commit_worktree(&CommitWorktreeRequest {
                path: path.clone(),
                message: message.clone(),
            })?;
            if cli.json {
                print_json(&response);
            } else {
                println!("{}", response.message);
                if let Some(ref commit_message) = response.commit_message {
                    println!("commit: {commit_message}");
                }
            }
        },
        WorktreesCommand::Push { path } => {
            let response = client.push_worktree(&PushWorktreeRequest { path: path.clone() })?;
            if cli.json {
                print_json(&response);
            } else {
                println!("{}", response.message);
            }
        },
    }
    Ok(())
}

fn run_terminals(
    cli: &Cli,
    client: &DaemonClient,
    cmd: &TerminalsCommand,
) -> Result<(), DaemonClientError> {
    match cmd {
        TerminalsCommand::List => {
            let terminals = client.list_terminals()?;
            if cli.json {
                print_json(&terminals);
            } else {
                for t in &terminals {
                    let state = t
                        .state
                        .as_ref()
                        .map(|s| format!("{s:?}"))
                        .unwrap_or_else(|| "unknown".to_owned());
                    let title = t.title.as_deref().unwrap_or("");
                    println!("{}\t{state}\t{title}\t{}", t.session_id, t.cwd.display());
                }
            }
        },
        TerminalsCommand::Create {
            cwd,
            session_id,
            shell,
            cols,
            rows,
            title,
            command,
        } => {
            let response = client.create_terminal(&CreateTerminalRequest {
                session_id: session_id.clone().map(Into::into),
                workspace_id: None,
                cwd: cwd.clone(),
                shell: shell.clone(),
                cols: *cols,
                rows: *rows,
                title: title.clone(),
                command: command.clone(),
            })?;
            if cli.json {
                print_json(&response);
            } else {
                let new = if response.is_new_session {
                    "created"
                } else {
                    "attached"
                };
                println!("{new}: {}", response.session.session_id);
            }
        },
        TerminalsCommand::Read {
            session_id,
            max_lines,
        } => {
            let snapshot = client.read_terminal_output(session_id, *max_lines)?;
            if cli.json {
                print_json(&snapshot);
            } else {
                print!("{}", snapshot.output_tail);
            }
        },
        TerminalsCommand::Write { session_id, data } => {
            client.write_terminal_input(session_id, data.as_bytes())?;
            if !cli.json {
                println!("ok");
            }
        },
        TerminalsCommand::Resize {
            session_id,
            cols,
            rows,
        } => {
            client.resize_terminal(session_id, &TerminalResizeRequest {
                cols: *cols,
                rows: *rows,
            })?;
            if !cli.json {
                println!("ok");
            }
        },
        TerminalsCommand::Signal { session_id, signal } => {
            client.signal_terminal(session_id, &TerminalSignalRequest {
                signal: signal.clone(),
            })?;
            if !cli.json {
                println!("ok");
            }
        },
        TerminalsCommand::Detach { session_id } => {
            client.detach_terminal(session_id)?;
            if !cli.json {
                println!("ok");
            }
        },
        TerminalsCommand::Kill { session_id } => {
            client.kill_terminal(session_id)?;
            if !cli.json {
                println!("ok");
            }
        },
    }
    Ok(())
}

fn run_processes(
    cli: &Cli,
    client: &DaemonClient,
    cmd: &ProcessesCommand,
) -> Result<(), DaemonClientError> {
    match cmd {
        ProcessesCommand::List => {
            let processes = client.list_processes()?;
            if cli.json {
                print_json(&processes);
            } else {
                for p in &processes {
                    println!("{}\t{:?}\t{}", p.name, p.status, p.command);
                }
            }
        },
        ProcessesCommand::StartAll => {
            let processes = client.start_all_processes()?;
            if cli.json {
                print_json(&processes);
            } else {
                for p in &processes {
                    println!("{}\t{:?}", p.name, p.status);
                }
            }
        },
        ProcessesCommand::StopAll => {
            let processes = client.stop_all_processes()?;
            if cli.json {
                print_json(&processes);
            } else {
                for p in &processes {
                    println!("{}\t{:?}", p.name, p.status);
                }
            }
        },
        ProcessesCommand::Start { name } => {
            let info = client.start_process(name)?;
            if cli.json {
                print_json(&info);
            } else {
                println!("{}\t{:?}", info.name, info.status);
            }
        },
        ProcessesCommand::Stop { name } => {
            let info = client.stop_process(name)?;
            if cli.json {
                print_json(&info);
            } else {
                println!("{}\t{:?}", info.name, info.status);
            }
        },
        ProcessesCommand::Restart { name } => {
            let info = client.restart_process(name)?;
            if cli.json {
                print_json(&info);
            } else {
                println!("{}\t{:?}", info.name, info.status);
            }
        },
    }
    Ok(())
}

fn run_tasks(
    cli: &Cli,
    client: &DaemonClient,
    cmd: &TasksCommand,
) -> Result<(), DaemonClientError> {
    match cmd {
        TasksCommand::List => {
            let tasks = client.list_tasks()?;
            if cli.json {
                print_json(&tasks);
            } else {
                for t in &tasks {
                    let trigger = if t.has_trigger {
                        " [trigger]"
                    } else {
                        ""
                    };
                    println!("{}\t{:?}\t{}{trigger}", t.name, t.status, t.schedule);
                }
            }
        },
        TasksCommand::Run { name } => {
            let info = client.run_task(name)?;
            if cli.json {
                print_json(&info);
            } else {
                println!("{}\t{:?}", info.name, info.status);
            }
        },
        TasksCommand::History { name } => {
            let history = client.task_history(name)?;
            if cli.json {
                print_json(&history);
            } else {
                for exec in &history {
                    let exit = exec
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "-".to_owned());
                    let agent = if exec.agent_spawned {
                        " [agent]"
                    } else {
                        ""
                    };
                    println!(
                        "{}\texit={exit}{agent}\t{}",
                        exec.task_name, exec.started_at_unix_ms
                    );
                }
            }
        },
    }
    Ok(())
}

fn print_json(value: &impl Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("error: failed to serialize JSON: {error}"),
    }
}
