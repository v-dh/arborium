use {
    crate::graphql::{PullRequestDetails, ReviewThreads, pull_request_details, review_threads},
    graphql_client::GraphQLQuery,
    serde::Deserialize,
    std::{
        process::{Command, Stdio},
        sync::Arc,
    },
};

fn github_api_agent() -> ureq::Agent {
    let config = ureq::config::Config::builder()
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
}

fn split_slug(repo_slug: &str) -> Result<(&str, &str), String> {
    repo_slug
        .split_once('/')
        .ok_or_else(|| format!("invalid repository slug: {repo_slug}"))
}

// ---------------------------------------------------------------------------
// Review comment types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSide {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct ReviewComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
    #[allow(dead_code)]
    pub outdated: bool,
    #[allow(dead_code)]
    pub path: String,
    #[allow(dead_code)]
    pub line: Option<usize>,
    #[allow(dead_code)]
    pub start_line: Option<usize>,
    #[allow(dead_code)]
    pub side: DiffSide,
    #[allow(dead_code)]
    pub in_reply_to: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ReviewThread {
    pub id: String,
    pub path: String,
    pub line: Option<usize>,
    #[allow(dead_code)]
    pub start_line: Option<usize>,
    pub side: DiffSide,
    pub is_resolved: bool,
    #[allow(dead_code)]
    pub is_outdated: bool,
    pub comments: Vec<ReviewComment>,
}

// ---------------------------------------------------------------------------
// GitHubReviewService trait
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub trait GitHubReviewService: Send + Sync {
    /// Fetch all review threads for a PR.
    fn fetch_review_threads(
        &self,
        repo_slug: &str,
        pr_number: u64,
    ) -> Result<Vec<ReviewThread>, String>;

    /// Post a new inline comment on a PR.
    fn post_review_comment(
        &self,
        repo_slug: &str,
        pr_number: u64,
        path: &str,
        line: usize,
        side: DiffSide,
        body: &str,
        commit_sha: &str,
    ) -> Result<ReviewComment, String>;

    /// Reply to an existing comment thread.
    fn reply_to_thread(
        &self,
        repo_slug: &str,
        pr_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<ReviewComment, String>;
}

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

#[derive(Debug, Clone)]
pub struct PrDetails {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    pub base_ref_name: String,
    pub additions: usize,
    pub deletions: usize,
    pub review_decision: ReviewDecision,
    pub checks_status: CheckStatus,
    pub checks: Vec<(String, CheckStatus)>,
}

// ---------------------------------------------------------------------------
// Typed GraphQL helper
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GraphQLResponse<D> {
    data: Option<D>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize)]
struct GraphQLError {
    message: String,
}

fn execute_graphql<Q: GraphQLQuery>(
    token: &str,
    variables: Q::Variables,
) -> Result<Q::ResponseData, String> {
    let request_body = Q::build_query(variables);
    let body = serde_json::to_string(&request_body)
        .map_err(|e| format!("failed to serialize GraphQL request: {e}"))?;

    let response = github_api_agent()
        .post("https://api.github.com/graphql")
        .header("Authorization", &format!("Bearer {token}"))
        .header("User-Agent", "Arbor")
        .content_type("application/json")
        .send(&body)
        .map_err(|e| format!("GitHub GraphQL request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.into_body().read_to_string().unwrap_or_default();
        return Err(format!("GitHub GraphQL returned {status}: {text}"));
    }

    let text = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("failed to read GraphQL response: {e}"))?;

    let gql: GraphQLResponse<Q::ResponseData> = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse GraphQL response: {e}"))?;

    if let Some(errors) = gql.errors {
        let msgs: Vec<String> = errors.into_iter().map(|e| e.message).collect();
        return Err(format!("GraphQL errors: {}", msgs.join(", ")));
    }

    gql.data
        .ok_or_else(|| "GraphQL response contained no data".to_owned())
}

// ---------------------------------------------------------------------------
// PR details mapping from typed GraphQL response
// ---------------------------------------------------------------------------

use pull_request_details::{
    CheckConclusionState, CheckStatusState,
    PullRequestDetailsRepositoryPullRequestsNodes as PrNode,
    PullRequestDetailsRepositoryPullRequestsNodesCommitsNodesCommitStatusCheckRollupContextsNodes as ContextNode,
    PullRequestReviewDecision as GqlReviewDecision, PullRequestState, StatusState,
};

fn check_status_from_context(ctx: &ContextNode) -> (String, CheckStatus) {
    match ctx {
        ContextNode::CheckRun(run) => {
            let status = if !matches!(run.status, CheckStatusState::COMPLETED) {
                CheckStatus::Pending
            } else {
                match &run.conclusion {
                    Some(
                        CheckConclusionState::SUCCESS
                        | CheckConclusionState::NEUTRAL
                        | CheckConclusionState::SKIPPED,
                    ) => CheckStatus::Success,
                    None => CheckStatus::Pending,
                    _ => CheckStatus::Failure,
                }
            };
            (run.name.clone(), status)
        },
        ContextNode::StatusContext(sc) => {
            let status = match sc.state {
                StatusState::SUCCESS => CheckStatus::Success,
                StatusState::FAILURE | StatusState::ERROR => CheckStatus::Failure,
                _ => CheckStatus::Pending,
            };
            (sc.context.clone(), status)
        },
    }
}

fn aggregate_checks_status(checks: &[(String, CheckStatus)]) -> CheckStatus {
    if checks.is_empty() {
        CheckStatus::Pending
    } else if checks.iter().any(|(_, s)| *s == CheckStatus::Failure) {
        CheckStatus::Failure
    } else if checks.iter().all(|(_, s)| *s == CheckStatus::Success) {
        CheckStatus::Success
    } else {
        CheckStatus::Pending
    }
}

fn pr_details_from_node(node: PrNode) -> PrDetails {
    let state = if node.is_draft {
        PrState::Draft
    } else {
        match node.state {
            PullRequestState::MERGED => PrState::Merged,
            PullRequestState::CLOSED => PrState::Closed,
            _ => PrState::Open,
        }
    };

    let review_decision = match node.review_decision {
        Some(GqlReviewDecision::APPROVED) => ReviewDecision::Approved,
        Some(GqlReviewDecision::CHANGES_REQUESTED) => ReviewDecision::ChangesRequested,
        _ => ReviewDecision::Pending,
    };

    let context_nodes: Vec<ContextNode> = node
        .commits
        .nodes
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.as_ref())
        .flat_map(|c| {
            c.commit
                .status_check_rollup
                .as_ref()
                .and_then(|r| r.contexts.nodes.as_deref())
                .unwrap_or_default()
                .iter()
                .filter_map(|n| n.as_ref())
                .cloned()
        })
        .collect();

    let checks: Vec<(String, CheckStatus)> = context_nodes
        .iter()
        .map(check_status_from_context)
        .collect();
    let checks_status = aggregate_checks_status(&checks);

    PrDetails {
        number: node.number as u64,
        title: node.title,
        url: node.url,
        state,
        base_ref_name: node.base_ref_name,
        additions: node.additions as usize,
        deletions: node.deletions as usize,
        review_decision,
        checks_status,
        checks,
    }
}

/// Fetch rich PR details using the GitHub GraphQL API. Returns `None` if no
/// token is available, the request fails, or no PR exists for the given branch.
pub fn pull_request_details(
    repo_slug: &str,
    branch: &str,
    token: Option<&str>,
) -> Option<PrDetails> {
    let token = token?;
    let (owner, repo) = split_slug(repo_slug).ok()?;

    let vars = pull_request_details::Variables {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        head: branch.to_owned(),
    };

    let data = execute_graphql::<PullRequestDetails>(token, vars).ok()?;

    data.repository?
        .pull_requests
        .nodes?
        .into_iter()
        .flatten()
        .next()
        .map(pr_details_from_node)
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

// ---------------------------------------------------------------------------
// UreqReviewService — uses ureq for GitHub GraphQL & REST API
// ---------------------------------------------------------------------------

pub struct UreqReviewService {
    token_fn: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl UreqReviewService {
    fn token(&self) -> Result<String, String> {
        (self.token_fn)().ok_or_else(|| "GitHub token not available".to_owned())
    }
}

// REST response for posting a comment ---------------------------------------

#[allow(dead_code)]
#[derive(Deserialize)]
struct RestCommentResponse {
    id: u64,
    body: String,
    path: String,
    line: Option<usize>,
    #[serde(rename = "start_line")]
    start_line: Option<usize>,
    side: Option<String>,
    user: Option<RestUser>,
    created_at: String,
    #[serde(rename = "in_reply_to_id")]
    in_reply_to_id: Option<u64>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct RestUser {
    login: String,
}

fn parse_diff_side(raw: Option<&str>) -> DiffSide {
    match raw {
        Some("LEFT") => DiffSide::Left,
        _ => DiffSide::Right,
    }
}

fn gql_diff_side_to_diff_side(side: &review_threads::DiffSide) -> DiffSide {
    match side {
        review_threads::DiffSide::LEFT => DiffSide::Left,
        _ => DiffSide::Right,
    }
}

impl GitHubReviewService for UreqReviewService {
    #[allow(deprecated)] // databaseId deprecated in favour of fullDatabaseId (BigInt)
    fn fetch_review_threads(
        &self,
        repo_slug: &str,
        pr_number: u64,
    ) -> Result<Vec<ReviewThread>, String> {
        let token = self.token()?;
        let (owner, repo) = split_slug(repo_slug)?;

        let vars = review_threads::Variables {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            pr: pr_number as i64,
        };

        let data = execute_graphql::<ReviewThreads>(&token, vars)?;

        let threads = data
            .repository
            .and_then(|r| r.pull_request)
            .map(|pr| pr.review_threads.nodes.unwrap_or_default())
            .unwrap_or_default();

        Ok(threads
            .into_iter()
            .flatten()
            .map(|thread| {
                let side = gql_diff_side_to_diff_side(&thread.diff_side);
                let comments_vec = thread.comments.nodes.unwrap_or_default();
                let first_comment_id = comments_vec
                    .first()
                    .and_then(|first| first.as_ref())
                    .and_then(|first| first.database_id)
                    .map(|id| id as u64);
                let comments = comments_vec
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, c)| ReviewComment {
                        id: c.database_id.map(|id| id as u64).unwrap_or(0),
                        author: c
                            .author
                            .map(|a| a.login)
                            .unwrap_or_else(|| "ghost".to_owned()),
                        body: c.body,
                        created_at: c.created_at,
                        outdated: c.outdated,
                        path: c.path,
                        line: c.line.map(|l| l as usize),
                        start_line: c.start_line.map(|l| l as usize),
                        side,
                        in_reply_to: if i > 0 {
                            first_comment_id
                        } else {
                            None
                        },
                    })
                    .collect::<Vec<_>>();

                ReviewThread {
                    id: thread.id,
                    path: thread.path,
                    line: thread.line.map(|l| l as usize),
                    start_line: thread.start_line.map(|l| l as usize),
                    side,
                    is_resolved: thread.is_resolved,
                    is_outdated: thread.is_outdated,
                    comments,
                }
            })
            .collect())
    }

    fn post_review_comment(
        &self,
        repo_slug: &str,
        pr_number: u64,
        path: &str,
        line: usize,
        side: DiffSide,
        body: &str,
        commit_sha: &str,
    ) -> Result<ReviewComment, String> {
        let token = self.token()?;
        let side_str = match side {
            DiffSide::Left => "LEFT",
            DiffSide::Right => "RIGHT",
        };

        let url = format!("https://api.github.com/repos/{repo_slug}/pulls/{pr_number}/comments");
        let payload = serde_json::to_string(&serde_json::json!({
            "body": body,
            "commit_id": commit_sha,
            "path": path,
            "line": line,
            "side": side_str,
        }))
        .map_err(|e| format!("failed to serialize comment payload: {e}"))?;

        let response = github_api_agent()
            .post(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("User-Agent", "Arbor")
            .content_type("application/json")
            .header("Accept", "application/vnd.github+json")
            .send(&payload)
            .map_err(|e| format!("GitHub REST POST failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.into_body().read_to_string().unwrap_or_default();
            return Err(format!("GitHub REST POST returned {status}: {text}"));
        }

        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| format!("failed to read comment response: {e}"))?;

        let resp: RestCommentResponse = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse comment response: {e}"))?;

        Ok(ReviewComment {
            id: resp.id,
            author: resp
                .user
                .map(|u| u.login)
                .unwrap_or_else(|| "ghost".to_owned()),
            body: resp.body,
            created_at: resp.created_at,
            outdated: false,
            path: resp.path,
            line: resp.line,
            start_line: resp.start_line,
            side: parse_diff_side(resp.side.as_deref()),
            in_reply_to: resp.in_reply_to_id,
        })
    }

    fn reply_to_thread(
        &self,
        repo_slug: &str,
        pr_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<ReviewComment, String> {
        let token = self.token()?;
        let url = format!(
            "https://api.github.com/repos/{repo_slug}/pulls/{pr_number}/comments/{comment_id}/replies"
        );
        let payload = serde_json::to_string(&serde_json::json!({ "body": body }))
            .map_err(|e| format!("failed to serialize reply payload: {e}"))?;

        let response = github_api_agent()
            .post(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("User-Agent", "Arbor")
            .content_type("application/json")
            .header("Accept", "application/vnd.github+json")
            .send(&payload)
            .map_err(|e| format!("GitHub REST POST reply failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.into_body().read_to_string().unwrap_or_default();
            return Err(format!("GitHub REST reply returned {status}: {text}"));
        }

        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| format!("failed to read reply response: {e}"))?;

        let resp: RestCommentResponse = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse reply response: {e}"))?;

        Ok(ReviewComment {
            id: resp.id,
            author: resp
                .user
                .map(|u| u.login)
                .unwrap_or_else(|| "ghost".to_owned()),
            body: resp.body,
            created_at: resp.created_at,
            outdated: false,
            path: resp.path,
            line: resp.line,
            start_line: resp.start_line,
            side: parse_diff_side(resp.side.as_deref()),
            in_reply_to: resp.in_reply_to_id,
        })
    }
}

pub fn default_review_service(
    token_fn: Arc<dyn Fn() -> Option<String> + Send + Sync>,
) -> Arc<dyn GitHubReviewService> {
    Arc::new(UreqReviewService { token_fn })
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn in_progress_checks_remain_pending() {
        use pull_request_details::{
            CheckStatusState,
            PullRequestDetailsRepositoryPullRequestsNodesCommitsNodesCommitStatusCheckRollupContextsNodesOnCheckRun as CheckRun,
        };

        let contexts = [
            ContextNode::CheckRun(CheckRun {
                name: "Clippy".to_owned(),
                conclusion: None,
                status: CheckStatusState::IN_PROGRESS,
            }),
            ContextNode::CheckRun(CheckRun {
                name: "Test".to_owned(),
                conclusion: None,
                status: CheckStatusState::IN_PROGRESS,
            }),
        ];

        let checks: Vec<(String, CheckStatus)> =
            contexts.iter().map(check_status_from_context).collect();

        assert_eq!(aggregate_checks_status(&checks), CheckStatus::Pending);
        assert_eq!(checks, vec![
            ("Clippy".to_owned(), CheckStatus::Pending),
            ("Test".to_owned(), CheckStatus::Pending),
        ]);
    }

    #[test]
    fn completed_check_with_success_conclusion() {
        use pull_request_details::{
            CheckConclusionState, CheckStatusState,
            PullRequestDetailsRepositoryPullRequestsNodesCommitsNodesCommitStatusCheckRollupContextsNodesOnCheckRun as CheckRun,
        };

        let ctx = ContextNode::CheckRun(CheckRun {
            name: "Build".to_owned(),
            conclusion: Some(CheckConclusionState::SUCCESS),
            status: CheckStatusState::COMPLETED,
        });

        let (name, status) = check_status_from_context(&ctx);
        assert_eq!(name, "Build");
        assert_eq!(status, CheckStatus::Success);
    }

    #[test]
    fn status_context_failure() {
        use pull_request_details::{
            PullRequestDetailsRepositoryPullRequestsNodesCommitsNodesCommitStatusCheckRollupContextsNodesOnStatusContext as StatusCtx,
            StatusState,
        };

        let ctx = ContextNode::StatusContext(StatusCtx {
            context: "ci/deploy".to_owned(),
            state: StatusState::FAILURE,
        });

        let (name, status) = check_status_from_context(&ctx);
        assert_eq!(name, "ci/deploy");
        assert_eq!(status, CheckStatus::Failure);
    }
}
