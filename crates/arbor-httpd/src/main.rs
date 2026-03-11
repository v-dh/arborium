mod auth;
mod github_service;
#[cfg(feature = "mdns")]
mod mdns;
mod process_manager;
mod repository_store;
mod routes;
mod terminal_daemon;
mod types;

pub(crate) use types::*;
use {
    crate::{process_manager::ProcessManager, routes::*, terminal_daemon::LocalTerminalDaemon},
    arbor_core::daemon::JsonDaemonSessionStore,
    axum::{
        Router,
        handler::HandlerWithoutStateExt,
        routing::{delete, get, post},
    },
    std::{collections::HashMap, env, net::SocketAddr, path::PathBuf, sync::Arc},
    tokio::sync::Mutex,
    tower_http::services::ServeDir,
};

fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/repositories", get(list_repositories))
        .route("/worktrees", get(list_worktrees).post(create_worktree))
        .route("/worktrees/delete", post(delete_worktree))
        .route("/worktrees/changes", get(list_worktree_changes))
        .route("/worktrees/commit", post(commit_worktree))
        .route("/worktrees/push", post(push_worktree))
        .route("/terminals", get(list_terminals).post(create_terminal))
        .route(
            "/terminals/{session_id}/snapshot",
            get(get_terminal_snapshot),
        )
        .route("/terminals/{session_id}/write", post(write_terminal))
        .route("/terminals/{session_id}/resize", post(resize_terminal))
        .route("/terminals/{session_id}/signal", post(signal_terminal))
        .route("/terminals/{session_id}/detach", post(detach_terminal))
        .route("/terminals/{session_id}", delete(kill_terminal))
        .route("/terminals/{session_id}/ws", get(terminal_ws))
        .route("/agent/notify", post(agent_notify))
        .route("/agent/activity", get(list_agent_activity))
        .route("/agent/activity/ws", get(agent_activity_ws))
        .route("/processes", get(list_processes))
        .route("/processes/start-all", post(start_all_processes))
        .route("/processes/stop-all", post(stop_all_processes))
        .route("/processes/{name}/start", post(start_process))
        .route("/processes/{name}/stop", post(stop_process))
        .route("/processes/{name}/restart", post(restart_process))
        .route("/processes/ws", get(process_status_ws))
        .route("/shutdown", post(shutdown_daemon))
        .route("/config/bind", post(set_bind_mode).get(get_bind_mode))
        .route("/logs/ws", get(logs_ws));

    let with_state = Router::new().nest("/api/v1", api).with_state(state);

    // Always set up ServeDir — check for assets dynamically per-request so
    // that a long-running daemon picks up assets installed after startup
    // (e.g. an app update while the detached httpd process is still running).
    let dist_dir = arbor_web_ui::dist_dir();
    with_state.fallback_service(
        ServeDir::new(dist_dir).not_found_service(web_ui_spa_or_unavailable.into_service()),
    )
}

fn configure_embedded_terminal_engine() {
    let requested = env::var("ARBOR_TERMINAL_ENGINE")
        .ok()
        .or_else(load_embedded_terminal_engine_setting);
    match arbor_terminal_emulator::parse_terminal_engine_kind(requested.as_deref()) {
        Ok(engine) => arbor_terminal_emulator::set_default_terminal_engine(engine),
        Err(error) => {
            tracing::warn!(%error, "invalid embedded terminal engine configuration");
            arbor_terminal_emulator::set_default_terminal_engine(
                arbor_terminal_emulator::TerminalEngineKind::default(),
            );
        },
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing with a broadcast layer so logs can be streamed to the GUI.
    let (log_broadcast, _) = tokio::sync::broadcast::channel::<String>(LOG_BROADCAST_CAPACITY);
    {
        use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let broadcast_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let fmt_layer = tracing_subscriber::fmt::layer().with_filter(env_filter);
        let broadcast_layer = BroadcastLogLayer {
            sender: log_broadcast.clone(),
        }
        .with_filter(broadcast_filter);
        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(broadcast_layer)
            .init();
    }

    let web_ui_result = ensure_web_ui_assets();
    if let Err(error) = &web_ui_result {
        eprintln!("web-ui build skipped: {error}");
    }

    // Load daemon config and ensure auth token exists
    let mut daemon_config = load_daemon_config();
    ensure_auth_token(&mut daemon_config);
    configure_embedded_terminal_engine();
    let allow_remote = is_public_bind(
        daemon_config.auth_token.as_deref(),
        daemon_config.bind.as_deref(),
    );
    // Always bind to 0.0.0.0 when auth is configured so remote access can be
    // toggled at runtime without restarting.  The auth middleware enforces
    // localhost-only mode via the `allow_remote` flag instead.
    let bind_addr = resolve_bind_addr(
        daemon_config.auth_token.as_deref(),
        daemon_config.bind.as_deref(),
    )?;
    let has_auth = daemon_config.auth_token.is_some();
    let auth_state = auth::AuthState::new(daemon_config.auth_token, allow_remote);

    let daemon_store = JsonDaemonSessionStore::default();
    let (agent_broadcast, _) = tokio::sync::broadcast::channel::<AgentWsEvent>(64);

    // Initialize process manager — scan repository roots for arbor.toml files
    let repository_store = repository_store::default_repository_store();
    let process_manager = {
        let roots = repository_store.load_roots().unwrap_or_default();
        let resolved = repository_store::resolve_repository_roots(roots);
        let repo_root = resolved
            .into_iter()
            .next()
            .unwrap_or_else(|| PathBuf::from("."));
        let mut pm = ProcessManager::new(repo_root.clone());
        let configs = process_manager::load_process_configs(&repo_root);
        if !configs.is_empty() {
            println!(
                "loaded {} process config(s) from {}/arbor.toml",
                configs.len(),
                repo_root.display()
            );
        }
        pm.load_configs(configs);
        pm
    };

    let state = AppState {
        repository_store: repository_store.clone(),
        daemon: Arc::new(Mutex::new(LocalTerminalDaemon::new(daemon_store))),
        process_manager: Arc::new(Mutex::new(process_manager)),
        github_service: github_service::default_github_pr_service(),
        agent_sessions: Arc::new(Mutex::new(HashMap::new())),
        agent_broadcast,
        log_broadcast,
        pr_cache: Arc::new(Mutex::new(HashMap::new())),
        repo_cache: Arc::new(Mutex::new(HashMap::new())),
        shutdown_signal: Arc::new(tokio::sync::Notify::new()),
        auth_state: auth_state.clone(),
    };

    // Spawn background task to monitor process lifecycle
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
            let mut ticks_since_reap: u32 = 0;
            loop {
                interval.tick().await;
                let restart_schedule = {
                    let mut pm = state.process_manager.lock().await;
                    let mut daemon = state.daemon.lock().await;

                    // Periodically reap exited terminal sessions to free memory
                    // (~23 MB per dead session from scrollback buffers).
                    ticks_since_reap += 1;
                    if ticks_since_reap >= 30 {
                        // Every ~60 seconds
                        daemon.reap_exited_sessions();
                        ticks_since_reap = 0;
                    }

                    pm.check_and_update(&mut *daemon)
                };
                for (name, delay) in restart_schedule {
                    let state = state.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let mut pm = state.process_manager.lock().await;
                        let mut daemon = state.daemon.lock().await;
                        let _ = pm.restart_process(&name, &mut *daemon);
                    });
                }
            }
        });
    }

    let shutdown_signal = state.shutdown_signal.clone();
    let app = auth::with_auth(router(state), auth_state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!(
        terminal_engine = arbor_terminal_emulator::default_terminal_engine().as_str(),
        "arbor-httpd listening on http://{local_addr}",
    );

    // Announce on the local network via mDNS — hold handle to keep registration alive
    #[cfg(feature = "mdns")]
    let _mdns = match mdns::register_service(local_addr.port(), false, has_auth) {
        Ok(registration) => {
            tracing::info!(port = local_addr.port(), "mDNS: announcing _arbor._tcp");
            Some(registration)
        },
        Err(e) => {
            tracing::warn!(%e, "mDNS: failed to register, LAN discovery disabled");
            None
        },
    };

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move { shutdown_signal.notified().await })
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            repository_store::JsonRepositoryStore, routes::process_ws_client_message,
            terminal_daemon::SessionEvent,
        },
        arbor_core::daemon::{CreateOrAttachRequest, KillRequest, TerminalDaemon},
        axum::{
            Json,
            body::Bytes,
            extract::{Path as AxumPath, State, ws::Message},
            http::StatusCode,
        },
        std::time::Duration,
    };

    #[tokio::test]
    #[cfg_attr(windows, ignore = "requires Unix shell (stty/cat)")]
    async fn write_terminal_accepts_raw_request_bytes() {
        let temp = match tempfile::tempdir() {
            Ok(temp) => temp,
            Err(error) => panic!("failed to create temp dir: {error}"),
        };
        let state = test_app_state(temp.path().to_path_buf());
        let session_id = create_raw_echo_session(&state, "rest-binary").await;
        let mut subscription = subscribe_to_session(&state, &session_id).await;
        let payload = vec![0xff, 0x00, 0x1b, b'[', b'6', b'n'];

        let response = write_terminal(
            State(state.clone()),
            AxumPath(session_id.clone()),
            Bytes::from(payload.clone()),
        )
        .await;
        let status = match response {
            Ok(status) => status,
            Err((status, Json(error))) => {
                panic!("write handler failed with {status}: {}", error.error)
            },
        };
        assert_eq!(status, StatusCode::NO_CONTENT);

        let echoed = collect_output_bytes(&mut subscription, payload.len()).await;
        assert_eq!(echoed, payload);

        kill_session(&state, &session_id).await;
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "requires Unix shell (stty/cat)")]
    async fn websocket_binary_frames_write_raw_terminal_bytes() {
        let temp = match tempfile::tempdir() {
            Ok(temp) => temp,
            Err(error) => panic!("failed to create temp dir: {error}"),
        };
        let state = test_app_state(temp.path().to_path_buf());
        let session_id = create_raw_echo_session(&state, "ws-binary").await;
        let mut subscription = subscribe_to_session(&state, &session_id).await;
        let payload = vec![0xde, 0xad, 0x00, 0xbe, 0xef];

        let keep_open = match process_ws_client_message(
            &state,
            &session_id,
            Message::Binary(payload.clone().into()),
        )
        .await
        {
            Ok(keep_open) => keep_open,
            Err(error) => panic!("binary websocket write failed: {error}"),
        };
        assert!(
            keep_open,
            "binary terminal input unexpectedly closed the socket"
        );

        let echoed = collect_output_bytes(&mut subscription, payload.len()).await;
        assert_eq!(echoed, payload);

        kill_session(&state, &session_id).await;
    }

    #[test]
    fn default_bind_addr_uses_public_interface_only_when_auth_is_enabled() {
        assert_eq!(
            default_bind_addr(Some("secret-token"), 8787),
            SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 8787))
        );
        assert_eq!(
            default_bind_addr(None, 8787),
            SocketAddr::from(([127, 0, 0, 1], 8787))
        );
        assert_eq!(
            default_bind_addr(Some("   "), 8787),
            SocketAddr::from(([127, 0, 0, 1], 8787))
        );
    }

    #[test]
    fn configured_bind_addr_overrides_default_bind_mode() {
        assert_eq!(
            configured_bind_addr(Some("localhost"), Some("secret-token"), 8787),
            SocketAddr::from(([127, 0, 0, 1], 8787))
        );
        assert_eq!(
            configured_bind_addr(Some("all-interfaces"), None, 8787),
            SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 8787))
        );
    }

    fn test_app_state(repo_root: PathBuf) -> AppState {
        let daemon_store = JsonDaemonSessionStore::new(repo_root.join("daemon-sessions.json"));
        let repository_store = Arc::new(JsonRepositoryStore::new(
            repo_root.join("repositories.json"),
        ));
        let (agent_broadcast, _) = tokio::sync::broadcast::channel(16);
        let (log_broadcast, _) = tokio::sync::broadcast::channel(16);

        AppState {
            repository_store,
            daemon: Arc::new(Mutex::new(LocalTerminalDaemon::new(daemon_store))),
            process_manager: Arc::new(Mutex::new(ProcessManager::new(repo_root))),
            github_service: github_service::default_github_pr_service(),
            agent_sessions: Arc::new(Mutex::new(HashMap::new())),
            agent_broadcast,
            log_broadcast,
            pr_cache: Arc::new(Mutex::new(HashMap::new())),
            repo_cache: Arc::new(Mutex::new(HashMap::new())),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            auth_state: auth::AuthState::new(None, false),
        }
    }

    async fn create_raw_echo_session(state: &AppState, session_id: &str) -> String {
        let cwd = match env::current_dir() {
            Ok(cwd) => cwd,
            Err(error) => panic!("failed to read current directory: {error}"),
        };
        let response = {
            let mut daemon = state.daemon.lock().await;
            daemon.create_or_attach(CreateOrAttachRequest {
                session_id: session_id.into(),
                workspace_id: cwd.display().to_string().into(),
                cwd,
                shell: String::new(),
                cols: 120,
                rows: 35,
                title: Some("binary-test".to_owned()),
                command: Some("stty raw -echo; cat".to_owned()),
            })
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => panic!("failed to create test terminal session: {error}"),
        };

        tokio::time::sleep(Duration::from_millis(100)).await;
        response.session.session_id.to_string()
    }

    async fn subscribe_to_session(
        state: &AppState,
        session_id: &str,
    ) -> tokio::sync::broadcast::Receiver<SessionEvent> {
        let daemon = state.daemon.lock().await;
        match daemon.subscribe(session_id) {
            Ok(subscription) => subscription,
            Err(error) => panic!("failed to subscribe to session `{session_id}`: {error}"),
        }
    }

    async fn collect_output_bytes(
        subscription: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
        expected_len: usize,
    ) -> Vec<u8> {
        let mut output = Vec::new();

        while output.len() < expected_len {
            let event =
                match tokio::time::timeout(Duration::from_secs(3), subscription.recv()).await {
                    Ok(Ok(event)) => event,
                    Ok(Err(error)) => panic!("terminal event stream failed: {error}"),
                    Err(_) => panic!("timed out waiting for terminal output"),
                };

            match event {
                SessionEvent::Output(bytes) => output.extend_from_slice(&bytes),
                SessionEvent::Exit { exit_code, state } => {
                    panic!(
                        "terminal session exited before output arrived: state={state:?} exit_code={exit_code:?}"
                    )
                },
                SessionEvent::Error(message) => {
                    panic!("terminal session reported an error: {message}")
                },
            }
        }

        output
    }

    async fn kill_session(state: &AppState, session_id: &str) {
        let result = {
            let mut daemon = state.daemon.lock().await;
            daemon.kill(KillRequest {
                session_id: session_id.into(),
            })
        };

        if let Err(error) = result {
            panic!("failed to kill test session `{session_id}`: {error}");
        }
    }
}
