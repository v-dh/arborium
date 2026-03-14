mod types;

pub use types::{
    AgentSessionDto, ChangedFileDto, CommitWorktreeRequest, CreateManagedWorktreeRequest,
    CreateTerminalRequest, CreateTerminalResponse, CreateWorktreeRequest, DeleteWorktreeRequest,
    GitActionResponse, HealthResponse, IssueDto, IssueListResponse, IssueReviewDto,
    IssueReviewKind, IssueSourceDto, ManagedWorktreePreviewRequest, ManagedWorktreePreviewResponse,
    PushWorktreeRequest, RepositoryDto, TerminalResizeRequest, TerminalSignalRequest, WorktreeDto,
    WorktreeMutationResponse,
};
use {
    arbor_core::{
        daemon::TerminalSnapshot,
        process::ProcessInfo,
        task::{TaskExecution, TaskInfo},
    },
    serde::{Serialize, de::DeserializeOwned},
    std::{env, path::PathBuf},
    thiserror::Error,
    types::ApiError,
};

const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:8787";

#[derive(Debug, Clone)]
pub struct DaemonClient {
    base_url: String,
    auth_token: Option<String>,
}

#[derive(Debug, Error)]
pub enum DaemonClientError {
    #[error("request failed: {0}")]
    Transport(String),
    #[error("daemon returned status {status}: {message}")]
    Api { status: u16, message: String },
    #[error("failed to parse daemon response: {0}")]
    Decode(String),
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::from_env()
    }
}

impl DaemonClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: normalize_base_url(base_url.into()),
            auth_token: None,
        }
    }

    pub fn from_env() -> Self {
        let mut client = Self::new(
            env::var("ARBOR_DAEMON_URL").unwrap_or_else(|_| DEFAULT_DAEMON_URL.to_owned()),
        );
        client.auth_token = env::var("ARBOR_DAEMON_AUTH_TOKEN")
            .ok()
            .and_then(|token| normalize_auth_token(Some(token)));
        client
    }

    pub fn with_auth_token(mut self, auth_token: Option<String>) -> Self {
        self.auth_token = normalize_auth_token(auth_token);
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn list_repositories(&self) -> Result<Vec<RepositoryDto>, DaemonClientError> {
        self.get_json("/api/v1/repositories")
    }

    pub fn list_worktrees(
        &self,
        repo_root: Option<&str>,
    ) -> Result<Vec<WorktreeDto>, DaemonClientError> {
        match repo_root {
            Some(repo_root) => self.get_json(&format!(
                "/api/v1/worktrees?repo_root={}",
                encode_query_value(repo_root)
            )),
            None => self.get_json("/api/v1/worktrees"),
        }
    }

    pub fn create_worktree(
        &self,
        request: &CreateWorktreeRequest,
    ) -> Result<WorktreeMutationResponse, DaemonClientError> {
        self.post_json("/api/v1/worktrees", request)
    }

    pub fn preview_managed_worktree(
        &self,
        request: &ManagedWorktreePreviewRequest,
    ) -> Result<ManagedWorktreePreviewResponse, DaemonClientError> {
        self.post_json("/api/v1/worktrees/managed/preview", request)
    }

    pub fn create_managed_worktree(
        &self,
        request: &CreateManagedWorktreeRequest,
    ) -> Result<WorktreeMutationResponse, DaemonClientError> {
        self.post_json("/api/v1/worktrees/managed", request)
    }

    pub fn delete_worktree(
        &self,
        request: &DeleteWorktreeRequest,
    ) -> Result<WorktreeMutationResponse, DaemonClientError> {
        self.post_json("/api/v1/worktrees/delete", request)
    }

    pub fn list_issues(&self, repo_root: &str) -> Result<IssueListResponse, DaemonClientError> {
        self.get_json(&format!(
            "/api/v1/issues?repo_root={}",
            encode_query_value(repo_root)
        ))
    }

    pub fn list_changed_files(&self, path: &str) -> Result<Vec<ChangedFileDto>, DaemonClientError> {
        self.get_json(&format!(
            "/api/v1/worktrees/changes?path={}",
            encode_query_value(path)
        ))
    }

    pub fn commit_worktree(
        &self,
        request: &CommitWorktreeRequest,
    ) -> Result<GitActionResponse, DaemonClientError> {
        self.post_json("/api/v1/worktrees/commit", request)
    }

    pub fn push_worktree(
        &self,
        request: &PushWorktreeRequest,
    ) -> Result<GitActionResponse, DaemonClientError> {
        self.post_json("/api/v1/worktrees/push", request)
    }

    pub fn health(&self) -> Result<HealthResponse, DaemonClientError> {
        self.get_json("/api/v1/health")
    }

    pub fn list_terminals(
        &self,
    ) -> Result<Vec<arbor_core::daemon::DaemonSessionRecord>, DaemonClientError> {
        self.get_json("/api/v1/terminals")
    }

    pub fn create_terminal(
        &self,
        request: &CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse, DaemonClientError> {
        self.post_json("/api/v1/terminals", request)
    }

    pub fn read_terminal_output(
        &self,
        session_id: &str,
        max_lines: Option<usize>,
    ) -> Result<TerminalSnapshot, DaemonClientError> {
        let path = match max_lines {
            Some(max_lines) => format!(
                "/api/v1/terminals/{}/snapshot?max_lines={}",
                encode_path_segment(session_id),
                max_lines.clamp(1, 2000)
            ),
            None => format!(
                "/api/v1/terminals/{}/snapshot",
                encode_path_segment(session_id)
            ),
        };
        self.get_json(&path)
    }

    pub fn write_terminal_input(
        &self,
        session_id: &str,
        data: &[u8],
    ) -> Result<(), DaemonClientError> {
        self.request_no_content(
            "POST",
            &format!(
                "/api/v1/terminals/{}/write",
                encode_path_segment(session_id)
            ),
            Some("application/octet-stream"),
            data,
        )
    }

    pub fn resize_terminal(
        &self,
        session_id: &str,
        request: &TerminalResizeRequest,
    ) -> Result<(), DaemonClientError> {
        self.request_json_no_content(
            "POST",
            &format!(
                "/api/v1/terminals/{}/resize",
                encode_path_segment(session_id)
            ),
            request,
        )
    }

    pub fn signal_terminal(
        &self,
        session_id: &str,
        request: &TerminalSignalRequest,
    ) -> Result<(), DaemonClientError> {
        self.request_json_no_content(
            "POST",
            &format!(
                "/api/v1/terminals/{}/signal",
                encode_path_segment(session_id)
            ),
            request,
        )
    }

    pub fn detach_terminal(&self, session_id: &str) -> Result<(), DaemonClientError> {
        self.request_no_content(
            "POST",
            &format!(
                "/api/v1/terminals/{}/detach",
                encode_path_segment(session_id)
            ),
            None,
            &[],
        )
    }

    pub fn kill_terminal(&self, session_id: &str) -> Result<(), DaemonClientError> {
        self.request_no_content(
            "DELETE",
            &format!("/api/v1/terminals/{}", encode_path_segment(session_id)),
            None,
            &[],
        )
    }

    pub fn list_agent_activity(&self) -> Result<Vec<AgentSessionDto>, DaemonClientError> {
        self.get_json("/api/v1/agent/activity")
    }

    pub fn list_processes(&self) -> Result<Vec<ProcessInfo>, DaemonClientError> {
        self.get_json("/api/v1/processes")
    }

    pub fn start_all_processes(&self) -> Result<Vec<ProcessInfo>, DaemonClientError> {
        self.post_empty_json("/api/v1/processes/start-all")
    }

    pub fn stop_all_processes(&self) -> Result<Vec<ProcessInfo>, DaemonClientError> {
        self.post_empty_json("/api/v1/processes/stop-all")
    }

    pub fn start_process(&self, name: &str) -> Result<ProcessInfo, DaemonClientError> {
        self.post_empty_json(&format!(
            "/api/v1/processes/{}/start",
            encode_path_segment(name)
        ))
    }

    pub fn stop_process(&self, name: &str) -> Result<ProcessInfo, DaemonClientError> {
        self.post_empty_json(&format!(
            "/api/v1/processes/{}/stop",
            encode_path_segment(name)
        ))
    }

    pub fn restart_process(&self, name: &str) -> Result<ProcessInfo, DaemonClientError> {
        self.post_empty_json(&format!(
            "/api/v1/processes/{}/restart",
            encode_path_segment(name)
        ))
    }

    pub fn symphony_state(&self) -> Result<arbor_symphony::RuntimeSnapshot, DaemonClientError> {
        self.get_json("/api/v1/symphony/state")
    }

    pub fn symphony_issue(
        &self,
        issue_identifier: &str,
    ) -> Result<arbor_symphony::IssueRuntimeSnapshot, DaemonClientError> {
        self.get_json(&format!(
            "/api/v1/symphony/{}",
            encode_path_segment(issue_identifier)
        ))
    }

    pub fn refresh_symphony(&self) -> Result<serde_json::Value, DaemonClientError> {
        self.post_empty_json("/api/v1/symphony/refresh")
    }

    pub fn list_tasks(&self) -> Result<Vec<TaskInfo>, DaemonClientError> {
        self.get_json("/api/v1/tasks")
    }

    pub fn run_task(&self, name: &str) -> Result<TaskInfo, DaemonClientError> {
        self.post_empty_json(&format!("/api/v1/tasks/{}/run", encode_path_segment(name)))
    }

    pub fn task_history(&self, name: &str) -> Result<Vec<TaskExecution>, DaemonClientError> {
        self.get_json(&format!(
            "/api/v1/tasks/{}/history",
            encode_path_segment(name)
        ))
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, DaemonClientError> {
        let response = self
            .get_request(path)
            .header("Accept", "application/json")
            .call()
            .map_err(map_ureq_error)?;
        decode_json_response(response)
    }

    fn post_json<TReq: Serialize, TResp: DeserializeOwned>(
        &self,
        path: &str,
        request: &TReq,
    ) -> Result<TResp, DaemonClientError> {
        let response = self
            .post_request(path)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(serialize_json_body(request)?.as_bytes())
            .map_err(map_ureq_error)?;
        decode_json_response(response)
    }

    fn request_json_no_content<TReq: Serialize>(
        &self,
        method: &str,
        path: &str,
        request: &TReq,
    ) -> Result<(), DaemonClientError> {
        let body = serialize_json_body(request)?;
        match method {
            "POST" => {
                let response = self
                    .post_request(path)
                    .header("Content-Type", "application/json")
                    .send(body.as_bytes())
                    .map_err(map_ureq_error)?;
                expect_no_content(response)
            },
            other => Err(DaemonClientError::Transport(format!(
                "unsupported HTTP method `{other}`"
            ))),
        }
    }

    fn post_empty_json<TResp: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<TResp, DaemonClientError> {
        let response = self
            .post_request(path)
            .header("Accept", "application/json")
            .send_empty()
            .map_err(map_ureq_error)?;
        decode_json_response(response)
    }

    fn request_no_content(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<(), DaemonClientError> {
        let response = match method {
            "POST" => {
                let mut request = self.post_request(path);
                if let Some(content_type) = content_type {
                    request = request.header("Content-Type", content_type);
                }
                if body.is_empty() {
                    request.send_empty().map_err(map_ureq_error)?
                } else {
                    request.send(body).map_err(map_ureq_error)?
                }
            },
            "DELETE" => {
                let mut request = self.delete_request(path);
                if let Some(content_type) = content_type {
                    request = request.header("Content-Type", content_type);
                }
                if body.is_empty() {
                    request.call().map_err(map_ureq_error)?
                } else {
                    request
                        .force_send_body()
                        .send(body)
                        .map_err(map_ureq_error)?
                }
            },
            other => {
                return Err(DaemonClientError::Transport(format!(
                    "unsupported HTTP method `{other}`"
                )));
            },
        };

        expect_no_content(response)
    }

    fn get_request(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.apply_auth(ureq::get(&format!("{}{}", self.base_url, path)))
    }

    fn post_request(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.apply_auth(ureq::post(&format!("{}{}", self.base_url, path)))
    }

    fn delete_request(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.apply_auth(ureq::delete(&format!("{}{}", self.base_url, path)))
    }

    fn apply_auth<B>(&self, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        match self.auth_token.as_deref() {
            Some(token) => request.header("Authorization", &format!("Bearer {token}")),
            None => request,
        }
    }
}

fn serialize_json_body<T: Serialize>(request: &T) -> Result<String, DaemonClientError> {
    serde_json::to_string(request).map_err(|error| DaemonClientError::Transport(error.to_string()))
}

fn decode_json_response<T: DeserializeOwned>(
    response: ureq::http::Response<ureq::Body>,
) -> Result<T, DaemonClientError> {
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .read_to_string()
        .map_err(|error| DaemonClientError::Transport(error.to_string()))?;

    if (200..300).contains(&status) {
        return serde_json::from_str(&body)
            .map_err(|error| DaemonClientError::Decode(format!("{error}: {body}")));
    }

    Err(decode_api_error(status, &body))
}

fn response_into_error(response: ureq::http::Response<ureq::Body>) -> DaemonClientError {
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .read_to_string()
        .unwrap_or_else(|error| format!("failed to read response body: {error}"));
    decode_api_error(status, &body)
}

fn decode_api_error(status: u16, body: &str) -> DaemonClientError {
    let message = serde_json::from_str::<ApiError>(body)
        .map(|error| error.error)
        .ok()
        .or_else(|| {
            let trimmed = body.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_owned())
        })
        .unwrap_or_else(|| "unknown daemon error".to_owned());

    DaemonClientError::Api { status, message }
}

fn map_ureq_error(error: ureq::Error) -> DaemonClientError {
    match error {
        ureq::Error::StatusCode(status) => DaemonClientError::Api {
            status,
            message: "request failed".to_owned(),
        },
        other => DaemonClientError::Transport(other.to_string()),
    }
}

fn expect_no_content(response: ureq::http::Response<ureq::Body>) -> Result<(), DaemonClientError> {
    if response.status().as_u16() == 204 {
        return Ok(());
    }

    Err(response_into_error(response))
}

fn normalize_base_url(url: String) -> String {
    url.trim_end_matches('/').to_owned()
}

fn normalize_auth_token(auth_token: Option<String>) -> Option<String> {
    auth_token.and_then(|token| {
        let trimmed = token.trim();
        (!trimmed.is_empty()).then_some(trimmed.to_owned())
    })
}

fn encode_path_segment(input: &str) -> String {
    percent_encode(input)
}

fn encode_query_value(input: &str) -> String {
    percent_encode(input)
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

pub fn read_json_text_resource<T: Serialize>(value: &T) -> Result<String, DaemonClientError> {
    serde_json::to_string_pretty(value)
        .map_err(|error| DaemonClientError::Decode(error.to_string()))
}

pub fn default_mcp_resources() -> [(&'static str, &'static str, &'static str); 7] {
    [
        ("arbor://health", "health", "Daemon health and version"),
        (
            "arbor://repositories",
            "repositories",
            "Tracked repository roots known to Arbor",
        ),
        (
            "arbor://worktrees",
            "worktrees",
            "Current worktree snapshot",
        ),
        ("arbor://processes", "processes", "Managed process snapshot"),
        ("arbor://tasks", "tasks", "Scheduled task snapshot"),
        (
            "arbor://terminals",
            "terminals",
            "Daemon-managed terminal sessions",
        ),
        (
            "arbor://agent-activity",
            "agent-activity",
            "Current AI agent activity snapshot",
        ),
    ]
}

pub fn default_mcp_resource_templates() -> [(&'static str, &'static str, &'static str); 2] {
    [
        (
            "arbor://worktree-changes/{encoded_path}",
            "worktree-changes",
            "Changed files for a worktree path encoded as a single URI segment",
        ),
        (
            "arbor://terminal-snapshot/{session_id}",
            "terminal-snapshot",
            "Terminal snapshot for a daemon session id",
        ),
    ]
}

pub fn parse_worktree_changes_resource(uri: &str) -> Option<PathBuf> {
    uri.strip_prefix("arbor://worktree-changes/")
        .map(percent_decode)
        .map(PathBuf::from)
}

pub fn parse_terminal_snapshot_resource(uri: &str) -> Option<String> {
    uri.strip_prefix("arbor://terminal-snapshot/")
        .map(percent_decode)
}

fn percent_decode(input: &str) -> String {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &input[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                output.push(value);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[cfg(test)]
mod tests {
    use {
        super::DaemonClient,
        std::{
            io::{Read, Write},
            net::TcpListener,
            sync::mpsc,
            thread,
            time::Duration,
        },
    };

    #[derive(Debug)]
    struct CapturedRequest {
        headers: String,
        body: Vec<u8>,
    }

    #[test]
    fn terminal_write_sends_raw_octets() -> Result<(), Box<dyn std::error::Error>> {
        let (base_url, receiver, handle) = spawn_capture_server(
            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".to_owned(),
        )?;
        let client = DaemonClient::new(base_url);
        let payload = vec![0xff, 0x00, 0x1b, b'[', b'2', b'J'];

        client.write_terminal_input("daemon-raw", &payload)?;

        let captured = receiver.recv_timeout(Duration::from_secs(2))?;
        assert!(
            captured
                .headers
                .starts_with("POST /api/v1/terminals/daemon-raw/write HTTP/1.1\r\n")
        );
        assert!(
            captured
                .headers
                .to_ascii_lowercase()
                .contains("content-type: application/octet-stream\r\n")
        );
        assert_eq!(captured.body, payload);
        let _ = handle.join().map_err(|_| "capture server panicked")?;
        Ok(())
    }

    #[test]
    fn authenticated_requests_send_bearer_header() -> Result<(), Box<dyn std::error::Error>> {
        let (base_url, receiver, handle) = spawn_capture_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 32\r\n\r\n{\"status\":\"ok\",\"version\":\"test\"}".to_owned(),
        )?;
        let client = DaemonClient::new(base_url).with_auth_token(Some("secret-token".to_owned()));

        let _ = client.health()?;

        let captured = receiver.recv_timeout(Duration::from_secs(2))?;
        assert!(
            captured
                .headers
                .to_ascii_lowercase()
                .contains("authorization: bearer secret-token\r\n")
        );
        let _ = handle.join().map_err(|_| "capture server panicked")?;
        Ok(())
    }

    fn spawn_capture_server(
        response: String,
    ) -> Result<
        (
            String,
            mpsc::Receiver<CapturedRequest>,
            thread::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>>,
        ),
        Box<dyn std::error::Error>,
    > {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (sender, receiver) = mpsc::channel();

        let handle = thread::spawn(
            move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
                let (mut stream, _) = listener.accept()?;
                let mut raw = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = stream.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buffer[..read]);
                    if let Some(request) = parse_http_request(&raw) {
                        sender.send(request)?;
                        stream.write_all(response.as_bytes())?;
                        stream.flush()?;
                        break;
                    }
                }
                Ok(())
            },
        );

        Ok((format!("http://{}", address), receiver, handle))
    }

    fn parse_http_request(raw: &[u8]) -> Option<CapturedRequest> {
        let raw_text = String::from_utf8_lossy(raw);
        let header_end = raw_text.find("\r\n\r\n")?;
        let headers = &raw_text[..header_end + 4];
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        let body_start = header_end + 4;
        if raw.len() < body_start + content_length {
            return None;
        }
        Some(CapturedRequest {
            headers: headers.to_owned(),
            body: raw[body_start..body_start + content_length].to_vec(),
        })
    }
}
