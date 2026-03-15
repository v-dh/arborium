use {
    crate::{IssueProviderError, managed_worktree},
    arbor_daemon_client::{
        IssueDto, IssueLabelDto, IssueListResponse, IssueSourceDto, IssueTypeDto,
    },
    secrecy::{ExposeSecret, SecretString},
    serde::Deserialize,
    std::{collections::HashSet, env, path::Path, time::Duration},
};

const GITHUB_GRAPHQL_API_URL: &str = "https://api.github.com/graphql";
const ISSUE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const GITLAB_METADATA_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const ISSUE_PAGE_SIZE: usize = 100;
const GITHUB_ISSUES_GRAPHQL_QUERY: &str = r#"
query($owner: String!, $repo: String!, $after: String) {
  repository(owner: $owner, name: $repo) {
    issues(first: 100, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}, after: $after) {
      pageInfo {
        hasNextPage
        endCursor
      }
      nodes {
        number
        title
        state
        url
        body
        updatedAt
        issueType {
          name
          color
        }
        labels(first: 8) {
          nodes {
            name
            color
          }
        }
      }
    }
  }
}
"#;

pub(crate) trait RepositoryIssueProvider: Send + Sync {
    fn resolve_source(
        &self,
        repo_root: &Path,
        origin_remote_url: &str,
    ) -> Option<ResolvedIssueSource>;
    fn list_issues(
        &self,
        source: &ResolvedIssueSource,
        github_token: Option<&SecretString>,
    ) -> Result<Vec<IssueDto>, IssueProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueProviderKind {
    GitHub,
    GitLab,
}

impl IssueProviderKind {
    const fn api_name(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedIssueSource {
    provider: IssueProviderKind,
    repository: String,
    url: Option<String>,
    api_base_url: String,
    gitlab_token_auth: GitLabTokenAuthPolicy,
}

impl ResolvedIssueSource {
    fn into_dto(self) -> IssueSourceDto {
        IssueSourceDto {
            provider: self.provider.api_name().to_owned(),
            label: self.provider.label().to_owned(),
            repository: self.repository,
            url: self.url,
        }
    }
}

pub(crate) struct RepositoryIssueService {
    providers: Vec<Box<dyn RepositoryIssueProvider>>,
}

impl RepositoryIssueService {
    pub(crate) fn new(providers: Vec<Box<dyn RepositoryIssueProvider>>) -> Self {
        Self { providers }
    }

    pub(crate) fn list_repository_issues(
        &self,
        repo_root: &Path,
        github_token: Option<String>,
    ) -> Result<IssueListResponse, IssueProviderError> {
        let Some(origin_remote_url) = origin_remote_url(repo_root)? else {
            return Ok(IssueListResponse {
                source: None,
                issues: Vec::new(),
                notice: Some("Repository has no origin remote.".to_owned()),
            });
        };

        for provider in &self.providers {
            let Some(source) = provider.resolve_source(repo_root, &origin_remote_url) else {
                continue;
            };

            let github_token = github_token
                .as_deref()
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(|token| SecretString::from(token.to_owned()));
            let issues = provider.list_issues(&source, github_token.as_ref())?;
            return Ok(IssueListResponse {
                source: Some(source.into_dto()),
                issues,
                notice: None,
            });
        }

        Ok(IssueListResponse {
            source: None,
            issues: Vec::new(),
            notice: Some("No supported issue provider resolved from the origin remote.".to_owned()),
        })
    }
}

impl Default for RepositoryIssueService {
    fn default() -> Self {
        Self::new(vec![
            Box::new(GitHubIssueProvider),
            Box::new(GitLabIssueProvider),
        ])
    }
}

struct GitHubIssueProvider;

impl RepositoryIssueProvider for GitHubIssueProvider {
    fn resolve_source(
        &self,
        _repo_root: &Path,
        origin_remote_url: &str,
    ) -> Option<ResolvedIssueSource> {
        let repository = github_repo_slug_from_remote_url(origin_remote_url)?;
        Some(ResolvedIssueSource {
            provider: IssueProviderKind::GitHub,
            repository: repository.clone(),
            url: Some(format!("https://github.com/{repository}/issues")),
            api_base_url: "https://api.github.com".to_owned(),
            gitlab_token_auth: GitLabTokenAuthPolicy::Disabled,
        })
    }

    fn list_issues(
        &self,
        source: &ResolvedIssueSource,
        github_token: Option<&SecretString>,
    ) -> Result<Vec<IssueDto>, IssueProviderError> {
        let (owner, repository) = source
            .repository
            .split_once('/')
            .ok_or_else(|| IssueProviderError::InvalidSlug(source.repository.clone()))?;
        let token = github_token.cloned().or_else(github_access_token_from_env);
        if token.is_none() {
            return list_github_issues_via_rest(source, owner, repository, None);
        }
        let mut issues = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let request_body = serde_json::json!({
                "query": GITHUB_ISSUES_GRAPHQL_QUERY,
                "variables": {
                    "owner": owner,
                    "repo": repository,
                    "after": after,
                },
            });
            let request_body_json = serde_json::to_string(&request_body).map_err(|error| {
                IssueProviderError::ApiRequest {
                    context: "failed to serialize GitHub GraphQL request".to_owned(),
                    reason: error.to_string(),
                }
            })?;
            let mut request = ureq::post(GITHUB_GRAPHQL_API_URL)
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .header("User-Agent", "Arbor");
            if let Some(token) = token.as_ref() {
                request = request.header(
                    "Authorization",
                    &format!("Bearer {}", token.expose_secret()),
                );
            }

            let response = request
                .config()
                .timeout_global(Some(ISSUE_REQUEST_TIMEOUT))
                .build()
                .send(&request_body_json);
            let mut response = match response {
                Ok(response) => response,
                Err(error) if is_github_graphql_forbidden(&error) => {
                    return list_github_issues_via_rest(source, owner, repository, token.as_ref());
                },
                Err(error) => {
                    return Err(IssueProviderError::ApiRequest {
                        context: "GitHub request failed".to_owned(),
                        reason: error.to_string(),
                    });
                },
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.body_mut().read_to_string().unwrap_or_default();
                return Err(IssueProviderError::ApiStatus {
                    provider: "GitHub".to_owned(),
                    status,
                    body,
                });
            }

            let body = response.body_mut().read_to_string().map_err(|error| {
                IssueProviderError::ApiRequest {
                    context: "failed to read GitHub response".to_owned(),
                    reason: error.to_string(),
                }
            })?;
            let response: GitHubGraphqlResponse<GitHubIssuesGraphqlData> =
                serde_json::from_str(&body).map_err(|error| IssueProviderError::ApiRequest {
                    context: "failed to decode GitHub GraphQL issues".to_owned(),
                    reason: error.to_string(),
                })?;
            if !response.errors.is_empty() {
                let messages = response
                    .errors
                    .into_iter()
                    .map(|error| error.message)
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(IssueProviderError::ApiRequest {
                    context: "GitHub GraphQL returned errors".to_owned(),
                    reason: messages,
                });
            }
            let connection = response
                .data
                .and_then(|data| data.repository)
                .map(|repository| repository.issues)
                .ok_or_else(|| {
                    IssueProviderError::Other(
                        "GitHub GraphQL response was missing repository issues".to_owned(),
                    )
                })?;

            issues.extend(connection.nodes.into_iter().flatten().map(|issue| {
                IssueDto {
                    id: issue.number.to_string(),
                    display_id: format!("#{}", issue.number),
                    title: issue.title.clone(),
                    state: issue.state,
                    url: Some(issue.url),
                    body: normalize_issue_body(issue.body),
                    suggested_worktree_name: issue_worktree_name(
                        &issue.number.to_string(),
                        &issue.title,
                    ),
                    updated_at: issue.updated_at,
                    labels: issue
                        .labels
                        .nodes
                        .into_iter()
                        .flatten()
                        .map(|label| IssueLabelDto {
                            name: label.name,
                            color: Some(label.color),
                        })
                        .collect(),
                    issue_type: issue.issue_type.map(|issue_type| IssueTypeDto {
                        name: issue_type.name,
                        color: Some(issue_type.color),
                    }),
                    linked_branch: None,
                    linked_review: None,
                }
            }));

            if !connection.page_info.has_next_page {
                break;
            }
            after = connection.page_info.end_cursor;
        }

        Ok(issues)
    }
}

fn list_github_issues_via_rest(
    source: &ResolvedIssueSource,
    owner: &str,
    repository: &str,
    token: Option<&SecretString>,
) -> Result<Vec<IssueDto>, IssueProviderError> {
    let mut issues = Vec::new();
    let mut page = 1usize;

    loop {
        let url = format!(
            "{}/repos/{}/{}/issues?state=open&sort=updated&direction=desc&per_page={}&page={page}",
            source.api_base_url,
            percent_encode(owner),
            percent_encode(repository),
            ISSUE_PAGE_SIZE,
        );
        let mut request = ureq::get(&url)
            .header("Accept", "application/json")
            .header("User-Agent", "Arbor");
        if let Some(token) = token {
            request = request.header(
                "Authorization",
                &format!("Bearer {}", token.expose_secret()),
            );
        }

        let mut response = request
            .config()
            .timeout_global(Some(ISSUE_REQUEST_TIMEOUT))
            .build()
            .call()
            .map_err(|error| IssueProviderError::ApiRequest {
                context: "GitHub request failed".to_owned(),
                reason: error.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.body_mut().read_to_string().unwrap_or_default();
            return Err(IssueProviderError::ApiStatus {
                provider: "GitHub".to_owned(),
                status,
                body,
            });
        }

        let body = response.body_mut().read_to_string().map_err(|error| {
            IssueProviderError::ApiRequest {
                context: "failed to read GitHub response".to_owned(),
                reason: error.to_string(),
            }
        })?;
        let page_items: Vec<GitHubIssueRestPayload> =
            serde_json::from_str(&body).map_err(|error| IssueProviderError::ApiRequest {
                context: "failed to decode GitHub issues".to_owned(),
                reason: error.to_string(),
            })?;
        let page_len = page_items.len();

        issues.extend(
            page_items
                .into_iter()
                .filter(|issue| issue.pull_request.is_none())
                .map(|issue| IssueDto {
                    id: issue.number.to_string(),
                    display_id: format!("#{}", issue.number),
                    title: issue.title.clone(),
                    state: issue.state,
                    url: Some(issue.html_url),
                    body: normalize_issue_body(issue.body),
                    suggested_worktree_name: issue_worktree_name(
                        &issue.number.to_string(),
                        &issue.title,
                    ),
                    updated_at: issue.updated_at,
                    labels: issue
                        .labels
                        .into_iter()
                        .map(|label| IssueLabelDto {
                            name: label.name,
                            color: Some(label.color),
                        })
                        .collect(),
                    issue_type: None,
                    linked_branch: None,
                    linked_review: None,
                }),
        );

        if page_len < ISSUE_PAGE_SIZE {
            break;
        }
        page += 1;
    }

    Ok(issues)
}

fn is_github_graphql_forbidden(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(status_code) => *status_code == 403,
        _ => false,
    }
}

struct GitLabIssueProvider;

impl RepositoryIssueProvider for GitLabIssueProvider {
    fn resolve_source(
        &self,
        _repo_root: &Path,
        origin_remote_url: &str,
    ) -> Option<ResolvedIssueSource> {
        let remote = parse_remote(origin_remote_url)?;
        let trusted_hosts = gitlab_trusted_hosts_from_env();
        resolve_gitlab_source(&remote, gitlab_instance_supports_issues, &trusted_hosts)
    }

    fn list_issues(
        &self,
        source: &ResolvedIssueSource,
        _github_token: Option<&SecretString>,
    ) -> Result<Vec<IssueDto>, IssueProviderError> {
        let token = gitlab_access_token_for_source(source);
        let mut issues = Vec::new();
        let mut page = 1usize;

        loop {
            let url = format!(
                "{}/projects/{}/issues?state=opened&order_by=updated_at&sort=desc&per_page={}&page={page}",
                source.api_base_url,
                percent_encode(&source.repository),
                ISSUE_PAGE_SIZE,
            );
            let mut request = ureq::get(&url)
                .header("Accept", "application/json")
                .header("User-Agent", "Arbor");
            if let Some(token) = token.as_ref() {
                request = request.header("PRIVATE-TOKEN", token.expose_secret());
            }

            let mut response = request
                .config()
                .timeout_global(Some(ISSUE_REQUEST_TIMEOUT))
                .build()
                .call()
                .map_err(|error| IssueProviderError::ApiRequest {
                    context: "GitLab request failed".to_owned(),
                    reason: error.to_string(),
                })?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.body_mut().read_to_string().unwrap_or_default();
                return Err(IssueProviderError::ApiStatus {
                    provider: "GitLab".to_owned(),
                    status,
                    body,
                });
            }

            let body = response.body_mut().read_to_string().map_err(|error| {
                IssueProviderError::ApiRequest {
                    context: "failed to read GitLab response".to_owned(),
                    reason: error.to_string(),
                }
            })?;
            let page_items: Vec<GitLabIssuePayload> =
                serde_json::from_str(&body).map_err(|error| IssueProviderError::ApiRequest {
                    context: "failed to decode GitLab issues".to_owned(),
                    reason: error.to_string(),
                })?;
            let page_len = page_items.len();

            issues.extend(page_items.into_iter().map(|issue| {
                IssueDto {
                    id: issue.id.to_string(),
                    display_id: format!("#{}", issue.iid),
                    title: issue.title.clone(),
                    state: issue.state,
                    url: issue.web_url,
                    body: normalize_issue_body(issue.description),
                    suggested_worktree_name: issue_worktree_name(
                        &issue.iid.to_string(),
                        &issue.title,
                    ),
                    updated_at: issue.updated_at,
                    labels: issue
                        .labels
                        .into_iter()
                        .map(|name| IssueLabelDto { name, color: None })
                        .collect(),
                    issue_type: issue
                        .issue_type
                        .map(|name| IssueTypeDto { name, color: None }),
                    linked_branch: None,
                    linked_review: None,
                }
            }));

            if page_len < ISSUE_PAGE_SIZE {
                break;
            }
            page += 1;
        }

        Ok(issues)
    }
}

#[derive(Debug, Deserialize)]
struct GitHubGraphqlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GitHubGraphqlError>,
}

#[derive(Debug, Deserialize)]
struct GitHubGraphqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubIssuesGraphqlData {
    repository: Option<GitHubIssuesGraphqlRepository>,
}

#[derive(Debug, Deserialize)]
struct GitHubIssuesGraphqlRepository {
    issues: GitHubIssuesGraphqlIssueConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubIssuesGraphqlIssueConnection {
    page_info: GitHubIssuesGraphqlPageInfo,
    #[serde(default)]
    nodes: Vec<Option<GitHubIssuesGraphqlIssue>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubIssuesGraphqlPageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubIssuesGraphqlIssue {
    number: u64,
    title: String,
    state: String,
    url: String,
    body: Option<String>,
    updated_at: Option<String>,
    issue_type: Option<GitHubIssuesGraphqlIssueType>,
    labels: GitHubIssuesGraphqlLabelConnection,
}

#[derive(Debug, Deserialize)]
struct GitHubIssuesGraphqlIssueType {
    name: String,
    color: String,
}

#[derive(Debug, Deserialize)]
struct GitHubIssuesGraphqlLabelConnection {
    #[serde(default)]
    nodes: Vec<Option<GitHubIssuesGraphqlLabel>>,
}

#[derive(Debug, Deserialize)]
struct GitHubIssuesGraphqlLabel {
    name: String,
    color: String,
}

#[derive(Debug, Deserialize)]
struct GitHubIssueRestPayload {
    number: u64,
    title: String,
    html_url: String,
    state: String,
    body: Option<String>,
    updated_at: Option<String>,
    #[serde(default)]
    labels: Vec<GitHubIssueRestLabel>,
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GitHubIssueRestLabel {
    name: String,
    color: String,
}

#[derive(Debug, Deserialize)]
struct GitLabIssuePayload {
    id: u64,
    iid: u64,
    title: String,
    state: String,
    web_url: Option<String>,
    description: Option<String>,
    updated_at: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    issue_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitLabMetadataPayload {
    version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteScheme {
    Http,
    Https,
}

impl RemoteScheme {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorityPortMode {
    Preserve,
    Strip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteHostKind {
    GitHub,
    GitLab,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitLabTokenAuthPolicy {
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteSpec {
    scheme: RemoteScheme,
    host: String,
    host_kind: RemoteHostKind,
    path: String,
}

impl RemoteSpec {
    fn base_url(&self) -> String {
        format!("{}://{}", self.scheme.as_str(), self.host)
    }

    fn host_name(&self) -> Option<&str> {
        authority_host_name(&self.host)
    }
}

fn origin_remote_url(repo_root: &Path) -> Result<Option<String>, IssueProviderError> {
    let repo = gix::open(repo_root).map_err(|error| {
        IssueProviderError::Other(format!(
            "failed to open repository `{}`: {error}",
            repo_root.display()
        ))
    })?;
    let remote = match repo.find_remote("origin") {
        Ok(remote) => remote,
        Err(_) => return Ok(None),
    };
    let Some(url) = remote.url(gix::remote::Direction::Fetch) else {
        return Ok(None);
    };
    let url = url.to_bstring().to_string();
    if url.is_empty() {
        Ok(None)
    } else {
        Ok(Some(url))
    }
}

fn github_repo_slug_from_remote_url(remote_url: &str) -> Option<String> {
    let remote = parse_remote(remote_url)?;
    github_repo_slug(&remote)
}

fn parse_remote(remote_url: &str) -> Option<RemoteSpec> {
    let trimmed = remote_url.trim();

    if let Some(rest) = trimmed.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/')?;
        return build_remote_spec(
            RemoteScheme::Https,
            authority,
            path,
            AuthorityPortMode::Strip,
        );
    }

    if let Some(rest) = trimmed.strip_prefix("https://") {
        let (authority, path) = rest.split_once('/')?;
        return build_remote_spec(
            RemoteScheme::Https,
            authority,
            path,
            AuthorityPortMode::Preserve,
        );
    }

    if let Some(rest) = trimmed.strip_prefix("http://") {
        let (authority, path) = rest.split_once('/')?;
        return build_remote_spec(
            RemoteScheme::Http,
            authority,
            path,
            AuthorityPortMode::Preserve,
        );
    }

    if let Some((authority, path)) = parse_scp_remote(trimmed) {
        return build_remote_spec(
            RemoteScheme::Https,
            authority,
            path,
            AuthorityPortMode::Strip,
        );
    }

    None
}

fn parse_scp_remote(remote_url: &str) -> Option<(&str, &str)> {
    let (authority, path) = remote_url.split_once(':')?;
    if authority.contains('/') || authority.contains("://") || !authority.contains('@') {
        return None;
    }
    Some((authority, path))
}

fn build_remote_spec(
    scheme: RemoteScheme,
    authority: &str,
    path: &str,
    port_mode: AuthorityPortMode,
) -> Option<RemoteSpec> {
    let host = sanitize_remote_authority(authority, port_mode)?;
    Some(RemoteSpec {
        scheme,
        host_kind: classify_remote_host(&host),
        host,
        path: normalize_remote_path(path)?,
    })
}

fn sanitize_remote_authority(authority: &str, port_mode: AuthorityPortMode) -> Option<String> {
    let trimmed = authority.trim();
    let without_userinfo = trimmed
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(trimmed)
        .trim();
    if without_userinfo.is_empty() {
        return None;
    }

    match port_mode {
        AuthorityPortMode::Preserve => Some(without_userinfo.to_owned()),
        AuthorityPortMode::Strip => strip_port_from_authority(without_userinfo),
    }
}

fn strip_port_from_authority(authority: &str) -> Option<String> {
    let trimmed = authority.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('[') {
        let (host, remainder) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        if remainder.is_empty() {
            return Some(format!("[{host}]"));
        }
        if remainder.starts_with(':') {
            return Some(format!("[{host}]"));
        }
        return None;
    }

    match trimmed.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty()
                && !port.is_empty()
                && port.chars().all(|character| character.is_ascii_digit()) =>
        {
            Some(host.to_owned())
        },
        _ => Some(trimmed.to_owned()),
    }
}

fn classify_remote_host(host: &str) -> RemoteHostKind {
    match authority_host_name(host)
        .unwrap_or(host)
        .to_ascii_lowercase()
        .as_str()
    {
        "github.com" => RemoteHostKind::GitHub,
        "gitlab.com" => RemoteHostKind::GitLab,
        _ => RemoteHostKind::Other,
    }
}

fn authority_host_name(authority: &str) -> Option<&str> {
    let trimmed = authority.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('[') {
        let (host, _) = rest.split_once(']')?;
        return (!host.is_empty()).then_some(host);
    }

    match trimmed.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty()
                && !port.is_empty()
                && port.chars().all(|character| character.is_ascii_digit()) =>
        {
            Some(host)
        },
        _ => Some(trimmed),
    }
}

fn normalize_remote_path(path: &str) -> Option<String> {
    let normalized = path.trim_end_matches('/');
    let path = normalized.strip_suffix(".git").unwrap_or(normalized);
    if path.is_empty() {
        None
    } else {
        Some(path.to_owned())
    }
}

fn issue_worktree_name(reference: &str, title: &str) -> String {
    let reference_slug = managed_worktree::sanitize_worktree_name(reference);
    let title_slug = managed_worktree::sanitize_worktree_name(title);

    let base_reference = if reference_slug.is_empty() {
        "issue".to_owned()
    } else if reference_slug
        .chars()
        .all(|character| character.is_ascii_digit() || character == '-')
    {
        format!("issue-{reference_slug}")
    } else {
        reference_slug
    };

    if title_slug.is_empty() {
        base_reference
    } else {
        format!("{base_reference}-{title_slug}")
    }
}

fn normalize_issue_body(body: Option<String>) -> Option<String> {
    body.and_then(|body| {
        if body.trim().is_empty() {
            None
        } else {
            Some(body)
        }
    })
}

fn github_repo_slug(remote: &RemoteSpec) -> Option<String> {
    if remote.host_kind != RemoteHostKind::GitHub {
        return None;
    }

    let (owner, repo_name) = remote.path.split_once('/')?;
    if owner.is_empty() || repo_name.is_empty() || repo_name.contains('/') {
        return None;
    }

    Some(format!("{owner}/{repo_name}"))
}

fn resolve_gitlab_source<F>(
    remote: &RemoteSpec,
    supports_custom_instance: F,
    trusted_hosts: &HashSet<String>,
) -> Option<ResolvedIssueSource>
where
    F: FnOnce(&RemoteSpec) -> bool,
{
    let is_gitlab = match remote.host_kind {
        RemoteHostKind::GitHub => false,
        RemoteHostKind::GitLab => true,
        RemoteHostKind::Other => supports_custom_instance(remote),
    };
    if !is_gitlab {
        return None;
    }

    let base_url = remote.base_url();
    Some(ResolvedIssueSource {
        provider: IssueProviderKind::GitLab,
        repository: remote.path.clone(),
        url: Some(format!("{base_url}/{}/-/issues", remote.path)),
        api_base_url: format!("{base_url}/api/v4"),
        gitlab_token_auth: gitlab_token_auth_policy(remote, trusted_hosts),
    })
}

fn gitlab_instance_supports_issues(remote: &RemoteSpec) -> bool {
    let url = format!("{}/api/v4/metadata", remote.base_url());
    let mut response = match ureq::get(&url)
        .header("Accept", "application/json")
        .header("User-Agent", "Arbor")
        .config()
        .timeout_global(Some(GITLAB_METADATA_PROBE_TIMEOUT))
        .build()
        .call()
    {
        Ok(response) => response,
        Err(_) => return false,
    };

    if !response.status().is_success() {
        return false;
    }

    let body = match response.body_mut().read_to_string() {
        Ok(body) => body,
        Err(_) => return false,
    };
    let metadata: GitLabMetadataPayload = match serde_json::from_str(&body) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };

    !metadata.version.trim().is_empty()
}

fn gitlab_token_auth_policy(
    remote: &RemoteSpec,
    trusted_hosts: &HashSet<String>,
) -> GitLabTokenAuthPolicy {
    if remote.scheme != RemoteScheme::Https {
        return GitLabTokenAuthPolicy::Disabled;
    }

    match remote.host_kind {
        RemoteHostKind::GitLab => GitLabTokenAuthPolicy::Enabled,
        RemoteHostKind::Other => remote
            .host_name()
            .map(|host| trusted_hosts.contains(&host.to_ascii_lowercase()))
            .map(|trusted| {
                if trusted {
                    GitLabTokenAuthPolicy::Enabled
                } else {
                    GitLabTokenAuthPolicy::Disabled
                }
            })
            .unwrap_or(GitLabTokenAuthPolicy::Disabled),
        RemoteHostKind::GitHub => GitLabTokenAuthPolicy::Disabled,
    }
}

fn gitlab_access_token_for_source(source: &ResolvedIssueSource) -> Option<SecretString> {
    match source.gitlab_token_auth {
        GitLabTokenAuthPolicy::Enabled => gitlab_access_token_from_env(),
        GitLabTokenAuthPolicy::Disabled => None,
    }
}

fn github_access_token_from_env() -> Option<SecretString> {
    env::var("GITHUB_TOKEN")
        .ok()
        .and_then(|value| non_empty_trimmed_str(&value).map(SecretString::from))
}

fn gitlab_access_token_from_env() -> Option<SecretString> {
    env::var("GITLAB_TOKEN")
        .ok()
        .or_else(|| env::var("ARBOR_GITLAB_TOKEN").ok())
        .and_then(|value| non_empty_trimmed_str(&value).map(SecretString::from))
}

fn gitlab_trusted_hosts_from_env() -> HashSet<String> {
    ["ARBOR_GITLAB_TRUSTED_HOSTS", "GITLAB_TRUSTED_HOSTS"]
        .into_iter()
        .filter_map(|name| env::var(name).ok())
        .flat_map(|value| {
            value
                .split(',')
                .filter_map(normalize_trusted_gitlab_host)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn normalize_trusted_gitlab_host(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    authority_host_name(authority).map(|host| host.to_ascii_lowercase())
}

fn non_empty_trimmed_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            },
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            },
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_repo_slug_from_remote_url_supports_common_formats() {
        assert_eq!(
            github_repo_slug_from_remote_url("git@github.com:penso/arbor.git"),
            Some("penso/arbor".to_owned())
        );
        assert_eq!(
            github_repo_slug_from_remote_url("https://github.com/penso/arbor"),
            Some("penso/arbor".to_owned())
        );
    }

    #[test]
    fn parse_remote_handles_gitlab_urls() {
        assert_eq!(
            parse_remote("git@gitlab.com:group/subgroup/arbor.git"),
            Some(RemoteSpec {
                scheme: RemoteScheme::Https,
                host: "gitlab.com".to_owned(),
                host_kind: RemoteHostKind::GitLab,
                path: "group/subgroup/arbor".to_owned(),
            })
        );
        assert_eq!(
            parse_remote("https://gitlab.example.com/group/arbor.git"),
            Some(RemoteSpec {
                scheme: RemoteScheme::Https,
                host: "gitlab.example.com".to_owned(),
                host_kind: RemoteHostKind::Other,
                path: "group/arbor".to_owned(),
            })
        );
        assert_eq!(
            parse_remote("ssh://git@gitlab.example.com:2222/group/arbor.git"),
            Some(RemoteSpec {
                scheme: RemoteScheme::Https,
                host: "gitlab.example.com".to_owned(),
                host_kind: RemoteHostKind::Other,
                path: "group/arbor".to_owned(),
            })
        );
        assert_eq!(
            parse_remote("alice@gitlab.example.com:group/arbor.git"),
            Some(RemoteSpec {
                scheme: RemoteScheme::Https,
                host: "gitlab.example.com".to_owned(),
                host_kind: RemoteHostKind::Other,
                path: "group/arbor".to_owned(),
            })
        );
        assert_eq!(
            parse_remote("https://gitlab.example.com:8443/group/arbor.git"),
            Some(RemoteSpec {
                scheme: RemoteScheme::Https,
                host: "gitlab.example.com:8443".to_owned(),
                host_kind: RemoteHostKind::Other,
                path: "group/arbor".to_owned(),
            })
        );
    }

    #[test]
    fn parse_remote_strips_credentials_from_https_authority() {
        assert_eq!(
            parse_remote("https://oauth2:secret-token@gitlab.example.com/group/arbor.git"),
            Some(RemoteSpec {
                scheme: RemoteScheme::Https,
                host: "gitlab.example.com".to_owned(),
                host_kind: RemoteHostKind::Other,
                path: "group/arbor".to_owned(),
            })
        );
    }

    #[test]
    fn resolve_gitlab_source_supports_custom_domains_via_probe() {
        let remote = parse_remote("https://code.company.com/group/arbor.git")
            .unwrap_or_else(|| panic!("remote should parse"));

        let source = resolve_gitlab_source(&remote, |_| true, &HashSet::new())
            .unwrap_or_else(|| panic!("custom GitLab instance should resolve"));

        assert_eq!(source.provider, IssueProviderKind::GitLab);
        assert_eq!(source.repository, "group/arbor");
        assert_eq!(
            source.url.as_deref(),
            Some("https://code.company.com/group/arbor/-/issues")
        );
        assert_eq!(source.api_base_url, "https://code.company.com/api/v4");
        assert_eq!(source.gitlab_token_auth, GitLabTokenAuthPolicy::Disabled);
    }

    #[test]
    fn resolve_gitlab_source_rejects_github_hosts_even_with_positive_probe() {
        let remote = parse_remote("git@github.com:penso/arbor.git")
            .unwrap_or_else(|| panic!("remote should parse"));

        assert_eq!(
            resolve_gitlab_source(&remote, |_| true, &HashSet::new()),
            None
        );
    }

    #[test]
    fn resolve_gitlab_source_allows_token_auth_for_gitlab_dot_com_https() {
        let remote = parse_remote("https://gitlab.com/group/arbor.git")
            .unwrap_or_else(|| panic!("remote should parse"));

        let source = resolve_gitlab_source(&remote, |_| true, &HashSet::new())
            .unwrap_or_else(|| panic!("gitlab.com should resolve"));

        assert_eq!(source.gitlab_token_auth, GitLabTokenAuthPolicy::Enabled);
    }

    #[test]
    fn resolve_gitlab_source_disables_token_auth_for_plain_http_remotes() {
        let remote = parse_remote("http://gitlab.example.com/group/arbor.git")
            .unwrap_or_else(|| panic!("remote should parse"));

        let source = resolve_gitlab_source(&remote, |_| true, &HashSet::new())
            .unwrap_or_else(|| panic!("GitLab http remote should still resolve"));

        assert_eq!(source.gitlab_token_auth, GitLabTokenAuthPolicy::Disabled);
    }

    #[test]
    fn resolve_gitlab_source_allows_token_auth_for_trusted_custom_hosts() {
        let remote = parse_remote("https://code.company.com/group/arbor.git")
            .unwrap_or_else(|| panic!("remote should parse"));
        let trusted_hosts = HashSet::from([String::from("code.company.com")]);

        let source = resolve_gitlab_source(&remote, |_| true, &trusted_hosts)
            .unwrap_or_else(|| panic!("trusted custom GitLab instance should resolve"));

        assert_eq!(source.gitlab_token_auth, GitLabTokenAuthPolicy::Enabled);
    }

    #[test]
    fn normalize_trusted_gitlab_host_accepts_urls_and_ports() {
        assert_eq!(
            normalize_trusted_gitlab_host("https://gitlab.example.com:8443/group/arbor"),
            Some("gitlab.example.com".to_owned())
        );
        assert_eq!(
            normalize_trusted_gitlab_host("code.company.com"),
            Some("code.company.com".to_owned())
        );
    }

    #[test]
    fn issue_worktree_name_uses_issue_prefix_for_numeric_references() {
        assert_eq!(
            issue_worktree_name("42", "Fix auth callback race"),
            "issue-42-fix-auth-callback-race"
        );
        assert_eq!(issue_worktree_name("42", ""), "issue-42");
    }

    #[test]
    fn normalize_issue_body_discards_empty_text() {
        assert_eq!(normalize_issue_body(None), None);
        assert_eq!(normalize_issue_body(Some(String::new())), None);
        assert_eq!(normalize_issue_body(Some("  \n\t  ".to_owned())), None);
        assert_eq!(
            normalize_issue_body(Some("Line one\n\n- bullet".to_owned())),
            Some("Line one\n\n- bullet".to_owned())
        );
    }
}
