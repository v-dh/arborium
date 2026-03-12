use std::{
    cmp::Reverse,
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
    process::Command,
    time::UNIX_EPOCH,
};

pub const DEFAULT_RECENT_AGENT_SESSION_LIMIT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentSessionProviderKind {
    Claude,
    Codex,
    Pi,
    OpenCode,
}

impl AgentSessionProviderKind {
    fn order(self) -> usize {
        match self {
            Self::Claude => 0,
            Self::Codex => 1,
            Self::Pi => 2,
            Self::OpenCode => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Pi => "Pi",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionSummary {
    pub provider: AgentSessionProviderKind,
    pub id: String,
    pub title: String,
    pub timestamp_unix_ms: Option<u64>,
    pub message_count: usize,
}

pub trait AgentSessionProvider {
    fn provider(&self) -> AgentSessionProviderKind;
    fn list_sessions(&self, worktree_path: &Path, limit: usize) -> Vec<AgentSessionSummary>;
}

/// Extract the initial user prompt from the most recent Claude, Codex, or Pi
/// session associated with `worktree_path`.
///
/// Tries Claude first (O(1) directory lookup), then Pi (same), then falls back
/// to scanning Codex session files (date-ordered, up to 30 days back).
pub fn extract_agent_task(worktree_path: &Path) -> Option<String> {
    extract_claude_task(worktree_path)
        .or_else(|| extract_pi_task(worktree_path))
        .or_else(|| extract_codex_task(worktree_path))
}

pub fn recent_agent_sessions(worktree_path: &Path, limit: usize) -> Vec<AgentSessionSummary> {
    let limit = if limit == 0 {
        DEFAULT_RECENT_AGENT_SESSION_LIMIT
    } else {
        limit
    };
    let claude = ClaudeSessionProvider;
    let codex = CodexSessionProvider;
    let pi = PiSessionProvider;
    let opencode = OpenCodeSessionProvider;
    let providers: [&dyn AgentSessionProvider; 4] = [&claude, &codex, &pi, &opencode];

    let mut sessions = providers
        .into_iter()
        .flat_map(|provider| provider.list_sessions(worktree_path, limit))
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .timestamp_unix_ms
            .cmp(&left.timestamp_unix_ms)
            .then_with(|| left.provider.order().cmp(&right.provider.order()))
            .then_with(|| left.title.cmp(&right.title))
    });
    sessions.truncate(limit);
    sessions
}

struct ClaudeSessionProvider;

impl AgentSessionProvider for ClaudeSessionProvider {
    fn provider(&self) -> AgentSessionProviderKind {
        AgentSessionProviderKind::Claude
    }

    fn list_sessions(&self, worktree_path: &Path, limit: usize) -> Vec<AgentSessionSummary> {
        let Some(home) = home_dir() else {
            return Vec::new();
        };
        let key = claude_project_key(worktree_path);
        let project_dir = home.join(".claude").join("projects").join(key);
        list_sorted_jsonl_files(&project_dir)
            .into_iter()
            .filter_map(|path| parse_claude_session_summary(&path))
            .take(limit)
            .collect()
    }
}

struct CodexSessionProvider;

impl AgentSessionProvider for CodexSessionProvider {
    fn provider(&self) -> AgentSessionProviderKind {
        AgentSessionProviderKind::Codex
    }

    fn list_sessions(&self, worktree_path: &Path, limit: usize) -> Vec<AgentSessionSummary> {
        let Some(home) = home_dir() else {
            return Vec::new();
        };
        let sessions_root = home.join(".codex").join("sessions");
        let Some(day_dirs) = collect_day_dirs(&sessions_root, 30) else {
            return Vec::new();
        };

        let mut sessions = Vec::new();
        for day_dir in day_dirs {
            let mut files = list_sorted_jsonl_files(&day_dir);
            files.sort_by_key(|path| Reverse(path.file_name().map(|name| name.to_os_string())));
            for path in files {
                if let Some(summary) = parse_codex_session_summary(&path, worktree_path) {
                    sessions.push(summary);
                    if sessions.len() >= limit {
                        return sessions;
                    }
                }
            }
        }

        sessions
    }
}

struct PiSessionProvider;

impl AgentSessionProvider for PiSessionProvider {
    fn provider(&self) -> AgentSessionProviderKind {
        AgentSessionProviderKind::Pi
    }

    fn list_sessions(&self, worktree_path: &Path, limit: usize) -> Vec<AgentSessionSummary> {
        let Some(home) = home_dir() else {
            return Vec::new();
        };
        let key = pi_project_key(worktree_path);
        let project_dir = home.join(".pi").join("agent").join("sessions").join(key);
        list_sorted_jsonl_files(&project_dir)
            .into_iter()
            .filter_map(|path| parse_pi_session_summary(&path))
            .take(limit)
            .collect()
    }
}

struct OpenCodeSessionProvider;

impl AgentSessionProvider for OpenCodeSessionProvider {
    fn provider(&self) -> AgentSessionProviderKind {
        AgentSessionProviderKind::OpenCode
    }

    fn list_sessions(&self, worktree_path: &Path, limit: usize) -> Vec<AgentSessionSummary> {
        if limit == 0 {
            return Vec::new();
        }

        let output = Command::new("opencode")
            .arg("session")
            .arg("list")
            .arg("--format")
            .arg("json")
            .arg("-n")
            .arg((limit.saturating_mul(4)).to_string())
            .output();
        let Ok(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }

        parse_opencode_session_list(
            &String::from_utf8_lossy(&output.stdout),
            worktree_path,
            limit,
        )
    }
}

/// Encode a worktree path the same way Claude CLI names its project dirs:
/// replace `/` and `.` with `-`.
fn claude_project_key(worktree_path: &Path) -> String {
    let s = worktree_path.to_string_lossy();
    s.chars()
        .map(|c| {
            if c == '/' || c == '.' {
                '-'
            } else {
                c
            }
        })
        .collect()
}

/// Look up the most recent `.jsonl` in `~/.claude/projects/{key}/` and extract
/// the first `"type": "user"` message content.
fn extract_claude_task(worktree_path: &Path) -> Option<String> {
    let home = home_dir()?;
    let key = claude_project_key(worktree_path);
    let project_dir = home.join(".claude").join("projects").join(&key);
    let newest = list_sorted_jsonl_files(&project_dir).into_iter().next()?;
    extract_claude_user_prompt(&newest)
}

fn parse_claude_session_summary(path: &Path) -> Option<AgentSessionSummary> {
    let file = fs::File::open(path).ok()?;
    let reader = io::BufReader::new(file);
    let mut title = None;
    let mut message_count = 0usize;

    for line in reader.lines() {
        let line = line.ok()?;
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        match value.get("type").and_then(|entry| entry.as_str()) {
            Some("user") => {
                message_count += 1;
                if title.is_none() {
                    title = extract_claude_prompt_from_value(&value);
                }
            },
            Some("assistant") => {
                message_count += 1;
            },
            _ => {},
        }
    }

    Some(AgentSessionSummary {
        provider: AgentSessionProviderKind::Claude,
        id: session_id_from_path(path),
        title: title?,
        timestamp_unix_ms: file_modified_unix_ms(path),
        message_count,
    })
}

fn extract_claude_prompt_from_value(value: &serde_json::Value) -> Option<String> {
    let content = value.pointer("/message/content")?;
    extract_text_from_content_blocks(content).map(|text| truncate_prompt(&text))
}

/// Read through a Claude `.jsonl` session file and return the text of the first
/// `"type": "user"` entry.
fn extract_claude_user_prompt(path: &Path) -> Option<String> {
    parse_claude_session_summary(path).map(|summary| summary.title)
}

/// Encode a worktree path the same way Pi names its session dirs.
fn pi_project_key(worktree_path: &Path) -> String {
    let path = worktree_path.to_string_lossy();
    let trimmed = path.trim_start_matches(['/', '\\']);
    format!("--{}--", trimmed.replace(['/', '\\', ':'], "-"))
}

/// Look up the most recent `.jsonl` in `~/.pi/agent/sessions/{key}/` and
/// extract the first user message content.
fn extract_pi_task(worktree_path: &Path) -> Option<String> {
    let home = home_dir()?;
    let key = pi_project_key(worktree_path);
    let project_dir = home.join(".pi").join("agent").join("sessions").join(&key);
    let newest = list_sorted_jsonl_files(&project_dir).into_iter().next()?;
    extract_pi_user_prompt(&newest)
}

fn parse_pi_session_summary(path: &Path) -> Option<AgentSessionSummary> {
    let file = fs::File::open(path).ok()?;
    let reader = io::BufReader::new(file);
    let mut title = None;
    let mut message_count = 0usize;

    for line in reader.lines() {
        let line = line.ok()?;
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        if value.get("type").and_then(|entry| entry.as_str()) != Some("message") {
            continue;
        }

        let role = value
            .pointer("/message/role")
            .and_then(|entry| entry.as_str());
        if matches!(role, Some("user" | "assistant")) {
            message_count += 1;
        }
        if title.is_none() && role == Some("user") {
            title = value
                .pointer("/message/content")
                .and_then(extract_text_from_content_blocks)
                .map(|text| truncate_prompt(&text));
        }
    }

    Some(AgentSessionSummary {
        provider: AgentSessionProviderKind::Pi,
        id: session_id_from_path(path),
        title: title?,
        timestamp_unix_ms: file_modified_unix_ms(path),
        message_count,
    })
}

/// Read through a Pi `.jsonl` session file and return the text of the first
/// `message.role == "user"` entry.
fn extract_pi_user_prompt(path: &Path) -> Option<String> {
    parse_pi_session_summary(path).map(|summary| summary.title)
}

/// Scan Codex session dirs (newest day first, up to 30 days) for a session
/// whose `session_meta.payload.cwd` matches `worktree_path`, and extract the
/// first `event_msg` with `payload.type == "user_message"`.
fn extract_codex_task(worktree_path: &Path) -> Option<String> {
    recent_agent_sessions_from_provider(&CodexSessionProvider, worktree_path, 1)
        .into_iter()
        .next()
        .map(|summary| summary.title)
}

fn parse_codex_session_summary(path: &Path, worktree_path: &Path) -> Option<AgentSessionSummary> {
    let file = fs::File::open(path).ok()?;
    let reader = io::BufReader::new(file);

    let worktree_str = worktree_path.to_string_lossy();
    let mut cwd_matches = false;
    let mut session_id = session_id_from_path(path);
    let mut event_title = None;
    let mut response_title = None;
    let mut response_message_count = 0usize;
    let mut event_message_count = 0usize;

    for line in reader.lines() {
        let line = line.ok()?;
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        match value
            .get("type")
            .and_then(|entry| entry.as_str())
            .unwrap_or_default()
        {
            "session_meta" => {
                let cwd = value
                    .pointer("/payload/cwd")
                    .and_then(|entry| entry.as_str())
                    .unwrap_or_default();
                if cwd != worktree_str {
                    return None;
                }
                cwd_matches = true;
                if let Some(id) = value
                    .pointer("/payload/id")
                    .and_then(|entry| entry.as_str())
                {
                    session_id = id.to_owned();
                }
            },
            "response_item" if cwd_matches => {
                if value
                    .pointer("/payload/type")
                    .and_then(|entry| entry.as_str())
                    == Some("message")
                {
                    let role = value
                        .pointer("/payload/role")
                        .and_then(|entry| entry.as_str());
                    if matches!(role, Some("user" | "assistant")) {
                        response_message_count += 1;
                    }
                    if response_title.is_none() && role == Some("user") {
                        response_title = value
                            .pointer("/payload/content")
                            .and_then(extract_codex_prompt_from_content)
                            .map(|text| truncate_prompt(&text));
                    }
                }
            },
            "event_msg" if cwd_matches => {
                let payload_type = value
                    .pointer("/payload/type")
                    .and_then(|entry| entry.as_str())
                    .unwrap_or_default();
                if payload_type == "user_message" || payload_type == "agent_message" {
                    event_message_count += 1;
                }
                if event_title.is_none() && payload_type == "user_message" {
                    event_title = value
                        .pointer("/payload/message")
                        .and_then(|entry| entry.as_str())
                        .map(truncate_prompt);
                }
            },
            _ => {},
        }
    }

    if !cwd_matches {
        return None;
    }

    Some(AgentSessionSummary {
        provider: AgentSessionProviderKind::Codex,
        id: session_id,
        title: event_title.or(response_title)?,
        timestamp_unix_ms: file_modified_unix_ms(path),
        message_count: response_message_count.max(event_message_count),
    })
}

fn extract_codex_prompt_from_content(content: &serde_json::Value) -> Option<String> {
    let blocks = content.as_array()?;
    for block in blocks {
        if let Some(text) = block.get("text").and_then(|entry| entry.as_str()) {
            return Some(text.to_owned());
        }
        if let Some(text) = block.get("message").and_then(|entry| entry.as_str()) {
            return Some(text.to_owned());
        }
        if let Some(text) = block
            .pointer("/content/0/text")
            .and_then(|entry| entry.as_str())
        {
            return Some(text.to_owned());
        }
    }
    None
}

/// Collect all `YYYY/MM/DD` directories under `sessions_root`, sorted newest
/// first, limited to `max_days` entries.
fn collect_day_dirs(sessions_root: &Path, max_days: usize) -> Option<Vec<PathBuf>> {
    let mut year_dirs: Vec<_> = fs::read_dir(sessions_root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .collect();
    year_dirs.sort_by(|left, right| right.cmp(left));

    let mut day_dirs = Vec::new();
    'outer: for year_dir in &year_dirs {
        let mut month_dirs: Vec<_> = fs::read_dir(year_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.path())
            .collect();
        month_dirs.sort_by(|left, right| right.cmp(left));

        for month_dir in &month_dirs {
            let mut days: Vec<_> = fs::read_dir(month_dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.path())
                .collect();
            days.sort_by(|left, right| right.cmp(left));

            for day in days {
                day_dirs.push(day);
                if day_dirs.len() >= max_days {
                    break 'outer;
                }
            }
        }
    }
    Some(day_dirs)
}

fn parse_opencode_session_list(
    raw: &str,
    worktree_path: &Path,
    limit: usize,
) -> Vec<AgentSessionSummary> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let Some(entries) = value.as_array() else {
        return Vec::new();
    };

    let worktree_str = worktree_path.to_string_lossy();
    let mut sessions = entries
        .iter()
        .filter_map(|entry| parse_opencode_session_entry(entry, &worktree_str))
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| right.timestamp_unix_ms.cmp(&left.timestamp_unix_ms));
    sessions.truncate(limit);
    sessions
}

fn parse_opencode_session_entry(
    entry: &serde_json::Value,
    worktree_str: &str,
) -> Option<AgentSessionSummary> {
    let cwd = json_string(entry, &[
        "cwd",
        "path",
        "projectPath",
        "project",
        "repoPath",
    ])?;
    if cwd != worktree_str {
        return None;
    }

    let id = json_string(entry, &["sessionID", "sessionId", "id"])?;
    let title = json_string(entry, &["title", "slug", "name"]).unwrap_or_else(|| id.clone());
    let message_count = json_usize(entry, &["messageCount", "message_count", "messages"])
        .or_else(|| {
            entry
                .get("messages")
                .and_then(|messages| messages.as_array())
                .map(Vec::len)
        })
        .unwrap_or(0);

    Some(AgentSessionSummary {
        provider: AgentSessionProviderKind::OpenCode,
        id,
        title,
        timestamp_unix_ms: json_u64(entry, &[
            "updatedAtUnixMs",
            "updated_at_unix_ms",
            "updatedAt",
            "createdAtUnixMs",
            "created_at_unix_ms",
            "createdAt",
        ]),
        message_count,
    })
}

fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|entry| entry.as_str()))
        .map(ToOwned::to_owned)
}

fn json_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|entry| entry.as_u64())
}

fn json_usize(value: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    json_u64(value, keys).and_then(|count| usize::try_from(count).ok())
}

fn recent_agent_sessions_from_provider(
    provider: &dyn AgentSessionProvider,
    worktree_path: &Path,
    limit: usize,
) -> Vec<AgentSessionSummary> {
    provider.list_sessions(worktree_path, limit)
}

fn list_sorted_jsonl_files(dir: &Path) -> Vec<PathBuf> {
    if !dir.is_dir() {
        return Vec::new();
    }

    let mut files: Vec<_> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .collect(),
        Err(_) => return Vec::new(),
    };
    files.sort_by_key(|path| Reverse(file_modified_unix_ms(path).unwrap_or(0)));
    files
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("session")
        .to_owned()
}

fn file_modified_unix_ms(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(duration.as_millis()).ok()
}

fn extract_text_from_content_blocks(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }

    let blocks = value.as_array()?;
    for block in blocks {
        if let Some(text) = block.get("text").and_then(|entry| entry.as_str()) {
            return Some(text.to_owned());
        }
        if let Some(text) = block.get("content").and_then(|entry| entry.as_str()) {
            return Some(text.to_owned());
        }
    }

    None
}

/// Take the first line of a prompt, capped at ~80 characters.
fn truncate_prompt(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or(text);
    if first_line.len() <= 80 {
        first_line.to_owned()
    } else {
        let mut end = 80;
        while !first_line.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &first_line[..end])
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn claude_project_key_encodes_correctly() {
        let path = Path::new("/Users/penso/.superset/worktrees/arbor/psychedelic-gravity");
        assert_eq!(
            claude_project_key(path),
            "-Users-penso--superset-worktrees-arbor-psychedelic-gravity"
        );
    }

    #[test]
    fn claude_project_key_handles_dots() {
        let path = Path::new("/home/user/.config/project");
        assert_eq!(claude_project_key(path), "-home-user--config-project");
    }

    #[test]
    fn pi_project_key_encodes_correctly() {
        let path = Path::new("/Users/penso/code/arbor");
        assert_eq!(pi_project_key(path), "--Users-penso-code-arbor--");
    }

    #[test]
    fn truncate_prompt_short_text() {
        assert_eq!(truncate_prompt("fix the bug"), "fix the bug");
    }

    #[test]
    fn truncate_prompt_multiline() {
        let text = "first line\nsecond line\nthird line";
        assert_eq!(truncate_prompt(text), "first line");
    }

    #[test]
    fn truncate_prompt_long_text() {
        let long = "a".repeat(100);
        let result = truncate_prompt(&long);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 83);
    }

    #[test]
    fn truncate_prompt_exactly_80() {
        let text = "b".repeat(80);
        assert_eq!(truncate_prompt(&text), text);
    }

    #[test]
    fn extract_claude_user_prompt_parses_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test-session.jsonl");
        let content = r#"{"type":"file-history-snapshot","messageId":"abc"}
{"type":"user","message":{"role":"user","content":"fix the login bug"},"sessionId":"123"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"OK"}]}}
"#;
        fs::write(&file_path, content).unwrap();
        let result = extract_claude_user_prompt(&file_path);
        assert_eq!(result.as_deref(), Some("fix the login bug"));
    }

    #[test]
    fn extract_claude_user_prompt_array_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test-session.jsonl");
        let content = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"refactor auth"}]},"sessionId":"123"}
"#;
        fs::write(&file_path, content).unwrap();
        let result = extract_claude_user_prompt(&file_path);
        assert_eq!(result.as_deref(), Some("refactor auth"));
    }

    #[test]
    fn parse_claude_session_summary_counts_messages() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test-session.jsonl");
        let content = r#"{"type":"user","message":{"role":"user","content":"fix auth"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#;
        fs::write(&file_path, content).unwrap();

        let summary =
            parse_claude_session_summary(&file_path).unwrap_or_else(|| panic!("summary expected"));
        assert_eq!(summary.provider, AgentSessionProviderKind::Claude);
        assert_eq!(summary.title, "fix auth");
        assert_eq!(summary.message_count, 2);
    }

    #[test]
    fn parse_codex_session_summary_matching_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("session.jsonl");
        let content = r#"{"type":"session_meta","payload":{"cwd":"/repos/project","id":"abc"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"add tests for parser"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"working"}]}}
"#;
        fs::write(&file_path, content).unwrap();

        let result = parse_codex_session_summary(&file_path, Path::new("/repos/project"))
            .unwrap_or_else(|| panic!("summary expected"));
        assert_eq!(result.id, "abc");
        assert_eq!(result.title, "add tests for parser");
        assert_eq!(result.message_count, 2);
    }

    #[test]
    fn extract_codex_user_prompt_wrong_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("session.jsonl");
        let content = r#"{"type":"session_meta","payload":{"cwd":"/repos/other","id":"abc"}}
{"type":"event_msg","payload":{"type":"user_message","message":"add tests"}}
"#;
        fs::write(&file_path, content).unwrap();
        let result = parse_codex_session_summary(&file_path, Path::new("/repos/project"));
        assert!(result.is_none());
    }

    #[test]
    fn extract_pi_user_prompt_string_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("session.jsonl");
        let content = r#"{"type":"session","version":3,"id":"uuid","timestamp":"2024-12-03T14:00:00.000Z","cwd":"/repos/project"}
{"type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2024-12-03T14:00:01.000Z","message":{"role":"user","content":"implement pi support"}}
"#;
        fs::write(&file_path, content).unwrap();
        let result = extract_pi_user_prompt(&file_path);
        assert_eq!(result.as_deref(), Some("implement pi support"));
    }

    #[test]
    fn extract_pi_user_prompt_array_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("session.jsonl");
        let content = r#"{"type":"session","version":3,"id":"uuid","timestamp":"2024-12-03T14:00:00.000Z","cwd":"/repos/project"}
{"type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2024-12-03T14:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"summarize recent changes"}]}}
"#;
        fs::write(&file_path, content).unwrap();
        let result = extract_pi_user_prompt(&file_path);
        assert_eq!(result.as_deref(), Some("summarize recent changes"));
    }

    #[test]
    fn parse_opencode_session_list_filters_matching_worktree() {
        let worktree = Path::new("/repos/arbor");
        let raw = r#"[
  {
    "sessionID": "abc",
    "cwd": "/repos/arbor",
    "title": "Review PR 42",
    "messageCount": 12,
    "updatedAtUnixMs": 1234
  },
  {
    "sessionID": "def",
    "cwd": "/repos/other",
    "title": "Ignore me",
    "messageCount": 99,
    "updatedAtUnixMs": 9999
  }
]"#;

        let sessions = parse_opencode_session_list(raw, worktree, 5);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].provider, AgentSessionProviderKind::OpenCode);
        assert_eq!(sessions[0].id, "abc");
        assert_eq!(sessions[0].title, "Review PR 42");
        assert_eq!(sessions[0].message_count, 12);
    }
}
