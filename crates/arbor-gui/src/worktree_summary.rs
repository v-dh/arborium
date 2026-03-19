use super::*;

impl WorktreeSummary {
    pub(crate) fn from_worktree(
        entry: &worktree::Worktree,
        repo_root: &Path,
        group_key: &str,
        checkout_kind: CheckoutKind,
        include_metadata: bool,
    ) -> Self {
        let label = entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.path.display().to_string());

        let branch = entry
            .branch
            .as_deref()
            .map(short_branch)
            .unwrap_or_else(|| "-".to_owned());
        let is_primary_checkout = entry.path.as_path() == repo_root;
        let (branch_divergence, managed_processes, last_activity_unix_ms) = if include_metadata {
            (
                branch_divergence_summary(&entry.path),
                managed_processes_for_worktree(repo_root, &entry.path),
                worktree::last_git_activity_ms(&entry.path),
            )
        } else {
            (None, Vec::new(), None)
        };

        Self {
            group_key: group_key.to_owned(),
            checkout_kind,
            repo_root: repo_root.to_path_buf(),
            path: entry.path.clone(),
            label,
            branch,
            is_primary_checkout,
            pr_loading: false,
            pr_loaded: false,
            pr_number: None,
            pr_url: None,
            pr_details: None,
            branch_divergence,
            diff_summary: None,
            detected_ports: Vec::new(),
            managed_processes,
            recent_turns: Vec::new(),
            stuck_turn_count: 0,
            recent_agent_sessions: Vec::new(),
            recent_agent_sessions_loaded: false,
            agent_state: None,
            agent_task: None,
            agent_task_loaded: false,
            last_activity_unix_ms,
        }
    }

    pub(crate) fn apply_cached_pull_request_state(
        &mut self,
        cached: &ui_state_store::CachedPullRequestState,
    ) {
        self.pr_loaded = true;
        self.pr_number = cached.number;
        self.pr_url = cached.url.clone();
        self.pr_details = cached.details.clone();
    }

    pub(crate) fn cached_pull_request_state(
        &self,
    ) -> Option<ui_state_store::CachedPullRequestState> {
        self.pr_loaded
            .then(|| ui_state_store::CachedPullRequestState {
                branch: self.branch.clone(),
                number: self.pr_number,
                url: self.pr_url.clone(),
                details: self.pr_details.clone(),
            })
    }
}

impl RepositorySummary {
    pub(crate) fn from_checkout_roots(
        root: PathBuf,
        group_key: String,
        checkout_roots: Vec<repository_store::RepositoryCheckoutRoot>,
    ) -> Self {
        let label = repository_display_name(&root);
        let github_repo_slug = github_repo_slug_for_repo(&root);
        let avatar_url = github_repo_slug
            .as_ref()
            .and_then(|repo_slug| github_avatar_url_for_repo_slug(repo_slug));

        Self {
            group_key,
            root,
            checkout_roots,
            label,
            avatar_url,
            github_repo_slug,
        }
    }

    pub(crate) fn contains_checkout_root(&self, root: &Path) -> bool {
        self.checkout_roots
            .iter()
            .any(|checkout_root| checkout_root.path == root)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) mod tests {
    use {
        crate::{WorktreeSummary, checkout::CheckoutKind},
        arbor_core::{agent::AgentState, changes::DiffLineSummary},
    };

    pub(crate) fn sample_worktree_summary() -> WorktreeSummary {
        WorktreeSummary {
            group_key: "/tmp/repo".to_owned(),
            checkout_kind: CheckoutKind::LinkedWorktree,
            repo_root: "/tmp/repo".into(),
            path: "/tmp/repo/wt".into(),
            label: "wt".to_owned(),
            branch: "feature/hover".to_owned(),
            is_primary_checkout: false,
            pr_loading: false,
            pr_loaded: false,
            pr_number: None,
            pr_url: None,
            pr_details: None,
            branch_divergence: None,
            diff_summary: Some(DiffLineSummary {
                additions: 3,
                deletions: 1,
            }),
            detected_ports: vec![],
            managed_processes: vec![],
            recent_turns: vec![],
            stuck_turn_count: 0,
            recent_agent_sessions: vec![],
            recent_agent_sessions_loaded: true,
            agent_state: Some(AgentState::Working),
            agent_task: Some("Investigating hover".to_owned()),
            agent_task_loaded: true,
            last_activity_unix_ms: None,
        }
    }
}
