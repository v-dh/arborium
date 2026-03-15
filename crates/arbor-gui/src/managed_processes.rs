use super::*;

pub(crate) fn managed_processes_for_worktree(
    repo_root: &Path,
    worktree_path: &Path,
) -> Vec<ManagedWorktreeProcess> {
    let mut processes = Vec::new();

    if paths_equivalent(repo_root, worktree_path) {
        processes.extend(arbor_toml_processes_for_worktree(repo_root, worktree_path));
    }
    processes.extend(procfile_processes_for_worktree(worktree_path));

    processes
}

pub(crate) fn arbor_toml_processes_for_worktree(
    repo_root: &Path,
    worktree_path: &Path,
) -> Vec<ManagedWorktreeProcess> {
    let Some(config) = repo_config::load_repo_config(repo_root) else {
        return Vec::new();
    };

    config
        .processes
        .into_iter()
        .filter(|process| !process.name.trim().is_empty() && !process.command.trim().is_empty())
        .map(|process| ManagedWorktreeProcess {
            id: managed_process_id(ProcessSource::ArborToml, worktree_path, &process.name),
            name: process.name,
            command: process.command,
            working_dir: process
                .working_dir
                .as_deref()
                .map(|dir| repo_root.join(dir))
                .unwrap_or_else(|| repo_root.to_path_buf()),
            source: ProcessSource::ArborToml,
        })
        .collect()
}

pub(crate) fn procfile_processes_for_worktree(worktree_path: &Path) -> Vec<ManagedWorktreeProcess> {
    match procfile::read_procfile(worktree_path) {
        Ok(Some(entries)) => entries
            .into_iter()
            .map(|entry| ManagedWorktreeProcess {
                id: managed_process_id(ProcessSource::Procfile, worktree_path, &entry.name),
                name: entry.name,
                command: entry.command,
                working_dir: worktree_path.to_path_buf(),
                source: ProcessSource::Procfile,
            })
            .collect(),
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(path = %worktree_path.display(), %error, "failed to read Procfile");
            Vec::new()
        },
    }
}

pub(crate) fn managed_process_id(
    source: ProcessSource,
    worktree_path: &Path,
    process_name: &str,
) -> String {
    format!(
        "{}:{}:{process_name}",
        managed_process_source_label(source),
        worktree_path.display()
    )
}

pub(crate) fn managed_process_source_label(source: ProcessSource) -> &'static str {
    match source {
        ProcessSource::ArborToml => "arbor-toml",
        ProcessSource::Procfile => "procfile",
    }
}

pub(crate) fn managed_process_source_display_name(source: ProcessSource) -> &'static str {
    match source {
        ProcessSource::ArborToml => "arbor.toml",
        ProcessSource::Procfile => "Procfile",
    }
}

pub(crate) fn managed_process_title(source: ProcessSource, process_name: &str) -> String {
    managed_process_session_title(source, process_name)
}

pub(crate) fn managed_process_id_from_title(worktree_path: &Path, title: &str) -> Option<String> {
    managed_process_source_and_name_from_title(title)
        .map(|(source, name)| managed_process_id(source, worktree_path, name))
}

pub(crate) fn managed_process_session_is_active(session: &TerminalSession) -> bool {
    session.is_initializing || session.state == TerminalState::Running
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use {
        arbor_core::process::ProcessSource,
        std::{env, fs, path::Path, time::SystemTime},
    };

    #[test]
    fn managed_process_title_round_trips_to_process_id() {
        let worktree_path = Path::new("/tmp/repo");
        assert_eq!(
            crate::managed_process_id_from_title(
                worktree_path,
                &crate::managed_process_title(ProcessSource::Procfile, "web"),
            ),
            Some(crate::managed_process_id(
                ProcessSource::Procfile,
                worktree_path,
                "web",
            ))
        );
        assert_eq!(
            crate::managed_process_id_from_title(
                worktree_path,
                &crate::managed_process_title(ProcessSource::ArborToml, "worker"),
            ),
            Some(crate::managed_process_id(
                ProcessSource::ArborToml,
                worktree_path,
                "worker",
            ))
        );
    }

    #[test]
    fn managed_processes_for_primary_worktree_include_arbor_toml_processes() {
        let unique_suffix = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos(),
            Err(error) => panic!("current time should be after the unix epoch: {error}"),
        };
        let repo_root = env::temp_dir().join(format!("arbor-managed-processes-{unique_suffix}"));
        let linked_worktree = repo_root.join("worktrees").join("feature");

        if let Err(error) = fs::create_dir_all(&linked_worktree) {
            panic!("linked worktree dir should be created: {error}");
        }
        if let Err(error) = fs::write(
            repo_root.join("arbor.toml"),
            "[[processes]]\nname = \"worker\"\ncommand = \"cargo run -- worker\"\nworking_dir = \"backend\"\n",
        ) {
            panic!("arbor.toml should be written: {error}");
        }

        let primary_processes = crate::managed_processes_for_worktree(&repo_root, &repo_root);
        assert!(primary_processes.iter().any(|process| {
            process.source == ProcessSource::ArborToml
                && process.name == "worker"
                && process.working_dir == repo_root.join("backend")
        }));

        let linked_processes = crate::managed_processes_for_worktree(&repo_root, &linked_worktree);
        assert!(
            !linked_processes
                .iter()
                .any(|process| process.source == ProcessSource::ArborToml)
        );

        if let Err(error) = fs::remove_dir_all(&repo_root) {
            panic!("temp repo root should be removed: {error}");
        }
    }

    #[test]
    fn completed_managed_process_sessions_are_not_active() {
        let mut session =
            crate::daemon_runtime::session_with_styled_line("", 0xffffff, 0x000000, None);
        session.managed_process_id = Some("procfile:/tmp/worktree:web".to_owned());
        session.is_initializing = false;
        session.state = crate::TerminalState::Completed;
        assert!(!crate::managed_process_session_is_active(&session));

        session.state = crate::TerminalState::Running;
        assert!(crate::managed_process_session_is_active(&session));

        session.is_initializing = true;
        session.state = crate::TerminalState::Completed;
        assert!(crate::managed_process_session_is_active(&session));
    }
}
