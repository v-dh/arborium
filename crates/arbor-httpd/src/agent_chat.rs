//! Interactive agent chat session manager.
//!
//! Spawns agent CLI processes via `acpx` (or direct CLI fallback), parses their
//! JSONL stdout into structured events, and broadcasts them over a
//! `tokio::sync::broadcast` channel for WebSocket consumers.
//!
//! Each "turn" (user message → agent response) is a separate subprocess
//! invocation with the same named session, matching the acpx protocol.

use {
    serde::{Deserialize, Serialize},
    serde_json::Value,
    std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::Arc,
    },
    tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        process::Command,
        sync::{Mutex, broadcast},
    },
};

/// Relative path under `$HOME` for the persistent agent chat store.
const AGENT_CHAT_STORE_RELATIVE_PATH: &str = ".arbor/daemon/agent-chats.json";

// ── Public types ─────────────────────────────────────────────────────

/// Status of an agent chat session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentChatStatus {
    /// Waiting for user input.
    Idle,
    /// Agent is processing a turn.
    Working,
    /// Agent process has exited (session ended or error).
    Exited,
}

/// A structured event emitted by an agent session, streamed to the web UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AgentChatEvent {
    /// A chunk of the assistant's text response (streamed).
    MessageChunk { content: String },
    /// A chunk of the agent's internal reasoning/thinking.
    ThoughtChunk { content: String },
    /// A tool invocation by the agent.
    ToolCall { name: String, status: String },
    /// Agent started processing a turn.
    TurnStarted,
    /// Agent finished processing a turn.
    TurnCompleted,
    /// Token usage update.
    UsageUpdate {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// Error from the agent.
    Error { message: String },
    /// The agent process exited.
    SessionExited { exit_code: Option<i32> },
    /// Snapshot of the full conversation history (sent on WebSocket connect).
    Snapshot {
        messages: Vec<ChatMessage>,
        status: AgentChatStatus,
        input_tokens: u64,
        output_tokens: u64,
    },
    /// A complete user message (for history reconstruction).
    UserMessage { content: String },
    /// Status update (mode changes, config updates, etc.).
    StatusUpdate { message: String },
}

/// A message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatMessage {
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) tool_calls: Vec<String>,
}

/// DTO for the agent chat session list endpoint.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AgentChatSessionDto {
    pub(crate) id: String,
    pub(crate) agent_kind: String,
    pub(crate) workspace_path: String,
    pub(crate) status: AgentChatStatus,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
}

/// Request to create a new agent chat session.
#[derive(Debug, Deserialize)]
pub(crate) struct CreateAgentChatRequest {
    pub(crate) workspace_path: String,
    pub(crate) agent_kind: String,
    pub(crate) initial_prompt: Option<String>,
}

/// Response from creating an agent chat session.
#[derive(Debug, Serialize)]
pub(crate) struct CreateAgentChatResponse {
    pub(crate) session_id: String,
}

/// Request to send a message to an agent.
#[derive(Debug, Deserialize)]
pub(crate) struct SendAgentMessageRequest {
    pub(crate) message: String,
}

// ── Persistent session record ────────────────────────────────────────

/// A serializable snapshot of an agent chat session for persistence across
/// daemon restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentChatRecord {
    id: String,
    agent_kind: String,
    workspace_path: PathBuf,
    session_name: String,
    messages: Vec<ChatMessage>,
    input_tokens: u64,
    output_tokens: u64,
}

// ── Internal session state ───────────────────────────────────────────

struct AgentChatSession {
    id: String,
    agent_kind: String,
    workspace_path: PathBuf,
    session_name: String,
    event_tx: broadcast::Sender<AgentChatEvent>,
    messages: Vec<ChatMessage>,
    /// Text being streamed for the current assistant turn (not yet finalized).
    pending_assistant_text: String,
    /// Tool calls accumulated during the current turn.
    pending_tool_calls: Vec<String>,
    status: AgentChatStatus,
    input_tokens: u64,
    output_tokens: u64,
    /// Handle to cancel a running turn.
    turn_cancel: Option<tokio::sync::watch::Sender<bool>>,
}

// ── Manager ──────────────────────────────────────────────────────────

/// Manages interactive agent chat sessions.
pub(crate) struct AgentChatManager {
    sessions: HashMap<String, AgentChatSession>,
}

impl AgentChatManager {
    pub(crate) fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Load previously persisted agent chat sessions from disk.
    /// Restored sessions are idle (no running process) but can accept new
    /// messages which will resume the underlying acpx session.
    pub(crate) fn load_persisted_sessions(&mut self) {
        let path = agent_chat_store_path();
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(%e, "failed to read agent chat store");
                return;
            },
        };

        let records: Vec<AgentChatRecord> = match serde_json::from_str(&data) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(%e, "failed to parse agent chat store");
                return;
            },
        };

        for record in records {
            // Skip sessions that are already loaded (shouldn't happen, but guard)
            if self.sessions.contains_key(&record.id) {
                continue;
            }

            let (event_tx, _) = broadcast::channel::<AgentChatEvent>(256);
            let session = AgentChatSession {
                id: record.id.clone(),
                agent_kind: record.agent_kind,
                workspace_path: record.workspace_path,
                session_name: record.session_name,
                event_tx,
                messages: record.messages,
                pending_assistant_text: String::new(),
                pending_tool_calls: Vec::new(),
                status: AgentChatStatus::Idle,
                input_tokens: record.input_tokens,
                output_tokens: record.output_tokens,
                turn_cancel: None,
            };
            self.sessions.insert(record.id, session);
        }

        tracing::info!(
            count = self.sessions.len(),
            "restored agent chat sessions from disk"
        );
    }

    /// Persist all non-exited sessions to disk.
    pub(crate) fn persist(&self) {
        let records: Vec<AgentChatRecord> = self
            .sessions
            .values()
            .filter(|s| s.status != AgentChatStatus::Exited)
            .map(|s| {
                let mut messages = s.messages.clone();
                // Include any pending assistant text as a finalized message in
                // the persisted record so it isn't lost.
                if !s.pending_assistant_text.is_empty() {
                    messages.push(ChatMessage {
                        role: "assistant".to_owned(),
                        content: s.pending_assistant_text.clone(),
                        tool_calls: s.pending_tool_calls.clone(),
                    });
                }
                AgentChatRecord {
                    id: s.id.clone(),
                    agent_kind: s.agent_kind.clone(),
                    workspace_path: s.workspace_path.clone(),
                    session_name: s.session_name.clone(),
                    messages,
                    input_tokens: s.input_tokens,
                    output_tokens: s.output_tokens,
                }
            })
            .collect();

        let path = agent_chat_store_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match serde_json::to_string_pretty(&records) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, format!("{json}\n")) {
                    tracing::warn!(%e, "failed to write agent chat store");
                }
            },
            Err(e) => {
                tracing::warn!(%e, "failed to serialize agent chat store");
            },
        }
    }

    /// Create a new agent chat session. Optionally starts the first turn with
    /// an initial prompt.
    pub(crate) fn create_session(
        &mut self,
        agent_kind: String,
        workspace_path: PathBuf,
        initial_prompt: Option<String>,
    ) -> (String, broadcast::Receiver<AgentChatEvent>) {
        let session_id = format!(
            "agent-chat-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let session_name = format!("arbor-{session_id}");
        let (event_tx, event_rx) = broadcast::channel::<AgentChatEvent>(256);

        let session = AgentChatSession {
            id: session_id.clone(),
            agent_kind: agent_kind.clone(),
            workspace_path: workspace_path.clone(),
            session_name: session_name.clone(),
            event_tx: event_tx.clone(),
            messages: Vec::new(),
            pending_assistant_text: String::new(),
            pending_tool_calls: Vec::new(),
            status: AgentChatStatus::Idle,
            input_tokens: 0,
            output_tokens: 0,
            turn_cancel: None,
        };

        self.sessions.insert(session_id.clone(), session);

        // If there's an initial prompt, start the first turn immediately
        if let Some(prompt) = initial_prompt
            && let Some(session) = self.sessions.get_mut(&session_id)
        {
            session.messages.push(ChatMessage {
                role: "user".to_owned(),
                content: prompt.clone(),
                tool_calls: Vec::new(),
            });
            let _ = event_tx.send(AgentChatEvent::UserMessage {
                content: prompt.clone(),
            });
            start_turn(session, prompt);
        }

        (session_id, event_rx)
    }

    /// Send a follow-up message in an existing session.
    pub(crate) fn send_message(&mut self, session_id: &str, message: String) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;

        if session.status == AgentChatStatus::Working {
            return Err("agent is already processing a turn".to_owned());
        }

        session.messages.push(ChatMessage {
            role: "user".to_owned(),
            content: message.clone(),
            tool_calls: Vec::new(),
        });
        let _ = session.event_tx.send(AgentChatEvent::UserMessage {
            content: message.clone(),
        });

        start_turn(session, message);
        self.persist();
        Ok(())
    }

    /// Cancel a running turn (sends SIGINT to the child process).
    pub(crate) fn cancel(&mut self, session_id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;

        if let Some(cancel_tx) = session.turn_cancel.take() {
            let _ = cancel_tx.send(true);
        }
        Ok(())
    }

    /// Kill a session entirely.
    pub(crate) fn kill(&mut self, session_id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;

        if let Some(cancel_tx) = session.turn_cancel.take() {
            let _ = cancel_tx.send(true);
        }

        session.status = AgentChatStatus::Exited;
        let _ = session
            .event_tx
            .send(AgentChatEvent::SessionExited { exit_code: None });
        Ok(())
    }

    /// Remove a session from the manager.
    pub(crate) fn remove(&mut self, session_id: &str) {
        if let Some(mut session) = self.sessions.remove(session_id)
            && let Some(cancel_tx) = session.turn_cancel.take()
        {
            let _ = cancel_tx.send(true);
        }
    }

    /// List all active sessions.
    pub(crate) fn list(&self) -> Vec<AgentChatSessionDto> {
        self.sessions
            .values()
            .map(|s| AgentChatSessionDto {
                id: s.id.clone(),
                agent_kind: s.agent_kind.clone(),
                workspace_path: s.workspace_path.display().to_string(),
                status: s.status,
                input_tokens: s.input_tokens,
                output_tokens: s.output_tokens,
            })
            .collect()
    }

    /// Get the conversation history for a session.
    /// Includes any in-progress assistant text as a partial message.
    pub(crate) fn history(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        let mut messages = session.messages.clone();
        // Append the in-progress assistant response so the GUI can stream it
        if !session.pending_assistant_text.is_empty() {
            messages.push(ChatMessage {
                role: "assistant".to_owned(),
                content: session.pending_assistant_text.clone(),
                tool_calls: session.pending_tool_calls.clone(),
            });
        }
        Ok(messages)
    }

    /// Get a broadcast receiver for a session's events.
    pub(crate) fn subscribe(
        &self,
        session_id: &str,
    ) -> Result<
        (
            broadcast::Receiver<AgentChatEvent>,
            AgentChatSessionDto,
            Vec<ChatMessage>,
        ),
        String,
    > {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;
        Ok((
            session.event_tx.subscribe(),
            AgentChatSessionDto {
                id: session.id.clone(),
                agent_kind: session.agent_kind.clone(),
                workspace_path: session.workspace_path.display().to_string(),
                status: session.status,
                input_tokens: session.input_tokens,
                output_tokens: session.output_tokens,
            },
            session.messages.clone(),
        ))
    }
}

// ── Turn execution ───────────────────────────────────────────────────

/// Start a new turn (subprocess) for the session.
fn start_turn(session: &mut AgentChatSession, prompt: String) {
    session.status = AgentChatStatus::Working;
    let _ = session.event_tx.send(AgentChatEvent::TurnStarted);

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    session.turn_cancel = Some(cancel_tx);

    let session_id = session.id.clone();
    let agent_kind = session.agent_kind.clone();
    let workspace_path = session.workspace_path.clone();
    let session_name = session.session_name.clone();
    let event_tx = session.event_tx.clone();

    // We need a shared reference to update session state after the turn.
    // The caller must wrap AgentChatManager in Arc<Mutex<>> so we use
    // a channel-based approach: the background task sends events, and
    // we post a completion event that the manager listens to.
    tokio::spawn(async move {
        let result = run_turn(
            &agent_kind,
            &workspace_path,
            &session_name,
            &prompt,
            &event_tx,
            cancel_rx,
        )
        .await;

        match result {
            Ok(()) => {
                let _ = event_tx.send(AgentChatEvent::TurnCompleted);
            },
            Err(error) => {
                let _ = event_tx.send(AgentChatEvent::Error {
                    message: error.clone(),
                });
                tracing::warn!(session_id, %error, "agent turn failed");
            },
        }
    });
}

/// Run a single turn: spawn acpx, write prompt to stdin, parse JSONL from stdout.
async fn run_turn(
    agent_kind: &str,
    workspace_path: &Path,
    session_name: &str,
    prompt: &str,
    event_tx: &broadcast::Sender<AgentChatEvent>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let acpx_path = which_acpx();
    let cwd_str = workspace_path.display().to_string();

    // Ensure the named session exists before prompting.
    let ensure_result = Command::new(&acpx_path)
        .args([
            "--cwd",
            &cwd_str,
            agent_kind,
            "sessions",
            "ensure",
            "--name",
            session_name,
        ])
        .current_dir(workspace_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("CLAUDECODE")
        .output()
        .await
        .map_err(|e| format!("failed to ensure acpx session: {e}"))?;

    if !ensure_result.status.success() {
        let stderr = String::from_utf8_lossy(&ensure_result.stderr);
        tracing::warn!(session_name, %stderr, "acpx sessions ensure failed, continuing anyway");
    }

    let mut child = Command::new(&acpx_path)
        .args([
            "--format",
            "json",
            "--json-strict",
            "--cwd",
            &cwd_str,
            agent_kind,
            "prompt",
            "--session",
            session_name,
            "--file",
            "-",
        ])
        .current_dir(workspace_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("CLAUDECODE")
        .spawn()
        .map_err(|e| format!("failed to spawn acpx: {e}"))?;

    // Write the prompt to stdin and close it.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| format!("failed to write to acpx stdin: {e}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|e| format!("failed to close acpx stdin: {e}"))?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "acpx stdout unavailable".to_owned())?;

    let mut lines = BufReader::new(stdout).lines();
    let mut assistant_text = String::new();
    let mut tool_calls: Vec<String> = Vec::new();

    loop {
        tokio::select! {
            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) => {
                        if let Some(event) = parse_acpx_event(&line) {
                            // Accumulate assistant text for history
                            match &event {
                                AgentChatEvent::MessageChunk { content } => {
                                    assistant_text.push_str(content);
                                },
                                AgentChatEvent::ToolCall { name, status } => {
                                    tool_calls.push(format!("{name} ({status})"));
                                },
                                _ => {},
                            }
                            let _ = event_tx.send(event);
                        }
                    },
                    Ok(None) => break, // EOF
                    Err(error) => {
                        let _ = event_tx.send(AgentChatEvent::Error {
                            message: format!("read error: {error}"),
                        });
                        break;
                    },
                }
            },
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    let _ = child.kill().await;
                    return Err("turn cancelled".to_owned());
                }
            },
        }
    }

    // Wait for the process to exit
    let exit_status = child
        .wait()
        .await
        .map_err(|e| format!("failed to wait for acpx: {e}"))?;

    // Record the assistant message in the event stream for history tracking.
    // The manager will pick this up from the TurnCompleted event.
    if !assistant_text.is_empty() {
        // We don't send another event here — the MessageChunks already went out.
        // The history is tracked by the manager listening to events.
    }

    if !exit_status.success() {
        let code = exit_status.code();
        // Read stderr for diagnostics
        let stderr_text = if let Some(mut stderr) = child.stderr.take() {
            let mut buf = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut buf).await;
            buf
        } else {
            String::new()
        };

        if code == Some(127) {
            return Err("acpx not found in PATH".to_owned());
        }
        // If we got no output at all, report the failure
        if assistant_text.is_empty() {
            let detail = if stderr_text.trim().is_empty() {
                format!("agent exited with code {}", code.unwrap_or(-1))
            } else {
                stderr_text.trim().to_owned()
            };
            return Err(detail);
        }
    }

    Ok(())
}

/// Find the acpx binary.
fn which_acpx() -> String {
    // Check if acpx is in PATH
    if let Ok(output) = std::process::Command::new("which").arg("acpx").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }
    // Fallback to bare command name (will fail at spawn time if not found)
    "acpx".to_owned()
}

// ── JSONL event parsing ──────────────────────────────────────────────

/// Parse a single line of JSONL output from acpx into an `AgentChatEvent`.
///
/// Mirrors polyphony's `parse_acpx_prompt_event_line` in
/// `crates/agent-acpx/src/lib.rs:479-526`.
fn parse_acpx_event(line: &str) -> Option<AgentChatEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed: Value = serde_json::from_str(trimmed).ok()?;
    let object = parsed.as_object()?;

    // Handle JSON-RPC error responses (e.g. "no session found")
    if let Some(error_obj) = object.get("error").and_then(Value::as_object) {
        let message = error_obj
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown JSON-RPC error")
            .to_owned();
        return Some(AgentChatEvent::Error { message });
    }

    // Handle ACP session/update wrapper
    let payload = if object.get("method").and_then(Value::as_str) == Some("session/update") {
        object.get("params")?.get("update")?.as_object()?.clone()
    } else {
        object.clone()
    };

    let tag = payload
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .or_else(|| payload.get("type").and_then(Value::as_str))
        .unwrap_or_default();

    match tag {
        "text" | "agent_message_chunk" => {
            extract_text(&payload).map(|content| AgentChatEvent::MessageChunk { content })
        },
        "thought" | "agent_thought_chunk" => {
            extract_text(&payload).map(|content| AgentChatEvent::ThoughtChunk { content })
        },
        "tool_call" | "tool_call_update" => {
            let name = payload
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("tool call")
                .to_owned();
            let status = payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            Some(AgentChatEvent::ToolCall { name, status })
        },
        "usage_update" => {
            // Try to extract token counts from the payload
            let input_tokens = payload
                .get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output_tokens = payload
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            Some(AgentChatEvent::UsageUpdate {
                input_tokens,
                output_tokens,
            })
        },
        "done" => None, // Handled by process exit
        "error" => {
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("agent error")
                .to_owned();
            Some(AgentChatEvent::Error { message })
        },
        "current_mode_update"
        | "config_option_update"
        | "available_commands_update"
        | "session_info_update"
        | "plan"
        | "client_operation"
        | "update" => {
            let message = extract_text(&payload).unwrap_or_else(|| tag.replace('_', " "));
            Some(AgentChatEvent::StatusUpdate { message })
        },
        _ => None,
    }
}

/// Extract text content from a payload, checking multiple field paths.
fn extract_text(payload: &serde_json::Map<String, Value>) -> Option<String> {
    payload
        .get("content")
        .and_then(|content| {
            if let Some(text) = content.as_str() {
                return Some(text.to_string());
            }
            content
                .as_object()
                .and_then(|obj| obj.get("text").and_then(Value::as_str))
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            payload
                .get("text")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            payload
                .get("summary")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

// ── Background event listener ────────────────────────────────────────

/// Spawn a background task that listens to a session's events and updates the
/// manager's state (accumulates messages, tracks status, updates tokens).
///
/// This is called after creating a session to keep the in-memory state in sync.
pub(crate) fn spawn_session_listener(
    manager: Arc<Mutex<AgentChatManager>>,
    session_id: String,
    mut event_rx: broadcast::Receiver<AgentChatEvent>,
) {
    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(event) => {
                    let mut mgr = manager.lock().await;
                    let Some(session) = mgr.sessions.get_mut(&session_id) else {
                        break;
                    };

                    match &event {
                        AgentChatEvent::MessageChunk { content } => {
                            // Append to pending text — history() includes this
                            // as a streaming partial message.
                            session.pending_assistant_text.push_str(content);
                        },
                        AgentChatEvent::ToolCall { name, status } => {
                            session
                                .pending_tool_calls
                                .push(format!("{name} ({status})"));
                        },
                        AgentChatEvent::TurnCompleted => {
                            // Finalize: move pending text into permanent messages
                            if !session.pending_assistant_text.is_empty() {
                                let text = std::mem::take(&mut session.pending_assistant_text);
                                let tools = std::mem::take(&mut session.pending_tool_calls);
                                session.messages.push(ChatMessage {
                                    role: "assistant".to_owned(),
                                    content: text,
                                    tool_calls: tools,
                                });
                            }
                            session.status = AgentChatStatus::Idle;
                            mgr.persist();
                        },
                        AgentChatEvent::Error { message } => {
                            // Finalize any partial text, then record the error
                            if !session.pending_assistant_text.is_empty() {
                                let text = std::mem::take(&mut session.pending_assistant_text);
                                let tools = std::mem::take(&mut session.pending_tool_calls);
                                session.messages.push(ChatMessage {
                                    role: "assistant".to_owned(),
                                    content: text,
                                    tool_calls: tools,
                                });
                            } else {
                                session.pending_assistant_text.clear();
                                session.pending_tool_calls.clear();
                            }
                            session.messages.push(ChatMessage {
                                role: "error".to_owned(),
                                content: message.clone(),
                                tool_calls: Vec::new(),
                            });
                            session.status = AgentChatStatus::Idle;
                            mgr.persist();
                        },
                        AgentChatEvent::SessionExited { .. } => {
                            session.status = AgentChatStatus::Exited;
                            mgr.persist();
                            break;
                        },
                        AgentChatEvent::UsageUpdate {
                            input_tokens,
                            output_tokens,
                        } => {
                            session.input_tokens = *input_tokens;
                            session.output_tokens = *output_tokens;
                        },
                        AgentChatEvent::TurnStarted => {
                            session.pending_assistant_text.clear();
                            session.pending_tool_calls.clear();
                            session.status = AgentChatStatus::Working;
                        },
                        _ => {},
                    }
                },
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(session_id, skipped, "session listener lagged");
                },
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Resolve the path to the persistent agent chat store file.
fn agent_chat_store_path() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(AGENT_CHAT_STORE_RELATIVE_PATH),
        Err(_) => PathBuf::from(AGENT_CHAT_STORE_RELATIVE_PATH),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_chunk() {
        let line = r#"{"type":"agent_message_chunk","content":{"type":"text","text":"Hello"}}"#;
        let event = parse_acpx_event(line).unwrap();
        match event {
            AgentChatEvent::MessageChunk { content } => assert_eq!(content, "Hello"),
            other => panic!("expected MessageChunk, got {other:?}"),
        }
    }

    #[test]
    fn parse_thought_chunk() {
        let line =
            r#"{"type":"agent_thought_chunk","content":{"type":"text","text":"thinking..."}}"#;
        let event = parse_acpx_event(line).unwrap();
        match event {
            AgentChatEvent::ThoughtChunk { content } => assert_eq!(content, "thinking..."),
            other => panic!("expected ThoughtChunk, got {other:?}"),
        }
    }

    #[test]
    fn parse_tool_call() {
        let line = r#"{"type":"tool_call","title":"Read file","status":"completed"}"#;
        let event = parse_acpx_event(line).unwrap();
        match event {
            AgentChatEvent::ToolCall { name, status } => {
                assert_eq!(name, "Read file");
                assert_eq!(status, "completed");
            },
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn parse_error() {
        let line = r#"{"type":"error","message":"rate limited"}"#;
        let event = parse_acpx_event(line).unwrap();
        match event {
            AgentChatEvent::Error { message } => assert_eq!(message, "rate limited"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn parse_done_returns_none() {
        let line = r#"{"type":"done"}"#;
        assert!(parse_acpx_event(line).is_none());
    }

    #[test]
    fn parse_empty_line_returns_none() {
        assert!(parse_acpx_event("").is_none());
        assert!(parse_acpx_event("  ").is_none());
    }

    #[test]
    fn parse_session_update_wrapper() {
        let line = r#"{"method":"session/update","params":{"update":{"type":"agent_message_chunk","content":"hi"}}}"#;
        let event = parse_acpx_event(line).unwrap();
        match event {
            AgentChatEvent::MessageChunk { content } => assert_eq!(content, "hi"),
            other => panic!("expected MessageChunk, got {other:?}"),
        }
    }
}
