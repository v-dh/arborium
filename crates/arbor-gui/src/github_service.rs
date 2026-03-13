use {
    serde::Deserialize,
    std::{
        process::{Command, Stdio},
        sync::Arc,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Success,
    Failure,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeableState {
    Conflicting,
    Mergeable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStateStatus {
    Behind,
    Blocked,
    Clean,
    Dirty,
    Draft,
    HasHooks,
    Unknown,
    Unstable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrDetails {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    pub additions: usize,
    pub deletions: usize,
    pub review_decision: ReviewDecision,
    pub mergeable: MergeableState,
    pub merge_state_status: MergeStateStatus,
    pub passed_checks: usize,
    pub checks_status: CheckStatus,
    pub checks: Vec<(String, CheckStatus)>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrResponse {
    number: u64,
    title: String,
    url: String,
    state: String,
    is_draft: bool,
    additions: usize,
    deletions: usize,
    review_decision: Option<String>,
    mergeable: Option<String>,
    merge_state_status: Option<String>,
    #[serde(default)]
    status_check_rollup: Vec<GhCheckContext>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReviewPullRequest {
    pub(crate) number: u64,
    pub(crate) title: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewPrResponse {
    number: u64,
    title: String,
}

#[derive(Deserialize)]
struct GhCheckContext {
    name: Option<String>,
    context: Option<String>,
    conclusion: Option<String>,
    status: Option<String>,
    state: Option<String>,
}

impl GhCheckContext {
    fn display_name(&self) -> String {
        self.name
            .as_deref()
            .or(self.context.as_deref())
            .unwrap_or("check")
            .to_owned()
    }

    fn to_check_status(&self) -> CheckStatus {
        if self
            .status
            .as_deref()
            .is_some_and(|status| !status.eq_ignore_ascii_case("completed"))
        {
            return CheckStatus::Pending;
        }

        if let Some(conclusion) = &self.conclusion {
            return match conclusion.as_str() {
                "" => CheckStatus::Pending,
                "SUCCESS" | "success" | "NEUTRAL" | "neutral" | "SKIPPED" | "skipped" => {
                    CheckStatus::Success
                },
                _ => CheckStatus::Failure,
            };
        }
        if let Some(state) = &self.state {
            return match state.as_str() {
                "SUCCESS" | "success" => CheckStatus::Success,
                "FAILURE" | "failure" | "ERROR" | "error" => CheckStatus::Failure,
                _ => CheckStatus::Pending,
            };
        }
        if let Some(status) = &self.status {
            return match status.as_str() {
                "COMPLETED" | "completed" => CheckStatus::Success,
                _ => CheckStatus::Pending,
            };
        }
        CheckStatus::Pending
    }
}

fn parse_pr_details(response: GhPrResponse) -> PrDetails {
    let state = if response.is_draft {
        PrState::Draft
    } else {
        match response.state.as_str() {
            "MERGED" | "merged" => PrState::Merged,
            "CLOSED" | "closed" => PrState::Closed,
            _ => PrState::Open,
        }
    };

    let review_decision = match response.review_decision.as_deref() {
        Some("APPROVED") => ReviewDecision::Approved,
        Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
        _ => ReviewDecision::Pending,
    };
    let mergeable = match response.mergeable.as_deref() {
        Some("CONFLICTING") => MergeableState::Conflicting,
        Some("MERGEABLE") => MergeableState::Mergeable,
        _ => MergeableState::Unknown,
    };
    let merge_state_status = match response.merge_state_status.as_deref() {
        Some("BEHIND") => MergeStateStatus::Behind,
        Some("BLOCKED") => MergeStateStatus::Blocked,
        Some("CLEAN") => MergeStateStatus::Clean,
        Some("DIRTY") => MergeStateStatus::Dirty,
        Some("DRAFT") => MergeStateStatus::Draft,
        Some("HAS_HOOKS") => MergeStateStatus::HasHooks,
        Some("UNSTABLE") => MergeStateStatus::Unstable,
        _ => MergeStateStatus::Unknown,
    };

    let mut checks: Vec<(String, CheckStatus)> = response
        .status_check_rollup
        .iter()
        .map(|c| (c.display_name(), c.to_check_status()))
        .collect();
    let passed_checks = checks
        .iter()
        .filter(|(_, status)| *status == CheckStatus::Success)
        .count();

    let checks_status = if checks.is_empty() {
        CheckStatus::Pending
    } else if checks.iter().any(|(_, s)| *s == CheckStatus::Failure) {
        CheckStatus::Failure
    } else if checks.iter().all(|(_, s)| *s == CheckStatus::Success) {
        CheckStatus::Success
    } else {
        CheckStatus::Pending
    };
    sort_checks_for_display(&mut checks);

    PrDetails {
        number: response.number,
        title: response.title,
        url: response.url,
        state,
        additions: response.additions,
        deletions: response.deletions,
        review_decision,
        mergeable,
        merge_state_status,
        passed_checks,
        checks_status,
        checks,
    }
}

fn sort_checks_for_display(checks: &mut [(String, CheckStatus)]) {
    checks.sort_by(|left, right| {
        check_status_sort_key(left.1)
            .cmp(&check_status_sort_key(right.1))
            .then(left.0.cmp(&right.0))
    });
}

fn check_status_sort_key(status: CheckStatus) -> usize {
    match status {
        CheckStatus::Failure => 0,
        CheckStatus::Pending => 1,
        CheckStatus::Success => 2,
    }
}

/// Fetch rich PR details using `gh pr view`. Returns `None` if `gh` is not
/// installed, the command fails, or no PR exists for the given branch.
pub fn pull_request_details(repo_slug: &str, branch: &str) -> Option<PrDetails> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            "--repo",
            repo_slug,
            "--json",
            "number,title,url,state,isDraft,additions,deletions,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup",
            branch,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let response: GhPrResponse = serde_json::from_slice(&output.stdout).ok()?;
    Some(parse_pr_details(response))
}

pub trait GitHubService: Send + Sync {
    fn create_pull_request(
        &self,
        repo_slug: &str,
        title: &str,
        branch: &str,
        base_branch: &str,
        token: &str,
    ) -> Result<String, String>;

    fn pull_request_number(&self, repo_slug: &str, branch: &str, token: &str) -> Option<u64>;
}

pub struct OctocrabGitHubService;

impl GitHubService for OctocrabGitHubService {
    fn create_pull_request(
        &self,
        repo_slug: &str,
        title: &str,
        branch: &str,
        base_branch: &str,
        token: &str,
    ) -> Result<String, String> {
        let (owner, repo_name) = repo_slug
            .split_once('/')
            .ok_or_else(|| format!("invalid repository slug: {repo_slug}"))?;

        let owner = owner.to_owned();
        let repo_name = repo_name.to_owned();
        let title = title.to_owned();
        let branch = branch.to_owned();
        let base_branch = base_branch.to_owned();
        let token = token.to_owned();

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|error| format!("failed to create runtime: {error}"))?;

        runtime.block_on(async move {
            let octocrab = octocrab::Octocrab::builder()
                .personal_token(token)
                .build()
                .map_err(|error| format!("failed to create GitHub client: {error}"))?;

            let pr = octocrab
                .pulls(&owner, &repo_name)
                .create(&title, &branch, &base_branch)
                .send()
                .await
                .map_err(|error| format!("failed to create pull request: {error}"))?;

            let url = pr.html_url.map(|u| u.to_string()).unwrap_or_default();
            Ok(format!("created PR: {url}"))
        })
    }

    fn pull_request_number(&self, repo_slug: &str, branch: &str, token: &str) -> Option<u64> {
        let (owner, repo_name) = repo_slug.split_once('/')?;
        let owner = owner.to_owned();
        let repo_name = repo_name.to_owned();
        let branch = branch.to_owned();
        let token = token.to_owned();

        let runtime = tokio::runtime::Runtime::new().ok()?;
        runtime.block_on(async move {
            let octocrab = octocrab::Octocrab::builder()
                .personal_token(token)
                .build()
                .ok()?;

            let page = octocrab
                .pulls(&owner, &repo_name)
                .list()
                .head(format!("{owner}:{branch}"))
                .state(octocrab::params::State::All)
                .per_page(1)
                .send()
                .await
                .ok()?;

            page.items.first().map(|pr| pr.number)
        })
    }
}

pub fn default_github_service() -> Arc<dyn GitHubService> {
    Arc::new(OctocrabGitHubService)
}

pub fn github_access_token_from_gh_cli() -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let token = stdout.trim();
    (!token.is_empty()).then_some(token.to_owned())
}

pub(crate) fn resolve_pull_request_for_review(
    repo_slug: &str,
    reference: &str,
    github_token: Option<&str>,
) -> Result<ReviewPullRequest, String> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err("pull request reference is required".to_owned());
    }

    if let Some(pull_request) = resolve_pull_request_for_review_via_gh(repo_slug, reference) {
        return Ok(pull_request);
    }

    let pull_request_number = parse_pull_request_number(reference).ok_or_else(|| {
        "failed to resolve pull request with gh; use a PR number or GitHub pull request URL"
            .to_owned()
    })?;
    let token = crate::resolve_github_access_token(github_token).ok_or_else(|| {
        "failed to resolve pull request with gh and no GitHub token is available for API fallback"
            .to_owned()
    })?;

    resolve_pull_request_for_review_via_api(repo_slug, pull_request_number, &token)
}

fn resolve_pull_request_for_review_via_gh(
    repo_slug: &str,
    reference: &str,
) -> Option<ReviewPullRequest> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            "--repo",
            repo_slug,
            "--json",
            "number,title",
            reference,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let response: GhReviewPrResponse = serde_json::from_slice(&output.stdout).ok()?;
    Some(ReviewPullRequest {
        number: response.number,
        title: response.title,
    })
}

fn resolve_pull_request_for_review_via_api(
    repo_slug: &str,
    pull_request_number: u64,
    token: &str,
) -> Result<ReviewPullRequest, String> {
    let (owner, repo_name) = repo_slug
        .split_once('/')
        .ok_or_else(|| format!("invalid repository slug: {repo_slug}"))?;

    let owner = owner.to_owned();
    let repo_name = repo_name.to_owned();
    let token = token.to_owned();

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| format!("failed to create runtime: {error}"))?;

    runtime.block_on(async move {
        let octocrab = octocrab::Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|error| format!("failed to create GitHub client: {error}"))?;

        let pull_request = octocrab
            .pulls(&owner, &repo_name)
            .get(pull_request_number)
            .await
            .map_err(|error| {
                format!("failed to resolve pull request #{pull_request_number}: {error}")
            })?;

        Ok(ReviewPullRequest {
            number: pull_request.number,
            title: pull_request
                .title
                .unwrap_or_else(|| format!("PR {}", pull_request.number)),
        })
    })
}

pub(crate) fn parse_pull_request_number(reference: &str) -> Option<u64> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }

    let trimmed = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if let Ok(number) = trimmed.parse::<u64>() {
        return Some(number);
    }

    let url_path = trimmed.strip_prefix("https://github.com/")?;
    let segments = url_path.split('/').collect::<Vec<_>>();
    if segments.len() < 4 || segments[2] != "pull" {
        return None;
    }

    segments[3].parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        CheckStatus, GhCheckContext, GhPrResponse, MergeStateStatus, MergeableState,
        parse_pr_details, parse_pull_request_number,
    };

    #[test]
    fn in_progress_checks_remain_pending_without_conclusion() {
        let details = parse_pr_details(GhPrResponse {
            number: 40,
            title: "Fix native toolbar buttons and linked worktree grouping".to_owned(),
            url: "https://github.com/penso/arbor/pull/40".to_owned(),
            state: "OPEN".to_owned(),
            is_draft: false,
            additions: 10,
            deletions: 2,
            review_decision: None,
            mergeable: Some("MERGEABLE".to_owned()),
            merge_state_status: Some("CLEAN".to_owned()),
            status_check_rollup: vec![
                GhCheckContext {
                    name: Some("Clippy".to_owned()),
                    context: None,
                    conclusion: Some(String::new()),
                    status: Some("IN_PROGRESS".to_owned()),
                    state: None,
                },
                GhCheckContext {
                    name: Some("Test".to_owned()),
                    context: None,
                    conclusion: Some(String::new()),
                    status: Some("IN_PROGRESS".to_owned()),
                    state: None,
                },
            ],
        });

        assert_eq!(details.checks_status, CheckStatus::Pending);
        assert_eq!(details.passed_checks, 0);
        assert_eq!(details.mergeable, MergeableState::Mergeable);
        assert_eq!(details.merge_state_status, MergeStateStatus::Clean);
        assert_eq!(details.checks, vec![
            ("Clippy".to_owned(), CheckStatus::Pending),
            ("Test".to_owned(), CheckStatus::Pending),
        ]);
    }

    #[test]
    fn parse_pull_request_number_supports_hash_number_and_url() {
        assert_eq!(parse_pull_request_number("#42"), Some(42));
        assert_eq!(parse_pull_request_number("42"), Some(42));
        assert_eq!(
            parse_pull_request_number("https://github.com/penso/arbor/pull/42"),
            Some(42)
        );
        assert_eq!(parse_pull_request_number("feature/test"), None);
    }
}
