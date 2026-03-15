use {
    crate::cli::*,
    arbor_daemon_client::{
        CommitWorktreeRequest, CreateTerminalRequest, CreateWorktreeRequest, DaemonClient,
        DaemonClientError, DeleteWorktreeRequest, PushWorktreeRequest, TerminalResizeRequest,
        TerminalSignalRequest,
    },
    serde::Serialize,
};

pub(crate) fn run(cli: &Cli, client: &DaemonClient) -> Result<(), DaemonClientError> {
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
