use super::*;

impl ArborWindow {
    /// Create a new agent chat session via the daemon and open it in a tab.
    pub(crate) fn spawn_agent_chat(&mut self, kind: AgentPresetKind, cx: &mut Context<Self>) {
        let Some(worktree_path) = self.selected_worktree_path().map(Path::to_path_buf) else {
            self.notice = Some("No worktree selected".to_owned());
            cx.notify();
            return;
        };

        let Some(daemon) = self.terminal_daemon.clone() else {
            self.notice = Some("No daemon connection".to_owned());
            cx.notify();
            return;
        };

        let workspace_path_str = worktree_path.display().to_string();
        let agent_kind = kind.key().to_owned();
        let local_id = self.next_agent_chat_id;
        self.next_agent_chat_id += 1;

        let workspace_path_clone = worktree_path.clone();
        let agent_kind_clone = agent_kind.clone();

        cx.spawn(async move |this, cx| {
            let result = daemon.create_agent_chat(&workspace_path_str, &agent_kind_clone, None);
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    match result {
                        Ok(response) => {
                            let session_id = response.session_id.clone();
                            let session = NativeAgentChatSession {
                                local_id,
                                session_id: response.session_id,
                                agent_kind: agent_kind_clone,
                                workspace_path: workspace_path_clone.clone(),
                                status: "idle".to_owned(),
                                messages: Vec::new(),
                                input_text: String::new(),
                                input_cursor: 0,
                                input_tokens: 0,
                                output_tokens: 0,
                            };
                            this.agent_chat_sessions.push(session);
                            this.active_agent_chat_by_worktree
                                .insert(workspace_path_clone, local_id);
                            // Clear terminal selection so agent chat tab shows
                            this.active_diff_session_id = None;
                            this.active_file_view_session_id = None;
                            this.logs_tab_active = false;
                            // Start WebSocket streaming for this session
                            this.start_agent_chat_ws(local_id, session_id, daemon.clone(), cx);
                            cx.notify();
                        },
                        Err(error) => {
                            this.notice = Some(format!("Failed to create agent chat: {error}"));
                            cx.notify();
                        },
                    }
                })
            })
        })
        .detach();
    }

    /// Restore agent chat sessions from the daemon on startup.
    /// Queries the daemon for existing agent chat sessions and creates local
    /// tabs for any that match known worktrees. Connects WebSocket to each
    /// to receive the conversation snapshot.
    pub(crate) fn restore_agent_chat_sessions(&mut self, cx: &mut Context<Self>) {
        let Some(daemon) = self.terminal_daemon.clone() else {
            return;
        };

        let sessions = match daemon.list_agent_chats() {
            Ok(s) => s,
            Err(error) => {
                tracing::debug!(%error, "failed to list agent chats for restore");
                return;
            },
        };

        if sessions.is_empty() {
            return;
        }

        for summary in sessions {
            let workspace_path = PathBuf::from(&summary.workspace_path);

            // Only restore sessions for known worktrees
            if !self.worktrees.iter().any(|w| w.path == workspace_path) {
                continue;
            }

            // Skip if we already have a session with this daemon ID
            if self
                .agent_chat_sessions
                .iter()
                .any(|s| s.session_id == summary.id)
            {
                continue;
            }

            let local_id = self.next_agent_chat_id;
            self.next_agent_chat_id += 1;

            let session = NativeAgentChatSession {
                local_id,
                session_id: summary.id.clone(),
                agent_kind: summary.agent_kind,
                workspace_path: workspace_path.clone(),
                status: summary.status,
                messages: Vec::new(), // Will be filled by WebSocket snapshot
                input_text: String::new(),
                input_cursor: 0,
                input_tokens: summary.input_tokens,
                output_tokens: summary.output_tokens,
            };

            self.agent_chat_sessions.push(session);
            self.active_agent_chat_by_worktree
                .insert(workspace_path, local_id);

            // Connect WebSocket to receive the conversation snapshot
            self.start_agent_chat_ws(local_id, summary.id, daemon.clone(), cx);
        }

        if !self.agent_chat_sessions.is_empty() {
            tracing::info!(
                count = self.agent_chat_sessions.len(),
                "restored agent chat tabs"
            );
            cx.notify();
        }
    }

    /// Handle keyboard events for the agent chat input field.
    /// Returns `true` if the key was consumed and should not propagate to IME.
    pub(crate) fn handle_agent_chat_key_down(
        &mut self,
        local_id: u64,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;

        // Dismiss agent selector popup on any key
        if self.agent_selector_open_for.is_some() {
            self.agent_selector_open_for = None;
            cx.notify();
            if key == "escape" {
                return true;
            }
        }

        match key {
            "escape" => true, // consumed above or no-op
            "enter" if modifiers.shift => {
                // Shift+Enter inserts a newline
                if let Some(session) = self.agent_chat_session_mut(local_id) {
                    let cursor = session.input_cursor;
                    session.input_text.insert(cursor, '\n');
                    session.input_cursor += 1;
                }
                cx.notify();
                true
            },
            "enter" => {
                self.send_agent_chat_message(local_id, cx);
                true
            },
            "backspace" => {
                if let Some(session) = self.agent_chat_session_mut(local_id)
                    && session.input_cursor > 0
                {
                    let remove_at = session.input_cursor - 1;
                    session.input_text.remove(remove_at);
                    session.input_cursor -= 1;
                }
                cx.notify();
                true
            },
            "delete" => {
                if let Some(session) = self.agent_chat_session_mut(local_id)
                    && session.input_cursor < session.input_text.len()
                {
                    session.input_text.remove(session.input_cursor);
                }
                cx.notify();
                true
            },
            "left" => {
                if let Some(session) = self.agent_chat_session_mut(local_id)
                    && session.input_cursor > 0
                {
                    session.input_cursor -= 1;
                }
                cx.notify();
                true
            },
            "right" => {
                if let Some(session) = self.agent_chat_session_mut(local_id)
                    && session.input_cursor < session.input_text.len()
                {
                    session.input_cursor += 1;
                }
                cx.notify();
                true
            },
            "v" if modifiers.platform => {
                if let Some(clipboard_item) = cx.read_from_clipboard()
                    && let Some(text) = clipboard_item.text()
                    && let Some(session) = self.agent_chat_session_mut(local_id)
                {
                    let cursor = session.input_cursor;
                    session.input_text.insert_str(cursor, &text);
                    session.input_cursor += text.len();
                }
                cx.notify();
                true
            },
            "a" if modifiers.platform => {
                if let Some(session) = self.agent_chat_session_mut(local_id) {
                    session.input_cursor = session.input_text.len();
                }
                cx.notify();
                true
            },
            "home" => {
                if let Some(session) = self.agent_chat_session_mut(local_id) {
                    session.input_cursor = 0;
                }
                cx.notify();
                true
            },
            "end" => {
                if let Some(session) = self.agent_chat_session_mut(local_id) {
                    session.input_cursor = session.input_text.len();
                }
                cx.notify();
                true
            },
            // Regular character keys — let them flow through to IME/replace_text_in_range
            _ => false,
        }
    }

    fn agent_chat_session_mut(&mut self, local_id: u64) -> Option<&mut NativeAgentChatSession> {
        self.agent_chat_sessions
            .iter_mut()
            .find(|s| s.local_id == local_id)
    }

    /// Send the current input text as a message in the agent chat.
    pub(crate) fn send_agent_chat_message(&mut self, local_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self
            .agent_chat_sessions
            .iter_mut()
            .find(|s| s.local_id == local_id)
        else {
            return;
        };

        let message = session.input_text.trim().to_owned();
        if message.is_empty() {
            return;
        }

        // Add user message to the local list immediately
        session.messages.push(AgentChatMessage {
            role: "user".to_owned(),
            content: message.clone(),
            tool_calls: Vec::new(),
        });
        session.input_text.clear();
        session.input_cursor = 0;
        session.status = "working".to_owned();
        cx.notify();

        let Some(daemon) = self.terminal_daemon.clone() else {
            return;
        };

        let session_id = session.session_id.clone();
        cx.spawn(async move |this, cx| {
            let result = daemon.send_agent_message(&session_id, &message);
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if let Err(error) = result {
                        this.notice = Some(format!("Failed to send message: {error}"));
                        // Revert status
                        if let Some(session) = this
                            .agent_chat_sessions
                            .iter_mut()
                            .find(|s| s.session_id == session_id)
                        {
                            session.status = "idle".to_owned();
                        }
                        cx.notify();
                    }
                    // On success, polling will pick up the response
                })
            })
        })
        .detach();
    }

    /// Cancel an in-progress agent chat turn.
    pub(crate) fn cancel_agent_chat(&mut self, local_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self
            .agent_chat_sessions
            .iter()
            .find(|s| s.local_id == local_id)
        else {
            return;
        };

        let Some(daemon) = self.terminal_daemon.clone() else {
            return;
        };

        let session_id = session.session_id.clone();
        cx.spawn(async move |this, cx| {
            let result = daemon.cancel_agent_chat(&session_id);
            cx.update(|cx| {
                this.update(cx, |this, cx| match result {
                    Ok(()) => {
                        if let Some(session) = this
                            .agent_chat_sessions
                            .iter_mut()
                            .find(|s| s.session_id == session_id)
                        {
                            session.status = "idle".to_owned();
                        }
                        cx.notify();
                    },
                    Err(error) => {
                        this.notice = Some(format!("Failed to cancel: {error}"));
                        cx.notify();
                    },
                })
            })
        })
        .detach();
    }

    /// Start a WebSocket connection to stream agent chat events in real time.
    fn start_agent_chat_ws(
        &self,
        local_id: u64,
        session_id: String,
        daemon: terminal_daemon_http::SharedTerminalDaemonClient,
        cx: &mut Context<Self>,
    ) {
        use terminal_daemon_http::AgentChatWsEvent;

        let connect_config = match daemon.agent_chat_websocket_config(&session_id) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(%error, "failed to build agent chat WS config");
                return;
            },
        };

        // Use a channel to bridge events from the background WS thread to the main thread.
        let (event_tx, event_rx) = smol::channel::bounded::<AgentChatWsEvent>(64);

        // Background OS thread: connect and read WS messages.
        std::thread::spawn(move || {
            let request = match daemon_websocket_request(&connect_config) {
                Ok(r) => r,
                Err(error) => {
                    tracing::warn!(%error, "failed to build agent chat WS request");
                    return;
                },
            };

            let (mut socket, _) = match tungstenite::connect(request) {
                Ok(pair) => pair,
                Err(error) => {
                    tracing::warn!(%error, "failed to connect agent chat WS");
                    return;
                },
            };

            // Set short read timeout so the loop can detect channel closure.
            if let tungstenite::stream::MaybeTlsStream::Plain(tcp) = socket.get_ref() {
                let _ = tcp.set_nodelay(true);
                let _ = tcp.set_read_timeout(Some(Duration::from_millis(50)));
            }

            loop {
                if event_tx.is_closed() {
                    let _ = socket.close(None);
                    break;
                }

                match socket.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        if let Ok(event) = serde_json::from_str::<AgentChatWsEvent>(&text)
                            && event_tx.send_blocking(event).is_err()
                        {
                            break;
                        }
                    },
                    Ok(tungstenite::Message::Close(_)) => break,
                    Ok(_) => {}, // Ping/Pong/Binary — ignore
                    Err(tungstenite::Error::Io(ref e))
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        // Timeout — loop back to check channel
                    },
                    Err(_) => break,
                }
            }
        });

        // Main-thread async task: receive events from the channel and apply them.
        cx.spawn(async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                let should_break = this
                    .update(cx, |this, cx| {
                        Self::apply_agent_chat_ws_event(this, local_id, event, cx)
                    })
                    .unwrap_or(true);
                if should_break {
                    break;
                }
            }
        })
        .detach();
    }

    /// Apply a single WebSocket event to the local agent chat session state.
    /// Returns `true` if the WS loop should break (session gone).
    fn apply_agent_chat_ws_event(
        &mut self,
        local_id: u64,
        event: terminal_daemon_http::AgentChatWsEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        use terminal_daemon_http::AgentChatWsEvent;

        let Some(session) = self
            .agent_chat_sessions
            .iter_mut()
            .find(|s| s.local_id == local_id)
        else {
            return true; // session removed, stop WS
        };

        match event {
            AgentChatWsEvent::Snapshot {
                messages,
                status,
                input_tokens,
                output_tokens,
            } => {
                session.messages = messages
                    .into_iter()
                    .map(|m| AgentChatMessage {
                        role: m.role,
                        content: m.content,
                        tool_calls: m.tool_calls,
                    })
                    .collect();
                session.status = status;
                session.input_tokens = input_tokens;
                session.output_tokens = output_tokens;
                cx.notify();
            },
            AgentChatWsEvent::MessageChunk { content } => {
                // Append to the last assistant message, or create one
                if let Some(last) = session.messages.last_mut() {
                    if last.role == "assistant" {
                        last.content.push_str(&content);
                    } else {
                        session.messages.push(AgentChatMessage {
                            role: "assistant".to_owned(),
                            content,
                            tool_calls: Vec::new(),
                        });
                    }
                } else {
                    session.messages.push(AgentChatMessage {
                        role: "assistant".to_owned(),
                        content,
                        tool_calls: Vec::new(),
                    });
                }
                cx.notify();
            },
            AgentChatWsEvent::ToolCall { name, status } => {
                if let Some(last) = session
                    .messages
                    .last_mut()
                    .filter(|m| m.role == "assistant")
                {
                    last.tool_calls.push(format!("{name} ({status})"));
                }
                cx.notify();
            },
            AgentChatWsEvent::TurnStarted => {
                session.status = "working".to_owned();
                cx.notify();
            },
            AgentChatWsEvent::TurnCompleted => {
                session.status = "idle".to_owned();
                cx.notify();
            },
            AgentChatWsEvent::UsageUpdate {
                input_tokens,
                output_tokens,
            } => {
                session.input_tokens = input_tokens;
                session.output_tokens = output_tokens;
                cx.notify();
            },
            AgentChatWsEvent::Error { message } => {
                session.messages.push(AgentChatMessage {
                    role: "error".to_owned(),
                    content: message,
                    tool_calls: Vec::new(),
                });
                session.status = "idle".to_owned();
                cx.notify();
            },
            AgentChatWsEvent::SessionExited { .. } => {
                session.status = "exited".to_owned();
                cx.notify();
                return true;
            },
            AgentChatWsEvent::UserMessage { content } => {
                // Daemon echoes user messages — only add if not already present
                if !session
                    .messages
                    .last()
                    .is_some_and(|m| m.role == "user" && m.content == content)
                {
                    session.messages.push(AgentChatMessage {
                        role: "user".to_owned(),
                        content,
                        tool_calls: Vec::new(),
                    });
                    cx.notify();
                }
            },
            AgentChatWsEvent::ThoughtChunk { .. } | AgentChatWsEvent::StatusUpdate { .. } => {
                // Could display thinking indicator, but status is already tracked
            },
        }

        false
    }
}
