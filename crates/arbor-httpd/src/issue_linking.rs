use {
    crate::managed_worktree,
    arbor_daemon_client::{IssueDto, IssueReviewDto, IssueReviewKind},
    std::path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkedIssueWorktree {
    pub(crate) path: PathBuf,
    pub(crate) branch: String,
    pub(crate) pr_number: Option<u64>,
    pub(crate) pr_url: Option<String>,
    pub(crate) last_activity_unix_ms: Option<u64>,
}

pub(crate) fn enrich_issues_with_worktree_links(
    repo_root: &Path,
    issues: &mut [IssueDto],
    worktrees: &[LinkedIssueWorktree],
) {
    for issue in issues {
        let Some(linked_worktree) = best_linked_worktree(repo_root, issue, worktrees) else {
            issue.linked_branch = None;
            issue.linked_review = None;
            continue;
        };

        issue.linked_branch = Some(linked_worktree.branch.clone());
        issue.linked_review = linked_worktree.pr_number.map(|pr_number| IssueReviewDto {
            kind: IssueReviewKind::PullRequest,
            label: format!("PR #{pr_number}"),
            url: linked_worktree.pr_url.clone(),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IssueMatchPattern {
    expected_branch: Option<String>,
    exact_worktree_name: String,
    reference_aliases: Vec<String>,
}

fn best_linked_worktree<'a>(
    repo_root: &Path,
    issue: &IssueDto,
    worktrees: &'a [LinkedIssueWorktree],
) -> Option<&'a LinkedIssueWorktree> {
    let pattern = issue_match_pattern(repo_root, issue)?;

    worktrees
        .iter()
        .max_by_key(|worktree| {
            match_score(&pattern, worktree)
                .map(|score| {
                    (
                        score,
                        u8::from(worktree.pr_number.is_some() || worktree.pr_url.is_some()),
                        worktree.last_activity_unix_ms.unwrap_or(0),
                    )
                })
                .unwrap_or((0, 0, 0))
        })
        .filter(|worktree| match_score(&pattern, worktree).is_some())
}

fn issue_match_pattern(repo_root: &Path, issue: &IssueDto) -> Option<IssueMatchPattern> {
    let exact_worktree_name =
        managed_worktree::sanitize_worktree_name(&issue.suggested_worktree_name);
    if exact_worktree_name.is_empty() {
        return None;
    }

    let expected_branch =
        managed_worktree::derive_managed_worktree_naming(repo_root, &issue.suggested_worktree_name)
            .ok()
            .map(|naming| naming.branch_name);

    let reference = issue_reference(issue);
    let reference_slug = issue_reference_slug(&reference);
    let reference_aliases = build_reference_aliases(&reference_slug);

    Some(IssueMatchPattern {
        expected_branch,
        exact_worktree_name,
        reference_aliases,
    })
}

fn issue_reference(issue: &IssueDto) -> String {
    non_empty_trimmed_str(issue.display_id.trim_start_matches('#'))
        .map(str::to_owned)
        .or_else(|| non_empty_trimmed_str(&issue.id).map(str::to_owned))
        .unwrap_or_default()
}

fn issue_reference_slug(reference: &str) -> String {
    let sanitized = managed_worktree::sanitize_worktree_name(reference);
    if sanitized.is_empty() {
        return String::new();
    }

    if sanitized
        .chars()
        .all(|character| character.is_ascii_digit() || character == '-')
    {
        return format!("issue-{sanitized}");
    }

    sanitized
}

fn build_reference_aliases(reference_slug: &str) -> Vec<String> {
    if reference_slug.is_empty() {
        return Vec::new();
    }

    let mut aliases = vec![reference_slug.to_owned()];
    if let Some(number) = reference_slug.strip_prefix("issue-") {
        aliases.push(format!("github-{number}"));
        aliases.push(format!("gitlab-{number}"));
        aliases.push(format!("linear-{number}"));
    }
    aliases
}

fn match_score(pattern: &IssueMatchPattern, worktree: &LinkedIssueWorktree) -> Option<u8> {
    let branch = worktree.branch.trim();
    if branch.is_empty() || branch == "-" {
        return None;
    }

    let branch_suffix = branch.rsplit('/').next().unwrap_or(branch);
    let worktree_name = worktree
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .map(managed_worktree::sanitize_worktree_name)
        .unwrap_or_default();

    if pattern
        .expected_branch
        .as_deref()
        .is_some_and(|expected| branch == expected)
    {
        return Some(5);
    }

    if worktree_name == pattern.exact_worktree_name {
        return Some(4);
    }

    if branch_suffix == pattern.exact_worktree_name {
        return Some(3);
    }

    if pattern.reference_aliases.iter().any(|alias| {
        slug_matches_issue_alias(&worktree_name, alias)
            || slug_matches_issue_alias(branch_suffix, alias)
    }) {
        return Some(2);
    }

    None
}

fn slug_matches_issue_alias(slug: &str, alias: &str) -> bool {
    if slug == alias {
        return true;
    }

    slug.strip_prefix(alias)
        .is_some_and(|suffix| suffix.starts_with('-') || suffix.starts_with('_'))
}

fn non_empty_trimmed_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(display_id: &str, title: &str, suggested_worktree_name: &str) -> IssueDto {
        IssueDto {
            id: display_id.trim_start_matches('#').to_owned(),
            display_id: display_id.to_owned(),
            title: title.to_owned(),
            state: "open".to_owned(),
            url: None,
            body: None,
            suggested_worktree_name: suggested_worktree_name.to_owned(),
            updated_at: Some("2026-03-13T12:00:00Z".to_owned()),
            linked_branch: None,
            linked_review: None,
        }
    }

    fn linked_worktree(
        path: &str,
        branch: &str,
        pr_number: Option<u64>,
        last_activity_unix_ms: u64,
    ) -> LinkedIssueWorktree {
        LinkedIssueWorktree {
            path: PathBuf::from(path),
            branch: branch.to_owned(),
            pr_number,
            pr_url: pr_number.map(|number| format!("https://github.com/penso/arbor/pull/{number}")),
            last_activity_unix_ms: Some(last_activity_unix_ms),
        }
    }

    #[test]
    fn enriches_issue_with_exact_worktree_and_pull_request_match() {
        let repo_root = Path::new("/tmp/arbor");
        let mut issues = vec![issue(
            "#512",
            "Ship daemon-backed issue worktrees",
            "issue-512-ship-daemon-backed-issue-worktrees",
        )];
        let worktrees = vec![linked_worktree(
            "/Users/penso/.arbor/worktrees/arbor/issue-512-ship-daemon-backed-issue-worktrees",
            "codex/issue-512-ship-daemon-backed-issue-worktrees",
            Some(365),
            10,
        )];

        enrich_issues_with_worktree_links(repo_root, &mut issues, &worktrees);

        assert_eq!(
            issues[0].linked_branch.as_deref(),
            Some("codex/issue-512-ship-daemon-backed-issue-worktrees")
        );
        assert_eq!(
            issues[0]
                .linked_review
                .as_ref()
                .map(|review| review.label.as_str()),
            Some("PR #365")
        );
    }

    #[test]
    fn matches_existing_branch_when_issue_title_changed() {
        let repo_root = Path::new("/tmp/arbor");
        let mut issues = vec![issue(
            "#42",
            "Fix auth callback race now",
            "issue-42-fix-auth-callback-race-now",
        )];
        let worktrees = vec![linked_worktree(
            "/Users/penso/.arbor/worktrees/arbor/issue-42-fix-auth-callback-race",
            "codex/issue-42-fix-auth-callback-race",
            None,
            100,
        )];

        enrich_issues_with_worktree_links(repo_root, &mut issues, &worktrees);

        assert_eq!(
            issues[0].linked_branch.as_deref(),
            Some("codex/issue-42-fix-auth-callback-race")
        );
        assert_eq!(issues[0].linked_review, None);
    }

    #[test]
    fn matches_provider_prefixed_legacy_worktree_names() {
        let repo_root = Path::new("/tmp/arbor");
        let mut issues = vec![issue(
            "#512",
            "Ship daemon-backed issue worktrees",
            "issue-512-ship-daemon-backed-issue-worktrees",
        )];
        let worktrees = vec![linked_worktree(
            "/Users/penso/.arbor/worktrees/arbor/github-512-ship-httpd-issues",
            "codex/github-512-ship-httpd-issues",
            Some(365),
            50,
        )];

        enrich_issues_with_worktree_links(repo_root, &mut issues, &worktrees);

        assert_eq!(
            issues[0].linked_branch.as_deref(),
            Some("codex/github-512-ship-httpd-issues")
        );
        assert_eq!(
            issues[0]
                .linked_review
                .as_ref()
                .map(|review| review.label.as_str()),
            Some("PR #365")
        );
    }

    #[test]
    fn prefers_exact_match_over_reference_only_match() {
        let repo_root = Path::new("/tmp/arbor");
        let mut issues = vec![issue(
            "#42",
            "Fix auth callback race now",
            "issue-42-fix-auth-callback-race-now",
        )];
        let worktrees = vec![
            linked_worktree(
                "/Users/penso/.arbor/worktrees/arbor/issue-42-old-title",
                "codex/issue-42-old-title",
                None,
                200,
            ),
            linked_worktree(
                "/Users/penso/.arbor/worktrees/arbor/issue-42-fix-auth-callback-race-now",
                "codex/issue-42-fix-auth-callback-race-now",
                Some(420),
                100,
            ),
        ];

        enrich_issues_with_worktree_links(repo_root, &mut issues, &worktrees);

        assert_eq!(
            issues[0].linked_branch.as_deref(),
            Some("codex/issue-42-fix-auth-callback-race-now")
        );
        assert_eq!(
            issues[0]
                .linked_review
                .as_ref()
                .map(|review| review.label.as_str()),
            Some("PR #420")
        );
    }
}
