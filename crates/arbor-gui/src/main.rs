mod actions;
mod agent_activity;
mod agent_presets;
mod app_config;
mod assets;
mod center_panel;
mod changes_pane;
mod checkout;
mod command_palette;
mod connection_history;
mod constants;
mod daemon_connection_ui;
mod daemon_runtime;
mod diff_engine;
mod diff_view;
mod error;
mod external_launchers;
mod file_view;
mod git_actions;
mod github_auth_modal;
mod github_auth_store;
mod github_helpers;
mod github_oauth;
mod github_service;
mod graphql;
mod helpers;
mod issue_cache_store;
mod issue_details_modal;
mod log_layer;
mod log_view;
mod manage_hosts;
mod mdns_browser;
mod notifications;
mod port_detection;
mod pr_summary_ui;
mod prompt_runner;
mod repo_presets;
mod repository_store;
mod settings_ui;
mod sidebar;
mod simple_http_client;
mod terminal_backend;
mod terminal_daemon_http;
mod terminal_interaction;
mod terminal_keys;
mod terminal_rendering;
mod theme;
mod theme_picker;
mod top_bar;
mod types;
mod ui_state_store;
mod welcome_ui;
mod workspace_layout;
mod workspace_navigation;
mod worktree_lifecycle;

pub(crate) use {
    actions::*, agent_activity::*, assets::*, constants::*, daemon_runtime::*, diff_engine::*,
    diff_view::*, error::*, external_launchers::*, file_view::*, git_actions::*, github_helpers::*,
    github_oauth::*, helpers::*, issue_details_modal::*, port_detection::*, pr_summary_ui::*,
    prompt_runner::*, repo_presets::*, settings_ui::*, terminal_rendering::*, theme_picker::*,
    types::*, workspace_layout::*,
};
use {
    arbor_core::{
        agent::AgentState,
        changes::{self, ChangeKind, ChangedFile},
        daemon::{
            self, CreateOrAttachRequest, DaemonSessionRecord, DetachRequest, KillRequest,
            ResizeRequest, SignalRequest, TerminalSessionState, TerminalSignal, WriteRequest,
        },
        process::{
            ProcessSource, managed_process_session_title,
            managed_process_source_and_name_from_title,
        },
        procfile, repo_config, worktree,
        worktree_scripts::{WorktreeScriptContext, WorktreeScriptPhase, run_worktree_scripts},
    },
    checkout::CheckoutKind,
    gix_diff::blob::v2::{
        Algorithm as DiffAlgorithm, Diff as BlobDiff, InternedInput as BlobInternedInput,
    },
    gpui::{
        Animation, AnimationExt, AnyElement, App, Application, Bounds, ClipboardItem, Context, Div,
        DragMoveEvent, ElementId, ElementInputHandler, EntityInputHandler, FocusHandle, FontWeight,
        Image, ImageFormat, KeyBinding, KeyDownEvent, Keystroke, Menu, MenuItem, MouseButton,
        MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, ScrollHandle,
        ScrollStrategy, Stateful, SystemMenuType, TextRun, TitlebarOptions, UTF16Selection,
        UniformListScrollHandle, Window, WindowBounds, WindowControlArea, WindowOptions, canvas,
        div, ease_in_out, fill, img, point, prelude::*, px, rgb, size, uniform_list,
    },
    ropey::Rope,
    std::{
        collections::{HashMap, HashSet},
        env, fs,
        net::TcpListener,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant, SystemTime},
    },
    syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet},
    terminal_backend::{
        EMBEDDED_TERMINAL_DEFAULT_BG, EMBEDDED_TERMINAL_DEFAULT_FG, EmbeddedTerminal,
        TerminalBackendKind, TerminalCursor, TerminalLaunch, TerminalModes, TerminalStyledCell,
        TerminalStyledLine, TerminalStyledRun,
    },
    theme::{ThemeKind, ThemePalette},
};

include!("app_bootstrap.rs");

impl ArborWindow {
    fn load_with_daemon_store<S>(
        startup_ui_state: ui_state_store::UiState,
        log_buffer: log_layer::LogBuffer,
        cx: &mut Context<Self>,
    ) -> Self
    where
        S: daemon::DaemonSessionStore + Default + 'static,
    {
        Self::load(Arc::new(S::default()), startup_ui_state, log_buffer, cx)
    }

    fn load(
        daemon_session_store: Arc<dyn daemon::DaemonSessionStore>,
        startup_ui_state: ui_state_store::UiState,
        log_buffer: log_layer::LogBuffer,
        cx: &mut Context<Self>,
    ) -> Self {
        let app_config_store = app_config::default_app_config_store();
        let repository_store = repository_store::default_repository_store();
        let ui_state_store = ui_state_store::default_ui_state_store();
        let issue_cache_store = issue_cache_store::default_issue_cache_store();
        let github_auth_store = github_auth_store::default_github_auth_store();
        let github_service = github_service::default_github_service();
        let notification_service = notifications::default_notification_service();
        let loaded_github_auth_state = github_auth_store.load().map_err(|e| e.to_string());
        let loaded_issue_cache = issue_cache_store.load().map_err(|e| e.to_string());
        let config_path = app_config_store.config_path();
        let cwd = match env::current_dir() {
            Ok(path) => path,
            Err(error) => {
                let mut notice_parts = vec![format!("failed to read current directory: {error}")];
                let loaded_config = app_config_store.load_or_create_config();
                notice_parts.extend(loaded_config.notices);
                let config_last_modified = app_config_store.config_last_modified();
                let github_auth_state = match loaded_github_auth_state.clone() {
                    Ok(state) => state,
                    Err(error) => {
                        notice_parts.push(format!("failed to load GitHub auth state: {error}"));
                        github_auth_store::GithubAuthState::default()
                    },
                };
                let startup_issue_cache = match loaded_issue_cache.clone() {
                    Ok(cache) => cache,
                    Err(error) => {
                        notice_parts.push(format!("failed to load issue cache: {error}"));
                        issue_cache_store::IssueCache::default()
                    },
                };

                let repositories = match repository_store.load_entries() {
                    Ok(entries) => repository_store::resolve_repositories_from_entries(entries),
                    Err(err) => {
                        notice_parts.push(format!("failed to load saved repositories: {err}"));
                        Vec::new()
                    },
                };
                let startup_repository_root = persisted_sidebar_selection_repository_root(
                    startup_ui_state.selected_sidebar_selection.as_ref(),
                );
                let active_repository_index = if let Some(root) = startup_repository_root.as_deref()
                {
                    repositories
                        .iter()
                        .position(|repository| repository.contains_checkout_root(root))
                        .or(Some(0))
                } else if repositories.is_empty() {
                    None
                } else {
                    Some(0)
                };
                let active_repository = active_repository_index
                    .and_then(|i| repositories.get(i))
                    .cloned();
                let repo_root = active_repository
                    .as_ref()
                    .map(|r| r.root.clone())
                    .unwrap_or_else(|| PathBuf::from("."));
                let github_repo_slug = active_repository
                    .as_ref()
                    .and_then(|repository| repository.github_repo_slug.clone());

                let active_backend_kind = match parse_terminal_backend_kind(
                    loaded_config.config.terminal_backend.as_deref(),
                ) {
                    Ok(kind) => kind,
                    Err(err) => {
                        notice_parts.push(err.to_string());
                        TerminalBackendKind::Embedded
                    },
                };
                let embedded_terminal_engine = resolve_embedded_terminal_engine(
                    loaded_config.config.embedded_terminal_engine.as_deref(),
                    &mut notice_parts,
                );
                tracing::info!(
                    terminal_engine = embedded_terminal_engine.as_str(),
                    "configured embedded terminal engine",
                );
                let theme_kind = match parse_theme_kind(loaded_config.config.theme.as_deref()) {
                    Ok(kind) => kind,
                    Err(err) => {
                        notice_parts.push(err.to_string());
                        ThemeKind::One
                    },
                };
                let startup_sidebar_order = startup_ui_state.sidebar_order.clone();
                let repository_sidebar_tabs = startup_ui_state.repository_sidebar_tabs.clone();
                let startup_collapsed_repository_groups =
                    startup_ui_state.collapsed_repository_group_keys.clone();
                let configured_embedded_shell = loaded_config.config.embedded_shell.clone();
                let notifications_enabled = loaded_config.config.notifications.unwrap_or(true);
                let remote_hosts: Vec<arbor_core::outpost::RemoteHost> = loaded_config
                    .config
                    .remote_hosts
                    .iter()
                    .map(|host_config| arbor_core::outpost::RemoteHost {
                        name: host_config.name.clone(),
                        hostname: host_config.hostname.clone(),
                        port: host_config.port,
                        user: host_config.user.clone(),
                        identity_file: host_config.identity_file.clone(),
                        remote_base_path: host_config.remote_base_path.clone(),
                        daemon_port: host_config.daemon_port,
                        mosh: host_config.mosh,
                        mosh_server_path: host_config.mosh_server_path.clone(),
                    })
                    .collect();
                let agent_presets = normalize_agent_presets(&loaded_config.config.agent_presets);
                let outpost_store = Arc::new(arbor_core::outpost_store::default_outpost_store());
                let outposts = load_outpost_summaries(outpost_store.as_ref(), &remote_hosts);
                let active_outpost_index = persisted_sidebar_selection_outpost_index(
                    startup_ui_state.selected_sidebar_selection.as_ref(),
                    &outposts,
                );
                let startup_right_pane_tab =
                    right_pane_tab_from_persisted(startup_ui_state.right_pane_tab);
                let startup_logs_tab_open = persisted_logs_tab_open(&startup_ui_state);
                let startup_logs_tab_active = persisted_logs_tab_active(&startup_ui_state);
                let pending_startup_worktree_restore = matches!(
                    startup_ui_state.selected_sidebar_selection.as_ref(),
                    Some(ui_state_store::PersistedSidebarSelection::Worktree { .. })
                );
                let collapsed_repositories = collapsed_repository_indices_from_group_keys(
                    &repositories,
                    &startup_collapsed_repository_groups,
                );
                let startup_issue_lists =
                    issue_cache_store::issue_lists_from_cache(&repositories, &startup_issue_cache);
                let (terminal_poll_tx, terminal_poll_rx) = std::sync::mpsc::channel();

                let app = Self {
                    app_config_store,
                    repository_store,
                    daemon_session_store,
                    terminal_daemon: None,
                    daemon_base_url: DEFAULT_DAEMON_BASE_URL.to_owned(),
                    ui_state_store,
                    issue_cache_store,
                    github_auth_store,
                    github_service,
                    github_auth_state,
                    github_auth_in_progress: false,
                    github_auth_copy_feedback_active: false,
                    github_auth_copy_feedback_generation: 0,
                    next_create_modal_instance_id: 1,
                    config_last_modified,
                    repositories,
                    active_repository_index,
                    repo_root: active_repository
                        .as_ref()
                        .map(|repository| repository.root.clone())
                        .or(startup_repository_root)
                        .unwrap_or(repo_root),
                    github_repo_slug,
                    worktrees: Vec::new(),
                    worktree_stats_loading: false,
                    worktree_prs_loading: false,
                    pending_startup_worktree_restore,
                    loading_animation_active: false,
                    loading_animation_frame: 0,
                    github_rate_limited_until: None,
                    expanded_pr_checks_worktree: None,
                    active_worktree_index: None,
                    pending_local_worktree_selection: None,
                    worktree_selection_epoch: 0,
                    changed_files: Vec::new(),
                    selected_changed_file: None,
                    terminals: Vec::new(),
                    terminal_poll_tx,
                    terminal_poll_rx: Some(terminal_poll_rx),
                    diff_sessions: Vec::new(),
                    active_diff_session_id: None,
                    file_view_sessions: Vec::new(),
                    active_file_view_session_id: None,
                    next_file_view_session_id: 1,
                    file_view_scroll_handle: UniformListScrollHandle::new(),
                    file_view_editing: false,
                    active_terminal_by_worktree: HashMap::new(),
                    next_terminal_id: 1,
                    next_diff_session_id: 1,
                    active_backend_kind,
                    configured_embedded_shell,
                    theme_kind,
                    left_pane_width: startup_ui_state
                        .left_pane_width
                        .map_or(DEFAULT_LEFT_PANE_WIDTH, |width| width as f32),
                    right_pane_width: startup_ui_state
                        .right_pane_width
                        .map_or(DEFAULT_RIGHT_PANE_WIDTH, |width| width as f32),
                    terminal_focus: cx.focus_handle(),
                    issue_details_focus: cx.focus_handle(),
                    welcome_clone_focus: cx.focus_handle(),
                    terminal_scroll_handle: ScrollHandle::new(),
                    issue_details_scroll_handle: ScrollHandle::new(),
                    issue_details_scrollbar_drag_offset: None,
                    last_terminal_grid_size: None,
                    center_tabs_scroll_handle: ScrollHandle::new(),
                    diff_scroll_handle: UniformListScrollHandle::new(),
                    terminal_selection: None,
                    terminal_selection_drag_anchor: None,
                    create_modal: None,
                    issue_details_modal: None,
                    preferred_checkout_kind: startup_ui_state
                        .preferred_checkout_kind
                        .unwrap_or_default(),
                    github_auth_modal: None,
                    delete_modal: None,
                    commit_modal: None,
                    outposts,
                    outpost_store,
                    active_outpost_index,
                    remote_hosts,
                    ssh_connection_pool: Arc::new(arbor_ssh::connection::SshConnectionPool::new()),
                    ssh_daemon_tunnel: None,
                    manage_hosts_modal: None,
                    manage_presets_modal: None,
                    agent_presets,
                    active_preset_tab: None,
                    repo_presets: Vec::new(),
                    manage_repo_presets_modal: None,
                    show_about: false,
                    show_theme_picker: false,
                    theme_picker_selected_index: theme_picker_index_for_kind(theme_kind),
                    theme_picker_scroll_handle: ScrollHandle::new(),
                    settings_modal: None,
                    daemon_auth_modal: None,
                    pending_remote_daemon_auth: None,
                    pending_remote_create_repo_root: None,
                    start_daemon_modal: false,
                    connect_to_host_modal: None,
                    command_palette_modal: None,
                    command_palette_scroll_handle: ScrollHandle::new(),
                    command_palette_recent_actions: Vec::new(),
                    command_palette_task_templates: Vec::new(),
                    compact_sidebar: startup_ui_state.compact_sidebar.unwrap_or(false),
                    execution_mode: startup_ui_state
                        .execution_mode
                        .unwrap_or(ExecutionMode::Build),
                    connection_history: connection_history::load_history(),
                    connection_history_save: PendingSave::default(),
                    repository_entries_save: PendingSave::default(),
                    daemon_auth_tokens: connection_history::load_tokens(),
                    daemon_auth_tokens_save: PendingSave::default(),
                    github_auth_state_save: PendingSave::default(),
                    pending_app_config_save_count: 0,
                    connected_daemon_label: None,
                    daemon_connect_epoch: 0,
                    pending_diff_scroll_to_file: None,
                    focus_terminal_on_next_render: true,
                    git_action_in_flight: None,
                    top_bar_quick_actions_open: false,
                    top_bar_quick_actions_submenu: None,
                    ide_launchers: Vec::new(),
                    last_persisted_ui_state: startup_ui_state,
                    pending_ui_state_save: None,
                    ui_state_save_in_flight: None,
                    last_persisted_issue_cache: startup_issue_cache,
                    pending_issue_cache_save: None,
                    issue_cache_save_in_flight: None,
                    daemon_session_store_save: PendingSave::default(),
                    last_ui_state_error: None,
                    last_issue_cache_error: None,
                    notification_service,
                    notifications_enabled,
                    agent_activity_sessions: HashMap::new(),
                    last_agent_finished_notifications: HashMap::new(),
                    auto_checkpoint_in_flight: Arc::new(Mutex::new(HashSet::new())),
                    agent_activity_epochs: Arc::new(Mutex::new(HashMap::new())),
                    window_is_active: true,
                    notice: (!notice_parts.is_empty()).then_some(notice_parts.join(" | ")),
                    theme_toast: None,
                    theme_toast_generation: 0,
                    right_pane_tab: startup_right_pane_tab,
                    right_pane_search: String::new(),
                    right_pane_search_cursor: 0,
                    right_pane_search_active: false,
                    sidebar_order: startup_sidebar_order,
                    repository_sidebar_tabs,
                    issue_lists: startup_issue_lists,
                    worktree_notes_lines: vec![String::new()],
                    worktree_notes_cursor: FileViewCursor { line: 0, col: 0 },
                    worktree_notes_path: None,
                    worktree_notes_active: false,
                    worktree_notes_error: None,
                    worktree_notes_save_pending: false,
                    worktree_notes_edit_generation: 0,
                    _worktree_notes_save_task: None,
                    file_tree_entries: Vec::new(),
                    file_tree_loading: false,
                    expanded_dirs: HashSet::new(),
                    selected_file_tree_entry: None,
                    left_pane_visible: true,
                    collapsed_repositories,
                    repository_context_menu: None,
                    worktree_context_menu: None,
                    worktree_hover_popover: None,
                    _hover_show_task: None,
                    _hover_dismiss_task: None,
                    _worktree_refresh_task: None,
                    _changed_files_refresh_task: None,
                    _config_refresh_task: None,
                    _repo_metadata_refresh_task: None,
                    _launcher_refresh_task: None,
                    _connection_history_save_task: None,
                    _repository_entries_save_task: None,
                    _daemon_auth_tokens_save_task: None,
                    _github_auth_state_save_task: None,
                    _ui_state_save_task: None,
                    _issue_cache_save_task: None,
                    _daemon_session_store_save_task: None,
                    _create_modal_preview_task: None,
                    _file_tree_refresh_task: None,
                    worktree_refresh_epoch: 0,
                    config_refresh_epoch: 0,
                    repo_metadata_refresh_epoch: 0,
                    launcher_refresh_epoch: 0,
                    last_mouse_position: point(px(0.), px(0.)),
                    outpost_context_menu: None,
                    discovered_daemons: Vec::new(),
                    mdns_browser: None,
                    active_discovered_daemon: None,
                    worktree_nav_back: Vec::new(),
                    worktree_nav_forward: Vec::new(),
                    log_buffer: log_buffer.clone(),
                    log_entries: Vec::new(),
                    log_generation: 0,
                    log_scroll_handle: ScrollHandle::new(),
                    log_auto_scroll: true,
                    logs_tab_open: startup_logs_tab_open,
                    logs_tab_active: startup_logs_tab_active,
                    quit_overlay_until: None,
                    quit_after_persistence_flush: false,
                    ime_marked_text: None,
                    welcome_clone_url: String::new(),
                    welcome_clone_url_cursor: 0,
                    welcome_clone_url_active: false,
                    welcome_cloning: false,
                    welcome_clone_error: None,
                    remote_daemon_states: HashMap::new(),
                    active_remote_worktree: None,
                };

                return app;
            },
        };

        let repo_root = worktree::repo_root(&cwd).ok();

        tracing::info!(config = %config_path.display(), "loading configuration");
        let loaded_config = app_config_store.load_or_create_config();
        let mut notice_parts = loaded_config.notices;
        let config_last_modified = app_config_store.config_last_modified();
        let github_auth_state = match loaded_github_auth_state {
            Ok(state) => state,
            Err(error) => {
                notice_parts.push(format!("failed to load GitHub auth state: {error}"));
                github_auth_store::GithubAuthState::default()
            },
        };
        let startup_issue_cache = match loaded_issue_cache {
            Ok(cache) => cache,
            Err(error) => {
                notice_parts.push(format!("failed to load issue cache: {error}"));
                issue_cache_store::IssueCache::default()
            },
        };

        if let Err(error) = daemon_session_store.load() {
            tracing::warn!(%error, "failed to load daemon session metadata");
            notice_parts.push(format!("failed to load daemon session metadata: {error}"));
        }
        let daemon_base_url =
            daemon_base_url_from_config(loaded_config.config.daemon_url.as_deref());
        tracing::info!(url = %daemon_base_url, "connecting to terminal daemon");
        let mut terminal_daemon =
            match terminal_daemon_http::default_terminal_daemon_client(&daemon_base_url) {
                Ok(client) => Some(client),
                Err(error) => {
                    tracing::error!(%error, url = %daemon_base_url, "invalid daemon URL");
                    notice_parts.push(format!("invalid daemon_url `{daemon_base_url}`: {error}"));
                    None
                },
            };
        let (initial_daemon_records, attach_daemon_runtime) =
            if let Some(daemon) = terminal_daemon.as_ref() {
                match daemon.list_sessions() {
                    Ok(records) => {
                        // Check for version mismatch on local daemons and restart if needed.
                        if daemon_url_is_local(&daemon_base_url) {
                            if let Some((records, restarted)) =
                                check_daemon_version_and_restart(daemon, &daemon_base_url)
                            {
                                if let Some(new_daemon) = restarted {
                                    terminal_daemon = Some(new_daemon);
                                }
                                (records, true)
                            } else {
                                (records, true)
                            }
                        } else {
                            (records, true)
                        }
                    },
                    Err(error) => {
                        let error_text = error.to_string();
                        if daemon_error_is_connection_refused(&error_text) {
                            tracing::debug!("daemon not running, attempting auto-start");
                            if let Some(started) = try_auto_start_daemon(&daemon_base_url) {
                                let records = started.list_sessions().unwrap_or_default();
                                terminal_daemon = Some(started);
                                (records, true)
                            } else {
                                tracing::debug!("auto-start failed, falling back to cold restore");
                                terminal_daemon = None;
                                let cold_records = daemon_session_store.load().unwrap_or_default();
                                (cold_records, false)
                            }
                        } else {
                            notice_parts.push(format!(
                                "failed to list terminal sessions from daemon at {}: {error}",
                                daemon.base_url()
                            ));
                            (Vec::new(), false)
                        }
                    },
                }
            } else {
                (Vec::new(), false)
            };

        let repository_store_file_exists = repository_store.has_store_file();
        let mut loaded_entries_were_empty = false;
        let mut repositories = match repository_store.load_entries() {
            Ok(entries) => {
                loaded_entries_were_empty = entries.is_empty();
                repository_store::resolve_repositories_from_entries(entries)
            },
            Err(error) => {
                notice_parts.push(format!("failed to load saved repositories: {error}"));
                Vec::new()
            },
        };
        let mut persist_repositories = false;

        if let Some(ref root) = repo_root
            && !repositories
                .iter()
                .any(|repository| repository.contains_checkout_root(root))
            && should_seed_repo_root_from_cwd(
                repository_store_file_exists,
                loaded_entries_were_empty,
            )
        {
            repositories.push(RepositorySummary::from_checkout_roots(
                root.clone(),
                repository_store::default_group_key_for_root(root),
                vec![repository_store::RepositoryCheckoutRoot {
                    path: root.clone(),
                    kind: CheckoutKind::LinkedWorktree,
                }],
            ));
            persist_repositories = true;
        }

        let startup_repository_root = persisted_sidebar_selection_repository_root(
            startup_ui_state.selected_sidebar_selection.as_ref(),
        );
        let preferred_repo_root = repo_root
            .clone()
            .or_else(|| startup_repository_root.clone());
        let active_repository_index = if let Some(ref root) = preferred_repo_root {
            repositories
                .iter()
                .position(|repository| repository.contains_checkout_root(root))
                .or(Some(0))
        } else if !repositories.is_empty() {
            Some(0)
        } else {
            None
        };
        let active_repository = active_repository_index
            .and_then(|index| repositories.get(index))
            .cloned();

        if persist_repositories {
            let entries_to_save =
                repository_store::repository_entries_from_summaries(&repositories);
            if let Err(error) = repository_store.save_entries(&entries_to_save) {
                notice_parts.push(format!("failed to save repositories: {error}"));
            }
        }

        let remote_hosts: Vec<arbor_core::outpost::RemoteHost> = loaded_config
            .config
            .remote_hosts
            .iter()
            .map(|host_config| arbor_core::outpost::RemoteHost {
                name: host_config.name.clone(),
                hostname: host_config.hostname.clone(),
                port: host_config.port,
                user: host_config.user.clone(),
                identity_file: host_config.identity_file.clone(),
                remote_base_path: host_config.remote_base_path.clone(),
                daemon_port: host_config.daemon_port,
                mosh: host_config.mosh,
                mosh_server_path: host_config.mosh_server_path.clone(),
            })
            .collect();
        let agent_presets = normalize_agent_presets(&loaded_config.config.agent_presets);

        let outpost_store = Arc::new(arbor_core::outpost_store::default_outpost_store());
        let outposts = load_outpost_summaries(outpost_store.as_ref(), &remote_hosts);
        let active_outpost_index = if repo_root.is_none() {
            persisted_sidebar_selection_outpost_index(
                startup_ui_state.selected_sidebar_selection.as_ref(),
                &outposts,
            )
        } else {
            None
        };

        let active_backend_kind =
            match parse_terminal_backend_kind(loaded_config.config.terminal_backend.as_deref()) {
                Ok(kind) => kind,
                Err(error) => {
                    notice_parts.push(error.to_string());
                    TerminalBackendKind::Embedded
                },
            };
        let embedded_terminal_engine = resolve_embedded_terminal_engine(
            loaded_config.config.embedded_terminal_engine.as_deref(),
            &mut notice_parts,
        );
        tracing::info!(
            terminal_engine = embedded_terminal_engine.as_str(),
            "configured embedded terminal engine",
        );
        let theme_kind = match parse_theme_kind(loaded_config.config.theme.as_deref()) {
            Ok(kind) => kind,
            Err(error) => {
                notice_parts.push(error.to_string());
                ThemeKind::One
            },
        };
        let startup_sidebar_order = startup_ui_state.sidebar_order.clone();
        let repository_sidebar_tabs = startup_ui_state.repository_sidebar_tabs.clone();
        let startup_collapsed_repository_groups =
            startup_ui_state.collapsed_repository_group_keys.clone();
        let configured_embedded_shell = loaded_config.config.embedded_shell.clone();
        let notifications_enabled = loaded_config.config.notifications.unwrap_or(true);
        let startup_right_pane_tab = right_pane_tab_from_persisted(startup_ui_state.right_pane_tab);
        let startup_logs_tab_open = persisted_logs_tab_open(&startup_ui_state);
        let startup_logs_tab_active = persisted_logs_tab_active(&startup_ui_state);
        let pending_startup_worktree_restore = matches!(
            startup_ui_state.selected_sidebar_selection.as_ref(),
            Some(ui_state_store::PersistedSidebarSelection::Worktree { .. })
        );
        let collapsed_repositories = collapsed_repository_indices_from_group_keys(
            &repositories,
            &startup_collapsed_repository_groups,
        );
        let startup_issue_lists =
            issue_cache_store::issue_lists_from_cache(&repositories, &startup_issue_cache);
        let (terminal_poll_tx, terminal_poll_rx) = std::sync::mpsc::channel();

        let mut app = Self {
            app_config_store,
            repository_store,
            daemon_session_store,
            terminal_daemon,
            daemon_base_url,
            ui_state_store,
            issue_cache_store,
            github_auth_store,
            github_service,
            github_auth_state,
            github_auth_in_progress: false,
            github_auth_copy_feedback_active: false,
            github_auth_copy_feedback_generation: 0,
            next_create_modal_instance_id: 1,
            config_last_modified,
            repositories,
            active_repository_index,
            repo_root: active_repository
                .as_ref()
                .map(|repository| repository.root.clone())
                .or(preferred_repo_root)
                .unwrap_or(cwd),
            github_repo_slug: active_repository.and_then(|repository| repository.github_repo_slug),
            worktrees: Vec::new(),
            worktree_stats_loading: false,
            worktree_prs_loading: false,
            pending_startup_worktree_restore,
            loading_animation_active: false,
            loading_animation_frame: 0,
            github_rate_limited_until: None,
            expanded_pr_checks_worktree: None,
            active_worktree_index: None,
            pending_local_worktree_selection: None,
            worktree_selection_epoch: 0,
            changed_files: Vec::new(),
            selected_changed_file: None,
            terminals: Vec::new(),
            terminal_poll_tx,
            terminal_poll_rx: Some(terminal_poll_rx),
            diff_sessions: Vec::new(),
            active_diff_session_id: None,
            file_view_sessions: Vec::new(),
            active_file_view_session_id: None,
            next_file_view_session_id: 1,
            file_view_scroll_handle: UniformListScrollHandle::new(),
            file_view_editing: false,
            active_terminal_by_worktree: HashMap::new(),
            next_terminal_id: 1,
            next_diff_session_id: 1,
            active_backend_kind,
            configured_embedded_shell,
            theme_kind,
            left_pane_width: startup_ui_state
                .left_pane_width
                .map_or(DEFAULT_LEFT_PANE_WIDTH, |width| width as f32),
            right_pane_width: startup_ui_state
                .right_pane_width
                .map_or(DEFAULT_RIGHT_PANE_WIDTH, |width| width as f32),
            terminal_focus: cx.focus_handle(),
            issue_details_focus: cx.focus_handle(),
            welcome_clone_focus: cx.focus_handle(),
            terminal_scroll_handle: ScrollHandle::new(),
            issue_details_scroll_handle: ScrollHandle::new(),
            issue_details_scrollbar_drag_offset: None,
            last_terminal_grid_size: None,
            center_tabs_scroll_handle: ScrollHandle::new(),
            diff_scroll_handle: UniformListScrollHandle::new(),
            terminal_selection: None,
            terminal_selection_drag_anchor: None,
            create_modal: None,
            issue_details_modal: None,
            preferred_checkout_kind: startup_ui_state.preferred_checkout_kind.unwrap_or_default(),
            github_auth_modal: None,
            delete_modal: None,
            commit_modal: None,
            outposts,
            outpost_store,
            active_outpost_index,
            remote_hosts,
            ssh_connection_pool: Arc::new(arbor_ssh::connection::SshConnectionPool::new()),
            ssh_daemon_tunnel: None,
            manage_hosts_modal: None,
            manage_presets_modal: None,
            agent_presets,
            active_preset_tab: None,
            repo_presets: Vec::new(),
            manage_repo_presets_modal: None,
            show_about: false,
            show_theme_picker: false,
            theme_picker_selected_index: theme_picker_index_for_kind(theme_kind),
            theme_picker_scroll_handle: ScrollHandle::new(),
            settings_modal: None,
            daemon_auth_modal: None,
            pending_remote_daemon_auth: None,
            pending_remote_create_repo_root: None,
            start_daemon_modal: false,
            connect_to_host_modal: None,
            command_palette_modal: None,
            command_palette_scroll_handle: ScrollHandle::new(),
            command_palette_recent_actions: Vec::new(),
            command_palette_task_templates: Vec::new(),
            compact_sidebar: startup_ui_state.compact_sidebar.unwrap_or(false),
            execution_mode: startup_ui_state
                .execution_mode
                .unwrap_or(ExecutionMode::Build),
            connection_history: connection_history::load_history(),
            connection_history_save: PendingSave::default(),
            repository_entries_save: PendingSave::default(),
            daemon_auth_tokens: connection_history::load_tokens(),
            daemon_auth_tokens_save: PendingSave::default(),
            github_auth_state_save: PendingSave::default(),
            pending_app_config_save_count: 0,
            connected_daemon_label: None,
            daemon_connect_epoch: 0,
            pending_diff_scroll_to_file: None,
            focus_terminal_on_next_render: true,
            git_action_in_flight: None,
            top_bar_quick_actions_open: false,
            top_bar_quick_actions_submenu: None,
            ide_launchers: Vec::new(),
            left_pane_visible: startup_ui_state.left_pane_visible.unwrap_or(true),
            collapsed_repositories,
            repository_context_menu: None,
            worktree_context_menu: None,
            worktree_hover_popover: None,
            _hover_show_task: None,
            _hover_dismiss_task: None,
            _worktree_refresh_task: None,
            _changed_files_refresh_task: None,
            _config_refresh_task: None,
            _repo_metadata_refresh_task: None,
            _launcher_refresh_task: None,
            _connection_history_save_task: None,
            _repository_entries_save_task: None,
            _daemon_auth_tokens_save_task: None,
            _github_auth_state_save_task: None,
            _ui_state_save_task: None,
            _issue_cache_save_task: None,
            _daemon_session_store_save_task: None,
            _create_modal_preview_task: None,
            _file_tree_refresh_task: None,
            worktree_refresh_epoch: 0,
            config_refresh_epoch: 0,
            repo_metadata_refresh_epoch: 0,
            launcher_refresh_epoch: 0,
            last_mouse_position: point(px(0.), px(0.)),
            outpost_context_menu: None,
            discovered_daemons: Vec::new(),
            mdns_browser: None,
            active_discovered_daemon: None,
            worktree_nav_back: Vec::new(),
            worktree_nav_forward: Vec::new(),
            last_persisted_ui_state: startup_ui_state,
            pending_ui_state_save: None,
            ui_state_save_in_flight: None,
            last_persisted_issue_cache: startup_issue_cache,
            pending_issue_cache_save: None,
            issue_cache_save_in_flight: None,
            daemon_session_store_save: PendingSave::default(),
            last_ui_state_error: None,
            last_issue_cache_error: None,
            notification_service,
            notifications_enabled,
            agent_activity_sessions: HashMap::new(),
            last_agent_finished_notifications: HashMap::new(),
            auto_checkpoint_in_flight: Arc::new(Mutex::new(HashSet::new())),
            agent_activity_epochs: Arc::new(Mutex::new(HashMap::new())),
            window_is_active: true,
            notice: (!notice_parts.is_empty()).then_some(notice_parts.join(" | ")),
            theme_toast: None,
            theme_toast_generation: 0,
            right_pane_tab: startup_right_pane_tab,
            right_pane_search: String::new(),
            right_pane_search_cursor: 0,
            right_pane_search_active: false,
            sidebar_order: startup_sidebar_order,
            repository_sidebar_tabs,
            issue_lists: startup_issue_lists,
            worktree_notes_lines: vec![String::new()],
            worktree_notes_cursor: FileViewCursor { line: 0, col: 0 },
            worktree_notes_path: None,
            worktree_notes_active: false,
            worktree_notes_error: None,
            worktree_notes_save_pending: false,
            worktree_notes_edit_generation: 0,
            _worktree_notes_save_task: None,
            file_tree_entries: Vec::new(),
            file_tree_loading: false,
            expanded_dirs: HashSet::new(),
            selected_file_tree_entry: None,
            log_buffer,
            log_entries: Vec::new(),
            log_generation: 0,
            log_scroll_handle: ScrollHandle::new(),
            log_auto_scroll: true,
            logs_tab_open: startup_logs_tab_open,
            logs_tab_active: startup_logs_tab_active,
            quit_overlay_until: None,
            quit_after_persistence_flush: false,
            ime_marked_text: None,
            welcome_clone_url: String::new(),
            welcome_clone_url_cursor: 0,
            welcome_clone_url_active: false,
            welcome_cloning: false,
            welcome_clone_error: None,
            remote_daemon_states: HashMap::new(),
            active_remote_worktree: None,
        };

        app.refresh_worktrees(cx);
        app.refresh_cached_issue_lists_on_startup(cx);
        app.refresh_repo_config_if_changed(cx);
        app.refresh_github_auth_identity(cx);
        app.restore_terminal_sessions_from_records(initial_daemon_records, attach_daemon_runtime);
        if app.active_outpost_index.is_some() {
            app.refresh_remote_changed_files(cx);
        } else {
            let _ = app.ensure_selected_worktree_terminal(cx);
        }
        app.sync_daemon_session_store(cx);
        app.start_terminal_poller(cx);
        app.start_log_poller(cx);
        app.start_worktree_auto_refresh(cx);
        app.start_github_pr_auto_refresh(cx);
        app.start_github_rate_limit_poller(cx);
        app.start_config_auto_refresh(cx);
        app.start_agent_activity_ws(cx);
        app.start_daemon_log_ws(cx);
        app.start_mdns_browser(cx);
        app.ensure_claude_code_hooks(cx);
        app.ensure_pi_agent_extension(cx);

        app
    }

    fn start_terminal_poller(&mut self, cx: &mut Context<Self>) {
        let Some(poll_rx) = self.terminal_poll_rx.take() else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let (bridge_tx, bridge_rx) = smol::channel::bounded::<()>(1);

            cx.background_spawn(async move {
                loop {
                    // Wait for a notification or fall back to 45ms timeout (for SSH/daemon
                    // terminals that use pull-based polling without a reader thread).
                    let _ = poll_rx.recv_timeout(Duration::from_millis(45));
                    // Drain queued notifications to coalesce burst output.
                    while poll_rx.try_recv().is_ok() {}
                    // Small deadline window to batch rapid output (e.g. `cat large_file`).
                    std::thread::sleep(Duration::from_millis(4));
                    while poll_rx.try_recv().is_ok() {}
                    if bridge_tx.send(()).await.is_err() {
                        break;
                    }
                }
            })
            .detach();

            loop {
                if bridge_rx.recv().await.is_err() {
                    break;
                }
                let updated = this.update(cx, |this, cx| this.sync_running_terminals(cx));
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn start_log_poller(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(LOG_POLLER_INTERVAL);
                })
                .await;

                let updated = this.update(cx, |this, cx| {
                    let current_generation = this.log_buffer.generation();
                    if current_generation == this.log_generation {
                        return;
                    }
                    this.log_generation = current_generation;
                    this.log_entries = this.log_buffer.snapshot();
                    if this.log_auto_scroll && this.logs_tab_active {
                        this.log_scroll_handle.scroll_to_bottom();
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn start_worktree_auto_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(WORKTREE_AUTO_REFRESH_INTERVAL);
                })
                .await;

                let updated = this.update(cx, |this, cx| {
                    if this.worktree_stats_loading {
                        return;
                    }

                    let refresh = this.refresh_worktree_inventory(
                        cx,
                        WorktreeInventoryRefreshMode::PreserveTerminalState,
                    );
                    if this.active_outpost_index.is_some() {
                        this.refresh_remote_changed_files(cx);
                    } else {
                        this.refresh_changed_files(cx);
                    }
                    if refresh.visible_change() {
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn start_github_pr_auto_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(GITHUB_PR_REFRESH_INTERVAL);
                })
                .await;

                let updated = this.update(cx, |this, cx| this.refresh_worktree_pull_requests(cx));
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn start_github_rate_limit_poller(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(Duration::from_secs(1));
                })
                .await;

                let updated = this.update(cx, |this, cx| {
                    if this.github_rate_limited_until.is_none() {
                        return;
                    }

                    if this.clear_expired_github_rate_limit() {
                        cx.notify();
                        return;
                    }

                    if this.github_rate_limit_remaining().is_some() {
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn start_config_auto_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(CONFIG_AUTO_REFRESH_INTERVAL);
                })
                .await;

                let updated = this.update(cx, |this, cx| {
                    this.refresh_config_if_changed(cx);
                    this.refresh_repo_config_if_changed(cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn has_active_loading_indicator(&self) -> bool {
        self.worktree_stats_loading
            || self.worktree_prs_loading
            || self.issue_lists.values().any(|state| state.loading)
            || self
                .create_modal
                .as_ref()
                .is_some_and(|modal| modal.managed_preview_loading)
    }

    fn ensure_loading_animation(&mut self, cx: &mut Context<Self>) {
        if self.loading_animation_active || !self.has_active_loading_indicator() {
            return;
        }

        self.loading_animation_active = true;

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(Duration::from_millis(100));
                })
                .await;

                let updated = this.update(cx, |this, cx| {
                    if !this.has_active_loading_indicator() {
                        this.loading_animation_active = false;
                        return false;
                    }

                    this.loading_animation_frame =
                        this.loading_animation_frame.wrapping_add(1) % LOADING_SPINNER_FRAMES.len();
                    cx.notify();
                    true
                });

                match updated {
                    Ok(true) => {},
                    Ok(false) | Err(_) => break,
                }
            }
        })
        .detach();
    }

    fn start_mdns_browser(&mut self, cx: &mut Context<Self>) {
        match mdns_browser::start_browsing() {
            Ok(browser) => {
                self.mdns_browser = Some(browser);
                tracing::info!("mDNS: browsing for _arbor._tcp services on the LAN");
            },
            Err(e) => {
                tracing::warn!("mDNS browsing unavailable: {e}");
                return;
            },
        }

        let local_hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_default();

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(Duration::from_secs(2));
                })
                .await;

                let updated = this.update(cx, |this, cx| {
                    if let Some(browser) = &this.mdns_browser {
                        let events = browser.poll_updates();
                        let mut changed = false;
                        for event in events {
                            match event {
                                mdns_browser::MdnsEvent::Added(daemon) => {
                                    // Skip our own instance
                                    if daemon.instance_name == local_hostname {
                                        tracing::debug!(
                                            name = %daemon.instance_name,
                                            "mDNS: ignoring own instance"
                                        );
                                        continue;
                                    }
                                    tracing::info!(
                                        name = %daemon.instance_name,
                                        host = %daemon.host,
                                        addresses = ?daemon.addresses,
                                        port = daemon.port,
                                        has_auth = daemon.has_auth,
                                        "mDNS: discovered LAN daemon"
                                    );
                                    // Update existing or insert new
                                    if let Some(existing) = this
                                        .discovered_daemons
                                        .iter_mut()
                                        .find(|d| d.instance_name == daemon.instance_name)
                                    {
                                        if existing != &daemon {
                                            *existing = daemon;
                                            changed = true;
                                        }
                                    } else {
                                        this.discovered_daemons.push(daemon);
                                        changed = true;
                                    }
                                },
                                mdns_browser::MdnsEvent::Removed(name) => {
                                    tracing::info!(name = %name, "mDNS: LAN daemon removed");
                                    let before = this.discovered_daemons.len();
                                    this.discovered_daemons.retain(|d| d.instance_name != name);
                                    if this.discovered_daemons.len() != before {
                                        changed = true;
                                        // Rebuild remote_daemon_states with new indices
                                        let new_states: HashMap<usize, RemoteDaemonState> = this
                                            .remote_daemon_states
                                            .drain()
                                            .filter(|(idx, _)| *idx < this.discovered_daemons.len())
                                            .collect();
                                        this.remote_daemon_states = new_states;
                                        if let Some(idx) = this.active_discovered_daemon
                                            && idx >= this.discovered_daemons.len()
                                        {
                                            this.active_discovered_daemon = None;
                                        }
                                    }
                                },
                            }
                        }
                        if changed {
                            cx.set_menus(build_app_menus(&this.discovered_daemons));
                            cx.notify();
                        }
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn refresh_config_if_changed(&mut self, cx: &mut Context<Self>) {
        struct ConfigRefreshOutcome {
            next_modified: Option<SystemTime>,
            next_theme_kind: Option<ThemeKind>,
            next_backend_kind: Option<TerminalBackendKind>,
            next_embedded_shell: Option<String>,
            next_daemon_base_url: String,
            next_terminal_daemon: Option<terminal_daemon_http::SharedTerminalDaemonClient>,
            daemon_records: Option<Vec<DaemonSessionRecord>>,
            daemon_connection_refused: bool,
            remote_hosts: Vec<arbor_core::outpost::RemoteHost>,
            agent_presets: Vec<AgentPreset>,
            notifications_enabled: bool,
            notices: Vec<String>,
        }

        let store = self.app_config_store.clone();
        let current_modified = self.config_last_modified;
        let current_daemon = self.terminal_daemon.clone();
        let current_daemon_base_url = self.daemon_base_url.clone();
        let next_epoch = self.config_refresh_epoch.wrapping_add(1);
        self.config_refresh_epoch = next_epoch;
        self._config_refresh_task = Some(cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    let next_modified = store.config_last_modified();
                    if next_modified == current_modified {
                        return None;
                    }

                    let loaded = store.load_or_create_config();
                    let mut notices = loaded.notices;

                    let next_theme_kind = match parse_theme_kind(loaded.config.theme.as_deref()) {
                        Ok(theme_kind) => Some(theme_kind),
                        Err(error) => {
                            notices.push(error.to_string());
                            None
                        },
                    };

                    let next_backend_kind =
                        match parse_terminal_backend_kind(loaded.config.terminal_backend.as_deref())
                        {
                            Ok(backend_kind) => Some(backend_kind),
                            Err(error) => {
                                notices.push(error.to_string());
                                None
                            },
                        };

                    let _ = resolve_embedded_terminal_engine(
                        loaded.config.embedded_terminal_engine.as_deref(),
                        &mut notices,
                    );

                    let next_daemon_base_url =
                        daemon_base_url_from_config(loaded.config.daemon_url.as_deref());
                    let daemon_url_changed = next_daemon_base_url != current_daemon_base_url;
                    if daemon_url_changed {
                        remove_claude_code_hooks();
                        remove_pi_agent_extension();
                    }

                    let next_terminal_daemon = if daemon_url_changed {
                        match terminal_daemon_http::default_terminal_daemon_client(
                            &next_daemon_base_url,
                        ) {
                            Ok(client) => Some(client),
                            Err(error) => {
                                notices.push(format!(
                                    "invalid daemon_url `{next_daemon_base_url}`: {error}"
                                ));
                                None
                            },
                        }
                    } else {
                        current_daemon.clone()
                    };

                    let mut daemon_records = None;
                    let mut daemon_connection_refused = false;
                    if let Some(daemon) = next_terminal_daemon.as_ref() {
                        match daemon.list_sessions() {
                            Ok(records) => daemon_records = Some(records),
                            Err(error) => {
                                let error_text = error.to_string();
                                daemon_connection_refused =
                                    daemon_error_is_connection_refused(&error_text);
                                if daemon_connection_refused {
                                    remove_claude_code_hooks();
                                    remove_pi_agent_extension();
                                }
                                if !daemon_connection_refused {
                                    notices.push(format!(
                                        "failed to list terminal sessions from daemon at {}: {error}",
                                        daemon.base_url()
                                    ));
                                }
                            },
                        }
                    }

                    let remote_hosts: Vec<arbor_core::outpost::RemoteHost> = loaded
                        .config
                        .remote_hosts
                        .iter()
                        .map(|host_config| arbor_core::outpost::RemoteHost {
                            name: host_config.name.clone(),
                            hostname: host_config.hostname.clone(),
                            port: host_config.port,
                            user: host_config.user.clone(),
                            identity_file: host_config.identity_file.clone(),
                            remote_base_path: host_config.remote_base_path.clone(),
                            daemon_port: host_config.daemon_port,
                            mosh: host_config.mosh,
                            mosh_server_path: host_config.mosh_server_path.clone(),
                        })
                        .collect();

                    Some(ConfigRefreshOutcome {
                        next_modified,
                        next_theme_kind,
                        next_backend_kind,
                        next_embedded_shell: loaded.config.embedded_shell.clone(),
                        next_daemon_base_url,
                        next_terminal_daemon,
                        daemon_records,
                        daemon_connection_refused,
                        remote_hosts,
                        agent_presets: normalize_agent_presets(&loaded.config.agent_presets),
                        notifications_enabled: loaded.config.notifications.unwrap_or(true),
                        notices,
                    })
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                if this.config_refresh_epoch != next_epoch {
                    return;
                }
                let Some(outcome) = outcome else {
                    return;
                };

                this.config_last_modified = outcome.next_modified;
                let mut changed = false;

                if let Some(theme_kind) = outcome.next_theme_kind
                    && this.theme_kind != theme_kind
                {
                    this.theme_kind = theme_kind;
                    changed = true;
                }
                if let Some(backend_kind) = outcome.next_backend_kind
                    && this.active_backend_kind != backend_kind
                {
                    this.active_backend_kind = backend_kind;
                    changed = true;
                }
                if this.configured_embedded_shell != outcome.next_embedded_shell {
                    this.configured_embedded_shell = outcome.next_embedded_shell.clone();
                    changed = true;
                }
                if this.daemon_base_url != outcome.next_daemon_base_url {
                    this.daemon_base_url = outcome.next_daemon_base_url.clone();
                    changed = true;
                }

                if outcome.daemon_connection_refused {
                    this.terminal_daemon = None;
                    changed = true;
                } else if this.terminal_daemon.as_ref().map(|daemon| daemon.base_url())
                    != outcome
                        .next_terminal_daemon
                        .as_ref()
                        .map(|daemon| daemon.base_url())
                {
                    this.terminal_daemon = outcome.next_terminal_daemon.clone();
                    changed = true;
                } else {
                    this.terminal_daemon = outcome.next_terminal_daemon.clone();
                }

                if let Some(records) = outcome.daemon_records {
                    changed |= this.restore_terminal_sessions_from_records(records, true);
                }

                if this.remote_hosts != outcome.remote_hosts {
                    this.remote_hosts = outcome.remote_hosts;
                    this.outposts =
                        load_outpost_summaries(this.outpost_store.as_ref(), &this.remote_hosts);
                    changed = true;
                }

                if this.agent_presets != outcome.agent_presets {
                    this.agent_presets = outcome.agent_presets;
                    if let Some(modal) = this.manage_presets_modal.as_mut()
                        && let Some(preset) = this
                            .agent_presets
                            .iter()
                            .find(|preset| preset.kind == modal.active_preset)
                    {
                        modal.command = preset.command.clone();
                    }
                    changed = true;
                }

                if this.notifications_enabled != outcome.notifications_enabled {
                    this.notifications_enabled = outcome.notifications_enabled;
                    changed = true;
                }

                if !outcome.notices.is_empty() {
                    this.notice = Some(outcome.notices.join(" | "));
                    changed = true;
                }

                if changed {
                    cx.notify();
                }
            });
        }));
    }

    fn refresh_repo_config_if_changed(&mut self, cx: &mut Context<Self>) {
        let repo_root = self.repo_root.clone();
        let result_repo_root = repo_root.clone();
        let selected_worktree_path = self.selected_worktree_path().map(Path::to_path_buf);
        let repositories = self.repositories.clone();
        let store = self.app_config_store.clone();
        let next_epoch = self.repo_metadata_refresh_epoch.wrapping_add(1);
        self.repo_metadata_refresh_epoch = next_epoch;
        self._repo_metadata_refresh_task = Some(cx.spawn(async move |this, cx| {
            let (next_presets, next_default_preset, task_templates) = cx
                .background_spawn(async move {
                    let mut presets = load_repo_presets(store.as_ref(), &repo_root);
                    if let Some(worktree_path) = selected_worktree_path
                        .as_ref()
                        .filter(|worktree_path| *worktree_path != &repo_root)
                    {
                        for preset in load_repo_presets(store.as_ref(), worktree_path) {
                            if !presets
                                .iter()
                                .any(|candidate| candidate.name == preset.name)
                            {
                                presets.push(preset);
                            }
                        }
                    }
                    let default_preset = store
                        .load_repo_config(&repo_root)
                        .and_then(|config| config.agent.default_preset)
                        .and_then(|value| AgentPresetKind::from_key(&value));
                    let mut task_templates = Vec::new();
                    for repository in repositories {
                        task_templates.extend(load_task_templates_for_repo(&repository.root));
                    }
                    (presets, default_preset, task_templates)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                if this.repo_metadata_refresh_epoch != next_epoch
                    || this.repo_root != result_repo_root
                {
                    return;
                }

                let mut changed = false;
                if this.repo_presets != next_presets {
                    this.repo_presets = next_presets;
                    changed = true;
                }
                if this.command_palette_task_templates != task_templates {
                    this.command_palette_task_templates = task_templates;
                    changed = true;
                }
                if this.active_preset_tab.is_none()
                    && let Some(preset) = next_default_preset
                {
                    this.active_preset_tab = Some(preset);
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            });
        }));
    }

    /// Returns the directory where repo preset edits should be saved.
    /// Prefers the selected worktree path, falls back to repo_root.
    fn active_arbor_toml_dir(&self) -> PathBuf {
        self.selected_worktree_path()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.repo_root.clone())
    }

    fn selected_agent_preset_or_default(&self) -> AgentPresetKind {
        self.active_preset_tab.unwrap_or(AgentPresetKind::Codex)
    }

    fn branch_prefix_github_login(&self) -> Option<String> {
        self.github_auth_state
            .user_login
            .clone()
            .or_else(|| env::var("ARBOR_GITHUB_USER").ok())
            .or_else(|| env::var("GITHUB_USER").ok())
            .and_then(|value| non_empty_trimmed_str(&value).map(str::to_owned))
    }

    fn sync_daemon_session_store(&mut self, cx: &mut Context<Self>) {
        let records = self.daemon_session_records_snapshot();
        self.daemon_session_store_save.queue(records);
        self.start_pending_daemon_session_store_save(cx);
    }

    fn daemon_session_records_snapshot(&self) -> Vec<DaemonSessionRecord> {
        let shell = self.embedded_shell();
        let updated_at_unix_ms = current_unix_timestamp_millis();
        self.terminals
            .iter()
            .map(|session| DaemonSessionRecord {
                session_id: session.daemon_session_id.clone().into(),
                workspace_id: session.worktree_path.display().to_string().into(),
                cwd: session.worktree_path.clone(),
                shell: if session.command.trim().is_empty() {
                    shell.clone()
                } else {
                    session.command.clone()
                },
                root_pid: session.root_pid,
                cols: session.cols.max(2),
                rows: session.rows.max(1),
                title: Some(session.title.clone()),
                last_command: session.last_command.clone(),
                output_tail: Some(terminal_output_tail_for_metadata(session, 64, 24_000)),
                exit_code: session.exit_code,
                state: Some(daemon_state_from_terminal_state(session.state)),
                updated_at_unix_ms: session.updated_at_unix_ms.or(updated_at_unix_ms),
            })
            .collect()
    }

    fn start_pending_daemon_session_store_save(&mut self, cx: &mut Context<Self>) {
        let Some(records) = self.daemon_session_store_save.begin_next() else {
            self.maybe_finish_quit_after_persistence_flush(cx);
            return;
        };

        let store = self.daemon_session_store.clone();
        self._daemon_session_store_save_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.save(&records) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.daemon_session_store_save.finish();
                if let Err(error) = result {
                    this.notice = Some(format!("failed to persist daemon sessions: {error}"));
                    cx.notify();
                }

                this.start_pending_daemon_session_store_save(cx);
                this.maybe_finish_quit_after_persistence_flush(cx);
            });
        }));
    }

    fn maybe_finish_quit_after_persistence_flush(&mut self, cx: &mut Context<Self>) {
        if !self.quit_after_persistence_flush {
            return;
        }

        if self.daemon_session_store_save.has_work()
            || self.connection_history_save.has_work()
            || self.repository_entries_save.has_work()
            || self.daemon_auth_tokens_save.has_work()
            || self.github_auth_state_save.has_work()
            || background_config_save_has_work(self.pending_app_config_save_count)
            || ui_state_save_has_work(
                self.pending_ui_state_save.as_ref(),
                self.ui_state_save_in_flight.as_ref(),
            )
            || issue_cache_save_has_work(
                self.pending_issue_cache_save.as_ref(),
                self.issue_cache_save_in_flight.as_ref(),
            )
            || self.worktree_notes_save_pending
            || self._worktree_notes_save_task.is_some()
        {
            return;
        }

        self.quit_after_persistence_flush = false;
        self.stop_active_ssh_daemon_tunnel();
        cx.quit();
    }

    fn request_quit_after_persistence_flush(&mut self, cx: &mut Context<Self>) {
        self.quit_after_persistence_flush = true;
        self.sync_daemon_session_store(cx);
        self.maybe_finish_quit_after_persistence_flush(cx);
    }

    fn restore_terminal_sessions_from_records(
        &mut self,
        mut records: Vec<DaemonSessionRecord>,
        attach_runtime: bool,
    ) -> bool {
        if records.is_empty() {
            return false;
        }

        // Don't restore terminals without a live runtime — they become
        // non-functional "ghost" sessions that show old output but cannot
        // accept input.  A fresh terminal will be created on demand.
        if !attach_runtime {
            tracing::debug!(
                count = records.len(),
                "skipping cold terminal restore (no daemon runtime available)"
            );
            return false;
        }

        records.sort_by(|left, right| {
            right
                .updated_at_unix_ms
                .unwrap_or(0)
                .cmp(&left.updated_at_unix_ms.unwrap_or(0))
                .then_with(|| left.session_id.as_str().cmp(right.session_id.as_str()))
        });

        let mut changed = false;

        for record in records {
            if record.session_id.as_str().trim().is_empty() {
                continue;
            }

            let Some(worktree_path) = self.worktree_path_for_session_record(&record) else {
                continue;
            };
            let session_state = terminal_state_from_daemon_record(&record);
            let title = record
                .title
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("term-{}", self.next_terminal_id));
            let managed_process_id = managed_process_id_from_title(&worktree_path, &title);
            let command = record.shell.clone();
            let output = record.output_tail.clone().unwrap_or_default();
            let cols = record.cols.max(2);
            let rows = record.rows.max(1);

            if let Some(session) = self
                .terminals
                .iter_mut()
                .find(|session| session.daemon_session_id == record.session_id.as_str())
            {
                if session.worktree_path != worktree_path {
                    session.worktree_path = worktree_path.clone();
                    changed = true;
                }
                if session.title != title {
                    session.title = title.clone();
                    changed = true;
                }
                if session.managed_process_id != managed_process_id {
                    session.managed_process_id = managed_process_id.clone();
                    changed = true;
                }
                if session.command != command {
                    session.command = command.clone();
                    changed = true;
                }
                if session.output != output {
                    session.output = output.clone();
                    session.styled_output.clear();
                    session.cursor = None;
                    session.modes = TerminalModes::default();
                    changed = true;
                }
                if session.state != session_state {
                    session.state = session_state;
                    changed = true;
                }
                if session.exit_code != record.exit_code {
                    session.exit_code = record.exit_code;
                    changed = true;
                }
                if session.updated_at_unix_ms != record.updated_at_unix_ms {
                    session.updated_at_unix_ms = record.updated_at_unix_ms;
                    changed = true;
                }
                if session.root_pid != record.root_pid {
                    session.root_pid = record.root_pid;
                    changed = true;
                }
                if session.cols != cols || session.rows != rows {
                    session.cols = cols;
                    session.rows = rows;
                    changed = true;
                }
                if attach_runtime && let Some(daemon) = self.terminal_daemon.as_ref() {
                    session.runtime = Some(local_daemon_runtime(
                        daemon.clone(),
                        session.daemon_session_id.clone(),
                        session.rows,
                        session.cols,
                        Some(self.terminal_poll_tx.clone()),
                    ));
                    changed = true;
                }
            } else {
                let session_id = self.next_terminal_id;
                self.next_terminal_id += 1;
                self.terminals.push(TerminalSession {
                    id: session_id,
                    daemon_session_id: record.session_id.to_string(),
                    worktree_path: worktree_path.clone(),
                    managed_process_id,
                    title,
                    last_command: record.last_command.clone(),
                    pending_command: String::new(),
                    command,
                    agent_preset: None,
                    execution_mode: None,
                    state: session_state,
                    exit_code: record.exit_code,
                    updated_at_unix_ms: record.updated_at_unix_ms,
                    root_pid: record.root_pid,
                    cols,
                    rows,
                    generation: 0,
                    output,
                    styled_output: Vec::new(),
                    cursor: None,
                    modes: TerminalModes::default(),
                    last_runtime_sync_at: None,
                    queued_input: Vec::new(),
                    is_initializing: false,
                    runtime: attach_runtime
                        .then(|| {
                            self.terminal_daemon.as_ref().map(|daemon| {
                                local_daemon_runtime(
                                    daemon.clone(),
                                    record.session_id.to_string(),
                                    rows,
                                    cols,
                                    Some(self.terminal_poll_tx.clone()),
                                )
                            })
                        })
                        .flatten(),
                });
                changed = true;
            }

            let mapped_terminal_id = self
                .terminals
                .iter()
                .find(|session| session.daemon_session_id == record.session_id.as_str())
                .map(|session| session.id);
            if let Some(mapped_terminal_id) = mapped_terminal_id {
                self.active_terminal_by_worktree
                    .entry(worktree_path)
                    .or_insert(mapped_terminal_id);
            }
        }

        changed
    }

    fn worktree_path_for_session_record(&self, record: &DaemonSessionRecord) -> Option<PathBuf> {
        if let Some(path) = self.match_worktree_path(record.cwd.as_path()) {
            return Some(path);
        }

        let workspace_path = PathBuf::from(record.workspace_id.to_string());
        self.match_worktree_path(workspace_path.as_path())
    }

    fn match_worktree_path(&self, candidate: &Path) -> Option<PathBuf> {
        self.worktrees
            .iter()
            .find(|worktree| paths_equivalent(worktree.path.as_path(), candidate))
            .map(|worktree| worktree.path.clone())
    }

    fn maybe_notify(&self, title: &str, body: &str, play_sound: bool) {
        if self.notifications_enabled && !self.window_is_active {
            self.notification_service.send(title, body, play_sound);
        }
    }

    fn maybe_notify_agent_finished(&mut self, worktree: &WorktreeSummary, updated_at: Option<u64>) {
        if !should_emit_agent_finished_notification(
            &mut self.last_agent_finished_notifications,
            &worktree.path,
            updated_at.or(worktree.last_activity_unix_ms),
        ) {
            return;
        }

        let repo_name = repository_display_name(&worktree.repo_root);
        let branch = worktree::short_branch(&worktree.branch);
        let body = if let Some(task) = worktree.agent_task.as_deref() {
            format!(
                "{} · {} · {} is waiting: {task}",
                repo_name, worktree.label, branch
            )
        } else {
            format!("{} · {} · {} is waiting", repo_name, worktree.label, branch)
        };
        self.maybe_notify("Agent finished", &body, true);
    }

    fn sync_running_terminals(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut should_refresh_ports = false;
        let follow_output = terminal_scroll_is_near_bottom(&self.terminal_scroll_handle);
        let active_terminal_id = self.active_terminal_id_for_selected_worktree();
        let target_grid_size =
            terminal_grid_size_from_scroll_handle(&self.terminal_scroll_handle, cx);
        let now = Instant::now();
        if let Some((rows, cols, ..)) = target_grid_size {
            self.last_terminal_grid_size = Some((rows, cols));
        }
        let mut sessions_to_close = Vec::new();
        let mut pending_notifications = Vec::new();
        let sync_indices = ordered_terminal_sync_indices(&self.terminals, active_terminal_id);

        for index in sync_indices {
            let Some(runtime) = self
                .terminals
                .get(index)
                .and_then(|session| session.runtime.clone())
            else {
                continue;
            };

            let session_id = self.terminals[index].id;
            let is_active = active_terminal_id == Some(session_id);
            if !runtime.should_sync(&self.terminals[index], is_active, target_grid_size, now) {
                continue;
            }
            let outcome = {
                let session = &mut self.terminals[index];
                runtime.sync(session, is_active, target_grid_size)
            };
            self.terminals[index].last_runtime_sync_at = Some(now);

            changed |= outcome.changed;
            if outcome.changed {
                let recent_output =
                    terminal_output_tail_for_metadata(&self.terminals[index], 24, 4_000);
                if output_contains_port_hint(&recent_output) {
                    should_refresh_ports = true;
                }
            }

            if outcome.clear_global_daemon {
                self.terminal_daemon = None;
            }
            if let Some(notice) = outcome.notice {
                self.notice = Some(notice);
            }
            if let Some(notification) = outcome.notification {
                pending_notifications.push(notification);
            }
            if outcome.close_session {
                sessions_to_close.push(session_id);
            }
        }

        for notification in pending_notifications {
            self.maybe_notify(
                &notification.title,
                &notification.body,
                notification.play_sound,
            );
        }

        for session_id in sessions_to_close {
            changed |= self.close_terminal_session_by_id(session_id);
        }

        if changed {
            self.sync_daemon_session_store(cx);
            if should_refresh_ports {
                self.refresh_worktree_ports(cx);
            }
            if should_auto_follow_terminal_output(changed, follow_output) {
                self.terminal_scroll_handle.scroll_to_bottom();
            }
            cx.notify();
        }
    }

    fn refresh_worktree_inventory(
        &mut self,
        cx: &mut Context<Self>,
        mode: WorktreeInventoryRefreshMode,
    ) -> WorktreeInventoryRefreshResult {
        let queued_ui_state = self.queued_ui_state_base();
        let previous_local_selection = refresh_worktree_previous_local_selection(
            self.pending_local_worktree_selection.as_deref(),
            self.selected_local_worktree_path(),
            queued_ui_state.selected_sidebar_selection.as_ref(),
        );
        let active_repository_group_key = self
            .active_repository_index
            .and_then(|repository_index| self.repositories.get(repository_index))
            .map(|repository| repository.group_key.clone());
        let preserve_non_local_selection =
            self.active_outpost_index.is_some() || self.active_remote_worktree.is_some();
        let repositories = self.repositories.clone();
        let previous_summaries: HashMap<PathBuf, changes::DiffLineSummary> = self
            .worktrees
            .iter()
            .filter_map(|worktree| {
                worktree
                    .diff_summary
                    .map(|summary| (worktree.path.clone(), summary))
            })
            .collect();
        let previous_pr_numbers: HashMap<PathBuf, u64> = self
            .worktrees
            .iter()
            .filter_map(|worktree| {
                worktree
                    .pr_number
                    .map(|pr_number| (worktree.path.clone(), pr_number))
            })
            .collect();
        let previous_branches: HashMap<PathBuf, String> = self
            .worktrees
            .iter()
            .map(|worktree| (worktree.path.clone(), worktree.branch.clone()))
            .collect();
        let previous_pr_loading: HashMap<PathBuf, bool> = self
            .worktrees
            .iter()
            .map(|worktree| (worktree.path.clone(), worktree.pr_loading))
            .collect();
        let previous_pr_loaded: HashMap<PathBuf, bool> = self
            .worktrees
            .iter()
            .map(|worktree| (worktree.path.clone(), worktree.pr_loaded))
            .collect();
        let previous_pr_urls: HashMap<PathBuf, String> = self
            .worktrees
            .iter()
            .filter_map(|worktree| {
                worktree
                    .pr_url
                    .as_ref()
                    .map(|pr_url| (worktree.path.clone(), pr_url.clone()))
            })
            .collect();
        let previous_pr_details: HashMap<PathBuf, github_service::PrDetails> = self
            .worktrees
            .iter()
            .filter_map(|worktree| {
                worktree
                    .pr_details
                    .as_ref()
                    .map(|details| (worktree.path.clone(), details.clone()))
            })
            .collect();
        let previous_agent_states: HashMap<PathBuf, AgentState> = self
            .worktrees
            .iter()
            .filter_map(|worktree| {
                worktree
                    .agent_state
                    .map(|state| (worktree.path.clone(), state))
            })
            .collect();
        let previous_agent_tasks: HashMap<PathBuf, String> = self
            .worktrees
            .iter()
            .filter_map(|worktree| {
                worktree
                    .agent_task
                    .as_ref()
                    .map(|task| (worktree.path.clone(), task.clone()))
            })
            .collect();
        let previous_recent_turns: HashMap<PathBuf, Vec<AgentTurnSnapshot>> = self
            .worktrees
            .iter()
            .map(|worktree| (worktree.path.clone(), worktree.recent_turns.clone()))
            .collect();
        let previous_detected_ports: HashMap<PathBuf, Vec<DetectedPort>> = self
            .worktrees
            .iter()
            .map(|worktree| (worktree.path.clone(), worktree.detected_ports.clone()))
            .collect();
        let previous_recent_agent_sessions: HashMap<
            PathBuf,
            Vec<arbor_core::session::AgentSessionSummary>,
        > = self
            .worktrees
            .iter()
            .map(|worktree| {
                (
                    worktree.path.clone(),
                    worktree.recent_agent_sessions.clone(),
                )
            })
            .collect();
        let previous_stuck_turn_counts: HashMap<PathBuf, usize> = self
            .worktrees
            .iter()
            .map(|worktree| (worktree.path.clone(), worktree.stuck_turn_count))
            .collect();
        let previous_activity: HashMap<PathBuf, u64> = self
            .worktrees
            .iter()
            .filter_map(|worktree| {
                worktree
                    .last_activity_unix_ms
                    .map(|ts| (worktree.path.clone(), ts))
            })
            .collect();
        let persisted_pr_cache = self.last_persisted_ui_state.pull_request_cache.clone();
        let next_epoch = self.worktree_refresh_epoch.wrapping_add(1);
        self.worktree_refresh_epoch = next_epoch;
        self._worktree_refresh_task = Some(cx.spawn(async move |this, cx| {
            let (mut next_worktrees, refresh_errors) = cx
                .background_spawn(async move {
                    let mut refresh_errors = Vec::new();
                    let mut next_worktrees = Vec::new();
                    let mut seen_worktree_paths = HashSet::new();
                    for repository in &repositories {
                        for checkout_root in &repository.checkout_roots {
                            match worktree::list(&checkout_root.path) {
                                Ok(entries) => {
                                    for entry in entries {
                                        if !seen_worktree_paths.insert(entry.path.clone()) {
                                            continue;
                                        }
                                        next_worktrees.push(WorktreeSummary::from_worktree(
                                            &entry,
                                            &checkout_root.path,
                                            &repository.group_key,
                                            if checkout_root.kind == CheckoutKind::DiscreteClone
                                                && entry.path == checkout_root.path
                                            {
                                                CheckoutKind::DiscreteClone
                                            } else {
                                                CheckoutKind::LinkedWorktree
                                            },
                                        ));
                                    }
                                },
                                Err(error) => refresh_errors.push(format!(
                                    "{} ({}): {error}",
                                    repository.label,
                                    checkout_root.path.display()
                                )),
                            }
                        }
                    }
                    (next_worktrees, refresh_errors)
                })
                .await;

            for worktree in &mut next_worktrees {
                let branch_unchanged = previous_branches
                    .get(&worktree.path)
                    .is_some_and(|previous_branch| previous_branch == &worktree.branch);
                worktree.pr_loading = branch_unchanged
                    && previous_pr_loading
                        .get(&worktree.path)
                        .copied()
                        .unwrap_or(false);
                worktree.pr_loaded = branch_unchanged
                    && previous_pr_loaded
                        .get(&worktree.path)
                        .copied()
                        .unwrap_or(false);
                worktree.diff_summary = previous_summaries.get(&worktree.path).copied();
                if branch_unchanged {
                    worktree.pr_number = previous_pr_numbers.get(&worktree.path).copied();
                    worktree.pr_url = previous_pr_urls.get(&worktree.path).cloned();
                    worktree.pr_details = previous_pr_details.get(&worktree.path).cloned();
                } else if let Some(cached) =
                    cached_pull_request_state_for_worktree(worktree, &persisted_pr_cache)
                {
                    worktree.apply_cached_pull_request_state(cached);
                }
                worktree.agent_state = previous_agent_states.get(&worktree.path).copied();
                worktree.agent_task = previous_agent_tasks.get(&worktree.path).cloned();
                worktree.detected_ports = previous_detected_ports
                    .get(&worktree.path)
                    .cloned()
                    .unwrap_or_default();
                worktree.recent_turns = previous_recent_turns
                    .get(&worktree.path)
                    .cloned()
                    .unwrap_or_default();
                worktree.recent_agent_sessions = previous_recent_agent_sessions
                    .get(&worktree.path)
                    .cloned()
                    .unwrap_or_default();
                worktree.stuck_turn_count = previous_stuck_turn_counts
                    .get(&worktree.path)
                    .copied()
                    .unwrap_or_default();
                let previous = previous_activity.get(&worktree.path).copied();
                worktree.last_activity_unix_ms = match (worktree.last_activity_unix_ms, previous) {
                    (Some(left), Some(right)) => Some(left.max(right)),
                    (left, right) => left.or(right),
                };
            }

            let _ =
                this.update(cx, |this, cx| {
                    if this.worktree_refresh_epoch != next_epoch {
                        return;
                    }

                    let should_refresh_pull_requests =
                        should_refresh_pull_requests_after_worktree_refresh(
                            &this.worktrees,
                            &next_worktrees,
                        );
                    let rows_changed = worktree_rows_changed(&this.worktrees, &next_worktrees);
                    this.worktrees = next_worktrees;
                    reconcile_worktree_agent_activity(this, false, cx);
                    this.worktree_stats_loading = this
                        .worktrees
                        .iter()
                        .any(|worktree| worktree.diff_summary.is_none());

                    this.active_worktree_index = next_active_worktree_index(
                        previous_local_selection.as_deref(),
                        active_repository_group_key.as_deref(),
                        &this.worktrees,
                        preserve_non_local_selection,
                    );
                    if this
                        .pending_local_worktree_selection
                        .as_ref()
                        .is_some_and(|path| {
                            this.worktrees
                                .iter()
                                .any(|worktree| worktree.path.as_path() == path.as_path())
                        })
                    {
                        this.pending_local_worktree_selection = None;
                    }
                    if this.pending_startup_worktree_restore
                        && (this.active_worktree().is_some() || refresh_errors.is_empty())
                    {
                        this.pending_startup_worktree_restore = false;
                    }
                    if this.right_pane_tab == RightPaneTab::FileTree
                        && this.file_tree_entries.is_empty()
                    {
                        this.rebuild_file_tree(cx);
                    }

                    this.active_terminal_by_worktree.retain(|path, _| {
                        this.worktrees
                            .iter()
                            .any(|worktree| worktree.path.as_path() == path.as_path())
                    });
                    this.diff_sessions.retain(|session| {
                        this.worktrees
                            .iter()
                            .any(|worktree| worktree.path == session.worktree_path)
                    });
                    if this.active_diff_session_id.is_some_and(|diff_id| {
                        !this
                            .diff_sessions
                            .iter()
                            .any(|session| session.id == diff_id)
                    }) {
                        this.active_diff_session_id = None;
                    }

                    this.sync_active_repository_from_selected_worktree();
                    this.sync_visible_repository_issue_tabs(cx);
                    this.sync_issue_cache_store(cx);
                    this.sync_pull_request_cache_store(cx);
                    this.sync_navigation_ui_state_store(cx);

                    if refresh_errors.is_empty() {
                        if this.notice.as_deref().is_some_and(|notice| {
                            notice.starts_with("failed to refresh worktrees:")
                        }) {
                            this.notice = None;
                        }
                    } else {
                        this.worktree_stats_loading = false;
                        this.notice = Some(format!(
                            "failed to refresh worktrees: {}",
                            refresh_errors.join(", ")
                        ));
                    }

                    this.refresh_worktree_diff_summaries(cx);
                    this.refresh_worktree_ports(cx);
                    this.refresh_agent_tasks(cx);
                    this.refresh_agent_sessions(cx);
                    if should_refresh_pull_requests {
                        this.refresh_worktree_pull_requests(cx);
                    }
                    if this.active_outpost_index.is_some() {
                        this.refresh_remote_changed_files(cx);
                    } else {
                        this.refresh_changed_files(cx);
                    }
                    this.sync_selected_worktree_notes(cx);
                    let created_terminal =
                        mode.created_terminal(|| this.ensure_selected_worktree_terminal(cx));
                    if created_terminal {
                        this.sync_daemon_session_store(cx);
                    }
                    if rows_changed || created_terminal {
                        cx.notify();
                    }
                });
        }));

        WorktreeInventoryRefreshResult::default()
    }

    fn refresh_worktrees(&mut self, cx: &mut Context<Self>) {
        tracing::debug!("refreshing worktrees");
        let refresh = self
            .refresh_worktree_inventory(cx, WorktreeInventoryRefreshMode::EnsureSelectedTerminal);
        if refresh.visible_change() {
            cx.notify();
        }
    }

    fn refresh_worktree_diff_summaries(&mut self, cx: &mut Context<Self>) {
        let worktree_paths: Vec<PathBuf> = self
            .worktrees
            .iter()
            .map(|worktree| worktree.path.clone())
            .collect();
        if worktree_paths.is_empty() {
            self.worktree_stats_loading = false;
            return;
        }

        cx.spawn(async move |this, cx| {
            let summaries = cx
                .background_spawn(async move {
                    let mut results = Vec::with_capacity(worktree_paths.len());
                    for path in worktree_paths {
                        results.push((path.clone(), changes::diff_line_summary(&path)));
                    }
                    results
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                let mut changed = false;
                for (path, summary_result) in summaries {
                    if let Some(worktree) = this
                        .worktrees
                        .iter_mut()
                        .find(|worktree| worktree.path == path)
                    {
                        let next_summary = summary_result.ok();
                        if worktree.diff_summary != next_summary {
                            worktree.diff_summary = next_summary;
                            changed = true;
                        }
                    }
                }
                if this.worktree_stats_loading {
                    this.worktree_stats_loading = false;
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn refresh_agent_tasks(&mut self, cx: &mut Context<Self>) {
        let worktree_paths: Vec<PathBuf> = self
            .worktrees
            .iter()
            .filter(|wt| wt.agent_task.is_none())
            .map(|wt| wt.path.clone())
            .collect();
        if worktree_paths.is_empty() {
            return;
        }

        cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move {
                    worktree_paths
                        .into_iter()
                        .map(|path| {
                            let task = arbor_core::session::extract_agent_task(&path);
                            (path, task)
                        })
                        .collect::<Vec<_>>()
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                let mut changed = false;
                for (path, task) in results {
                    if let Some(task) = task
                        && let Some(wt) = this.worktrees.iter_mut().find(|wt| wt.path == path)
                    {
                        wt.agent_task = Some(task);
                        changed = true;
                    }
                }
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn refresh_agent_sessions(&mut self, cx: &mut Context<Self>) {
        let worktree_paths: Vec<PathBuf> = self
            .worktrees
            .iter()
            .filter(|worktree| worktree.recent_agent_sessions.is_empty())
            .map(|worktree| worktree.path.clone())
            .collect();
        if worktree_paths.is_empty() {
            return;
        }

        cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move {
                    worktree_paths
                        .into_iter()
                        .map(|path| {
                            let sessions = arbor_core::session::recent_agent_sessions(&path, 6);
                            (path, sessions)
                        })
                        .collect::<Vec<_>>()
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                let mut changed = false;
                for (path, sessions) in results {
                    if let Some(worktree) = this
                        .worktrees
                        .iter_mut()
                        .find(|worktree| worktree.path == path)
                        && worktree.recent_agent_sessions != sessions
                    {
                        worktree.recent_agent_sessions = sessions;
                        changed = true;
                    }
                }
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn refresh_worktree_pull_requests(&mut self, cx: &mut Context<Self>) {
        if self.worktree_prs_loading {
            return;
        }

        let rate_limit_expired = self.clear_expired_github_rate_limit();

        let repository_slug_by_group_key: HashMap<String, String> = self
            .repositories
            .iter()
            .filter_map(|repository| {
                repository
                    .github_repo_slug
                    .as_ref()
                    .map(|slug| (repository.group_key.clone(), slug.clone()))
            })
            .collect();

        let tracked_branches: Vec<(PathBuf, String, String)> = self
            .worktrees
            .iter()
            .filter(|worktree| should_lookup_pull_request_for_worktree(worktree))
            .filter_map(|worktree| {
                repository_slug_by_group_key
                    .get(&worktree.group_key)
                    .cloned()
                    .map(|slug| (worktree.path.clone(), worktree.branch.clone(), slug))
            })
            .collect();
        let github_token = self.github_access_token();
        let github_service = self.github_service.clone();
        let tracked_paths: HashSet<PathBuf> = tracked_branches
            .iter()
            .map(|(path, ..)| path.clone())
            .collect();
        let rate_limit_remaining = self.github_rate_limit_remaining();

        let mut changed = rate_limit_expired;
        for worktree in &mut self.worktrees {
            let next_pr_loading =
                rate_limit_remaining.is_none() && tracked_paths.contains(&worktree.path);

            if worktree.pr_loading != next_pr_loading {
                worktree.pr_loading = next_pr_loading;
                changed = true;
            }
        }
        let cleared_untracked =
            clear_pull_request_data_for_untracked_worktrees(&mut self.worktrees, &tracked_paths);
        if cleared_untracked {
            changed = true;
        }

        let next_prs_loading = rate_limit_remaining.is_none() && !tracked_branches.is_empty();
        if self.worktree_prs_loading != next_prs_loading {
            self.worktree_prs_loading = next_prs_loading;
            changed = true;
        }

        if let Some(remaining) = rate_limit_remaining {
            if changed {
                self.sync_pull_request_cache_store(cx);
                cx.notify();
            }
            tracing::info!(
                remaining_seconds = remaining.as_secs(),
                tracked_worktrees = tracked_branches.len(),
                "skipping GitHub PR refresh because GitHub is rate limited"
            );
            return;
        }

        if tracked_branches.is_empty() {
            if changed {
                self.sync_pull_request_cache_store(cx);
                cx.notify();
            }
            return;
        }

        if changed {
            self.sync_pull_request_cache_store(cx);
            cx.notify();
        }

        tracing::info!(
            tracked_worktrees = tracked_branches.len(),
            refresh_interval_seconds = GITHUB_PR_REFRESH_INTERVAL.as_secs(),
            "refreshing GitHub PR details"
        );

        self.ensure_loading_animation(cx);

        cx.spawn(async move |this, cx| {
            let worker_count = tracked_branches.len().min(GITHUB_PR_REFRESH_CONCURRENCY);
            let (work_tx, work_rx) = smol::channel::unbounded::<(PathBuf, String, String)>();
            let (result_tx, result_rx) = smol::channel::unbounded::<(
                PathBuf,
                String,
                Option<u64>,
                Option<String>,
                Option<github_service::PrDetails>,
                Option<SystemTime>,
            )>();
            let stop_due_to_rate_limit = Arc::new(AtomicBool::new(false));

            for work_item in tracked_branches {
                if work_tx.send(work_item).await.is_err() {
                    break;
                }
            }
            drop(work_tx);

            for worker_index in 0..worker_count {
                let work_rx = work_rx.clone();
                let result_tx = result_tx.clone();
                let github_service = github_service.clone();
                let github_token = github_token.clone();
                let stop_due_to_rate_limit = stop_due_to_rate_limit.clone();

                cx.background_spawn(async move {
                    if let Some(delay) =
                        GITHUB_PR_REFRESH_WORKER_STAGGER.checked_mul(worker_index as u32)
                        && !delay.is_zero()
                    {
                        smol::Timer::after(delay).await;
                    }

                    while !stop_due_to_rate_limit.load(Ordering::Relaxed) {
                        let Ok((path, branch, repo_slug)) = work_rx.recv().await else {
                            break;
                        };
                        if stop_due_to_rate_limit.load(Ordering::Relaxed) {
                            break;
                        }

                        let lookup_branch = branch.clone();
                        let result = Self::lookup_worktree_pull_request(
                            github_service.as_ref(),
                            github_token.as_deref(),
                            path,
                            lookup_branch,
                            Some(repo_slug),
                        );
                        if result.4.is_some() {
                            stop_due_to_rate_limit.store(true, Ordering::Relaxed);
                        }

                        if result_tx
                            .send((result.0, branch, result.1, result.2, result.3, result.4))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            drop(result_tx);

            while let Ok((
                path_for_update,
                branch_for_update,
                next_num,
                next_url,
                next_details,
                rate_limited_until,
            )) = result_rx.recv().await
            {
                let _ = this.update(cx, |this, cx| {
                    let Some(worktree) = this
                        .worktrees
                        .iter_mut()
                        .find(|worktree| worktree.path == path_for_update)
                    else {
                        return;
                    };
                    if worktree.branch != branch_for_update {
                        return;
                    }

                    let preserve_cached_pr_data = should_preserve_cached_pr_data_on_rate_limit(
                        next_num,
                        next_url.as_deref(),
                        next_details.as_ref(),
                        rate_limited_until,
                    );
                    let mut changed = false;

                    if worktree.pr_loading {
                        worktree.pr_loading = false;
                        changed = true;
                    }
                    if !preserve_cached_pr_data && !worktree.pr_loaded {
                        worktree.pr_loaded = true;
                        changed = true;
                    }
                    if !preserve_cached_pr_data
                        && (worktree.pr_number != next_num
                            || worktree.pr_url != next_url
                            || worktree.pr_details != next_details)
                    {
                        worktree.pr_number = next_num;
                        worktree.pr_url = next_url;
                        worktree.pr_details = next_details;
                        changed = true;
                    }

                    if this.extend_github_rate_limit(rate_limited_until) {
                        changed = true;
                    }

                    let still_loading = this.worktrees.iter().any(|worktree| worktree.pr_loading);
                    if this.worktree_prs_loading != still_loading {
                        this.worktree_prs_loading = still_loading;
                        changed = true;
                    }

                    if changed {
                        this.sync_pull_request_cache_store(cx);
                        cx.notify();
                    }
                });
            }

            let _ = this.update(cx, |this, cx| {
                let mut changed = false;
                for worktree in &mut this.worktrees {
                    if worktree.pr_loading {
                        worktree.pr_loading = false;
                        changed = true;
                    }
                }
                if this.worktree_prs_loading {
                    this.worktree_prs_loading = false;
                    changed = true;
                }
                if changed {
                    this.sync_pull_request_cache_store(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn lookup_worktree_pull_request(
        github_service: &dyn github_service::GitHubService,
        github_token: Option<&str>,
        path: PathBuf,
        branch: String,
        repo_slug: Option<String>,
    ) -> (
        PathBuf,
        Option<u64>,
        Option<String>,
        Option<github_service::PrDetails>,
        Option<SystemTime>,
    ) {
        let (details, rate_limited_until) = repo_slug
            .as_ref()
            .map(|slug| github_service::pull_request_details(slug, &branch, github_token))
            .map(|outcome| (outcome.details, outcome.rate_limited_until))
            .unwrap_or((None, None));

        let (pr_number, pr_url) = if let Some(ref details) = details {
            (Some(details.number), Some(details.url.clone()))
        } else if rate_limited_until.is_some() {
            (None, None)
        } else {
            let pr_number = repo_slug.as_ref().and_then(|_| {
                github_pr_number_for_worktree(github_service, &path, &branch, github_token)
            });
            let pr_url = pr_number
                .and_then(|number| repo_slug.as_ref().map(|slug| github_pr_url(slug, number)));
            (pr_number, pr_url)
        };

        (path, pr_number, pr_url, details, rate_limited_until)
    }

    fn github_rate_limit_remaining(&self) -> Option<Duration> {
        self.github_rate_limited_until?
            .duration_since(SystemTime::now())
            .ok()
            .filter(|remaining| !remaining.is_zero())
    }

    fn clear_expired_github_rate_limit(&mut self) -> bool {
        if self.github_rate_limited_until.is_some() && self.github_rate_limit_remaining().is_none()
        {
            self.github_rate_limited_until = None;
            return true;
        }
        false
    }

    fn extend_github_rate_limit(&mut self, rate_limited_until: Option<SystemTime>) -> bool {
        let Some(rate_limited_until) = rate_limited_until else {
            return false;
        };
        if rate_limited_until <= SystemTime::now() {
            return false;
        }

        let next = match self.github_rate_limited_until {
            Some(current) if current >= rate_limited_until => current,
            _ => rate_limited_until,
        };
        if self.github_rate_limited_until == Some(next) {
            return false;
        }
        self.github_rate_limited_until = Some(next);
        true
    }

    fn switch_theme(&mut self, theme_kind: ThemeKind, cx: &mut Context<Self>) {
        if self.theme_kind == theme_kind {
            return;
        }

        self.theme_kind = theme_kind;
        self.theme_picker_selected_index = theme_picker_index_for_kind(theme_kind);
        self.config_last_modified = None;
        let store = self.app_config_store.clone();
        let theme_slug = theme_kind.slug();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    store.save_scalar_settings(&[("theme", Some(theme_slug))])
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.notice = Some(format!("failed to save theme setting: {error}"));
                    cx.notify();
                }
            });
        })
        .detach();
        if !self.show_theme_picker {
            self.theme_toast = Some(format!("Theme switched to {}", theme_kind.label()));
        }
        self.theme_toast_generation = self.theme_toast_generation.saturating_add(1);
        let generation = self.theme_toast_generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                std::thread::sleep(THEME_TOAST_DURATION);
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.theme_toast_generation == generation {
                    this.theme_toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn launch_repo_preset(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preset) = self.repo_presets.get(index) else {
            return;
        };
        let command = preset.command.trim().to_owned();
        let name = preset.name.clone();
        if command.is_empty() {
            self.notice = Some(format!("{name} preset command is empty"));
            cx.notify();
            return;
        }

        let terminal_count_before = self.terminals.len();
        self.spawn_terminal_session(window, cx);
        if self.terminals.len() <= terminal_count_before {
            return;
        }

        let Some(session_id) = self.terminals.last().map(|session| session.id) else {
            return;
        };

        let input = format!("{command}\n");
        if let Err(error) = self.write_input_to_terminal(session_id, input.as_bytes()) {
            self.notice = Some(format!("failed to run {name} preset: {error}"));
            cx.notify();
            return;
        }

        if let Some(session) = self
            .terminals
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.last_command = Some(command);
            session.pending_command.clear();
            session.updated_at_unix_ms = current_unix_timestamp_millis();
        }

        self.sync_daemon_session_store(cx);
        cx.notify();
    }

    fn handle_global_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.is_held {
            return;
        }

        if self.welcome_clone_url_active {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.welcome_clone_url_active = false;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                },
                "enter" | "return" => {
                    self.submit_welcome_clone(cx);
                    cx.stop_propagation();
                    return;
                },
                _ => {},
            }
            if let Some(action) = text_edit_action_for_event(event, cx) {
                apply_text_edit_action(
                    &mut self.welcome_clone_url,
                    &mut self.welcome_clone_url_cursor,
                    &action,
                );
                self.welcome_clone_error = None;
                cx.notify();
                cx.stop_propagation();
            }
            return;
        }

        if self.worktree_notes_active && self.right_pane_tab == RightPaneTab::Notes {
            if self.handle_worktree_notes_key_down(event, cx) {
                cx.stop_propagation();
            }
            return;
        }

        if self.right_pane_search_active {
            if event.keystroke.key.as_str() == "escape" {
                self.right_pane_search.clear();
                self.right_pane_search_cursor = 0;
                self.right_pane_search_active = false;
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if let Some(action) = text_edit_action_for_event(event, cx) {
                apply_text_edit_action(
                    &mut self.right_pane_search,
                    &mut self.right_pane_search_cursor,
                    &action,
                );
                cx.notify();
                cx.stop_propagation();
            }
            return;
        }

        if self.quit_overlay_until.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.quit_overlay_until = None;
                    cx.notify();
                    cx.stop_propagation();
                },
                "enter" | "return" => {
                    self.action_confirm_quit(window, cx);
                    cx.stop_propagation();
                },
                _ => {},
            }
            return;
        }

        if self.command_palette_modal.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_command_palette(cx);
                    cx.stop_propagation();
                    return;
                },
                "enter" | "return" => {
                    self.execute_command_palette_selection(window, cx);
                    cx.stop_propagation();
                    return;
                },
                "up" => {
                    self.move_command_palette_selection(-1, cx);
                    cx.stop_propagation();
                    return;
                },
                "down" => {
                    self.move_command_palette_selection(1, cx);
                    cx.stop_propagation();
                    return;
                },
                _ => {},
            }

            if let Some(action) = text_edit_action_for_event(event, cx) {
                if let Some(modal) = self.command_palette_modal.as_mut() {
                    apply_text_edit_action(&mut modal.query, &mut modal.query_cursor, &action);
                    modal.selected_index = 0;
                }
                self.command_palette_scroll_handle.scroll_to_item(0);
                cx.notify();
                cx.stop_propagation();
            }
            return;
        }

        if self.show_theme_picker {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.show_theme_picker = false;
                    cx.stop_propagation();
                    cx.notify();
                },
                "left" => {
                    self.move_theme_picker_selection(-1, cx);
                    cx.stop_propagation();
                },
                "right" => {
                    self.move_theme_picker_selection(1, cx);
                    cx.stop_propagation();
                },
                "up" => {
                    self.move_theme_picker_selection(
                        -(theme_picker_columns(ThemeKind::ALL.len()) as isize),
                        cx,
                    );
                    cx.stop_propagation();
                },
                "down" => {
                    self.move_theme_picker_selection(
                        theme_picker_columns(ThemeKind::ALL.len()) as isize,
                        cx,
                    );
                    cx.stop_propagation();
                },
                "enter" | "return" => {
                    self.apply_selected_theme_picker_theme(cx);
                    cx.stop_propagation();
                },
                _ => {},
            }
            return;
        }

        if self.settings_modal.is_some() {
            let active_control = self
                .settings_modal
                .as_ref()
                .map(|modal| modal.active_control)
                .unwrap_or(SettingsControl::DaemonBindMode);
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_settings_modal(cx);
                    cx.stop_propagation();
                    return;
                },
                "tab" => {
                    self.update_settings_modal_input(
                        SettingsModalInputEvent::CycleControl(event.keystroke.modifiers.shift),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                },
                "left" if active_control == SettingsControl::DaemonBindMode => {
                    self.update_settings_modal_input(
                        SettingsModalInputEvent::SelectDaemonBindMode(DaemonBindMode::Localhost),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                },
                "right" if active_control == SettingsControl::DaemonBindMode => {
                    self.update_settings_modal_input(
                        SettingsModalInputEvent::SelectDaemonBindMode(
                            DaemonBindMode::AllInterfaces,
                        ),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                },
                "space" => {
                    self.update_settings_modal_input(
                        SettingsModalInputEvent::ToggleActiveControl,
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                },
                "enter" | "return" => {
                    self.submit_settings_modal(cx);
                    cx.stop_propagation();
                    return;
                },
                _ => {},
            }
            return;
        }

        if self.github_auth_modal.is_some() {
            if event.keystroke.modifiers.platform {
                return;
            }

            if event.keystroke.key.as_str() == "escape" {
                self.close_github_auth_modal(cx);
                cx.stop_propagation();
            }
            return;
        }

        if self.delete_modal.is_some() {
            if event.keystroke.modifiers.platform {
                return;
            }
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_delete_modal(cx);
                    cx.stop_propagation();
                },
                "enter" | "return" => {
                    self.execute_delete(cx);
                    cx.stop_propagation();
                },
                "space" | " " => {
                    if let Some(modal) = self.delete_modal.as_mut()
                        && matches!(modal.target, DeleteTarget::Worktree(_))
                    {
                        modal.delete_branch = !modal.delete_branch;
                        cx.notify();
                    }
                    cx.stop_propagation();
                },
                _ => {},
            }
            return;
        }

        if self.start_daemon_modal {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.start_daemon_modal = false;
                    cx.notify();
                    cx.stop_propagation();
                },
                "enter" | "return" => {
                    self.start_daemon_modal = false;
                    self.try_start_and_connect_daemon(cx);
                    cx.stop_propagation();
                },
                _ => {},
            }
            return;
        }

        if self.daemon_auth_modal.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.daemon_auth_modal = None;
                    cx.notify();
                    cx.stop_propagation();
                },
                "enter" | "return" => {
                    self.submit_daemon_auth(cx);
                    cx.stop_propagation();
                },
                _ => {
                    if let Some(modal) = self.daemon_auth_modal.as_mut()
                        && let Some(action) = text_edit_action_for_event(event, cx)
                    {
                        apply_text_edit_action(&mut modal.token, &mut modal.token_cursor, &action);
                        modal.error = None;
                        cx.notify();
                        cx.stop_propagation();
                    }
                },
            }
            return;
        }

        if self.commit_modal.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_commit_modal(cx);
                    cx.stop_propagation();
                    return;
                },
                "enter" | "return" => {
                    self.submit_commit_modal(cx);
                    cx.stop_propagation();
                    return;
                },
                _ => {},
            }

            if let Some(action) = text_edit_action_for_event(event, cx) {
                if let Some(modal) = self.commit_modal.as_mut() {
                    apply_text_edit_action(&mut modal.message, &mut modal.message_cursor, &action);
                    modal.error = None;
                }
                cx.notify();
                cx.stop_propagation();
            }
            return;
        }

        if self.connect_to_host_modal.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.connect_to_host_modal = None;
                    cx.notify();
                    cx.stop_propagation();
                },
                "enter" | "return" => {
                    self.submit_connect_to_host(cx);
                    cx.stop_propagation();
                },
                _ => {
                    if let Some(modal) = self.connect_to_host_modal.as_mut()
                        && let Some(action) = text_edit_action_for_event(event, cx)
                    {
                        apply_text_edit_action(
                            &mut modal.address,
                            &mut modal.address_cursor,
                            &action,
                        );
                        modal.error = None;
                        cx.notify();
                        cx.stop_propagation();
                    }
                },
            }
            return;
        }

        if self.manage_hosts_modal.is_some() {
            if event.keystroke.modifiers.platform {
                return;
            }

            let adding = self
                .manage_hosts_modal
                .as_ref()
                .map(|m| m.adding)
                .unwrap_or(false);

            if adding {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        if let Some(modal) = self.manage_hosts_modal.as_mut() {
                            modal.adding = false;
                            modal.error = None;
                            cx.notify();
                        }
                        cx.stop_propagation();
                        return;
                    },
                    "tab" => {
                        self.update_manage_hosts_modal_input(
                            HostsModalInputEvent::MoveActiveField(event.keystroke.modifiers.shift),
                            cx,
                        );
                        cx.stop_propagation();
                        return;
                    },
                    "enter" | "return" => {
                        self.submit_add_host(cx);
                        cx.stop_propagation();
                        return;
                    },
                    _ => {},
                }

                if let Some(action) = text_edit_action_for_event(event, cx) {
                    self.update_manage_hosts_modal_input(HostsModalInputEvent::ClearError, cx);
                    self.update_manage_hosts_modal_input(HostsModalInputEvent::Edit(action), cx);
                    cx.stop_propagation();
                }
            } else if event.keystroke.key.as_str() == "escape" {
                self.close_manage_hosts_modal(cx);
                cx.stop_propagation();
            }
            return;
        }

        if self.manage_presets_modal.is_some() {
            if event.keystroke.modifiers.platform {
                return;
            }

            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_manage_presets_modal(cx);
                    cx.stop_propagation();
                    return;
                },
                "tab" => {
                    self.update_manage_presets_modal_input(
                        PresetsModalInputEvent::CycleActivePreset(event.keystroke.modifiers.shift),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                },
                "enter" | "return" => {
                    self.submit_manage_presets_modal(cx);
                    cx.stop_propagation();
                    return;
                },
                _ => {},
            }
            if let Some(action) = text_edit_action_for_event(event, cx) {
                self.update_manage_presets_modal_input(PresetsModalInputEvent::ClearError, cx);
                self.update_manage_presets_modal_input(PresetsModalInputEvent::Edit(action), cx);
                cx.stop_propagation();
            }
            return;
        }

        if self.manage_repo_presets_modal.is_some() {
            let active_tab = self
                .manage_repo_presets_modal
                .as_ref()
                .map(|modal| modal.active_tab)
                .unwrap_or(RepoPresetModalTab::Edit);

            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_manage_repo_presets_modal(cx);
                    cx.stop_propagation();
                    return;
                },
                "tab" => {
                    if active_tab == RepoPresetModalTab::Edit {
                        self.update_manage_repo_presets_modal_input(
                            RepoPresetsModalInputEvent::MoveActiveField(
                                event.keystroke.modifiers.shift,
                            ),
                            cx,
                        );
                    }
                    cx.stop_propagation();
                    return;
                },
                "enter" | "return" => {
                    if active_tab == RepoPresetModalTab::Edit {
                        self.submit_manage_repo_presets_modal(cx);
                    }
                    cx.stop_propagation();
                    return;
                },
                _ => {},
            }
            if active_tab == RepoPresetModalTab::Edit
                && let Some(action) = text_edit_action_for_event(event, cx)
            {
                self.update_manage_repo_presets_modal_input(
                    RepoPresetsModalInputEvent::ClearError,
                    cx,
                );
                self.update_manage_repo_presets_modal_input(
                    RepoPresetsModalInputEvent::Edit(action),
                    cx,
                );
                cx.stop_propagation();
            }
            return;
        }

        if self.issue_details_modal.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.close_issue_details_modal(Some(window), cx);
                    cx.stop_propagation();
                },
                "enter" | "return" => {
                    self.open_create_modal_from_issue_details(cx);
                    cx.stop_propagation();
                },
                _ => {},
            }
            return;
        }

        let Some(modal) = self.create_modal.as_ref() else {
            return;
        };

        let active_tab = modal.tab;

        match event.keystroke.key.as_str() {
            "escape" => {
                if self
                    .create_modal
                    .as_ref()
                    .is_some_and(|m| m.host_dropdown_open)
                {
                    if let Some(modal) = self.create_modal.as_mut() {
                        modal.host_dropdown_open = false;
                    }
                    cx.notify();
                } else {
                    self.close_create_modal(cx);
                }
                cx.stop_propagation();
                return;
            },
            "tab" => {
                match active_tab {
                    CreateModalTab::LocalWorktree => {
                        self.update_create_worktree_modal_input(
                            ModalInputEvent::MoveActiveField,
                            cx,
                        );
                    },
                    CreateModalTab::ReviewPullRequest => {
                        self.update_create_review_pr_modal_input(
                            ReviewPrModalInputEvent::MoveActiveField,
                            cx,
                        );
                    },
                    CreateModalTab::RemoteOutpost => {
                        self.update_create_outpost_modal_input(
                            OutpostModalInputEvent::MoveActiveField(
                                event.keystroke.modifiers.shift,
                            ),
                            cx,
                        );
                    },
                }
                cx.stop_propagation();
                return;
            },
            "enter" | "return" => {
                if active_tab == CreateModalTab::RemoteOutpost
                    && self
                        .create_modal
                        .as_ref()
                        .is_some_and(|m| m.outpost_active_field == CreateOutpostField::HostSelector)
                {
                    self.update_create_outpost_modal_input(
                        OutpostModalInputEvent::ToggleHostDropdown,
                        cx,
                    );
                } else {
                    match active_tab {
                        CreateModalTab::LocalWorktree => self.submit_create_worktree_modal(cx),
                        CreateModalTab::ReviewPullRequest => self.submit_create_review_pr_modal(cx),
                        CreateModalTab::RemoteOutpost => self.submit_create_outpost_modal(cx),
                    }
                }
                cx.stop_propagation();
                return;
            },
            "left" | "right" => {
                if active_tab == CreateModalTab::RemoteOutpost
                    && self
                        .create_modal
                        .as_ref()
                        .map(|m| m.outpost_active_field == CreateOutpostField::HostSelector)
                        .unwrap_or(false)
                {
                    let reverse = event.keystroke.key.as_str() == "left";
                    self.update_create_outpost_modal_input(
                        OutpostModalInputEvent::CycleHost(reverse),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                }
            },
            _ => {},
        }
        if let Some(action) = text_edit_action_for_event(event, cx) {
            match active_tab {
                CreateModalTab::LocalWorktree => {
                    self.update_create_worktree_modal_input(ModalInputEvent::ClearError, cx);
                    self.update_create_worktree_modal_input(ModalInputEvent::Edit(action), cx);
                },
                CreateModalTab::ReviewPullRequest => {
                    self.update_create_review_pr_modal_input(
                        ReviewPrModalInputEvent::ClearError,
                        cx,
                    );
                    self.update_create_review_pr_modal_input(
                        ReviewPrModalInputEvent::Edit(action),
                        cx,
                    );
                },
                CreateModalTab::RemoteOutpost => {
                    self.update_create_outpost_modal_input(OutpostModalInputEvent::ClearError, cx);
                    self.update_create_outpost_modal_input(
                        OutpostModalInputEvent::Edit(action),
                        cx,
                    );
                },
            }
            cx.stop_propagation();
        }
    }

    fn action_open_create_worktree(
        &mut self,
        _: &OpenCreateWorktree,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_index = self.active_repository_index.unwrap_or(0);
        self.open_create_modal(repo_index, CreateModalTab::LocalWorktree, cx);
    }

    fn action_open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_command_palette(cx);
    }

    fn action_open_add_repository(
        &mut self,
        _: &OpenAddRepository,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_add_repository_picker(cx);
    }

    fn action_spawn_terminal(
        &mut self,
        _: &SpawnTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_terminal_session(window, cx);
    }

    fn action_close_active_terminal(
        &mut self,
        _: &CloseActiveTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_active_tab(window, cx);
    }

    fn action_open_manage_presets(
        &mut self,
        _: &OpenManagePresets,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_manage_presets_modal(cx);
    }

    fn action_open_manage_repo_presets(
        &mut self,
        _: &OpenManageRepoPresets,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_manage_repo_presets_modal(None, cx);
    }

    fn action_refresh_worktrees(
        &mut self,
        _: &RefreshWorktrees,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_worktrees(cx);
        cx.notify();
    }

    fn action_refresh_changes(
        &mut self,
        _: &RefreshChanges,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_changed_files(cx);
        cx.notify();
    }

    fn action_toggle_left_pane(
        &mut self,
        _: &ToggleLeftPane,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.left_pane_visible = !self.left_pane_visible;
        cx.notify();
    }

    fn action_navigate_worktree_back(
        &mut self,
        _: &NavigateWorktreeBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.worktree_nav_back.pop() {
            if let Some(current) = self.active_worktree_index {
                self.worktree_nav_forward.push(current);
            }
            self.active_worktree_index = Some(target);
            self.active_diff_session_id = None;
            self.sync_active_repository_from_selected_worktree();
            self.refresh_changed_files(cx);
            if self.ensure_selected_worktree_terminal(cx) {
                self.sync_daemon_session_store(cx);
            }
            self.sync_navigation_ui_state_store(cx);
            self.terminal_scroll_handle.scroll_to_bottom();
            window.focus(&self.terminal_focus);
            self.focus_terminal_on_next_render = false;
            cx.notify();
        }
    }

    fn action_navigate_worktree_forward(
        &mut self,
        _: &NavigateWorktreeForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.worktree_nav_forward.pop() {
            if let Some(current) = self.active_worktree_index {
                self.worktree_nav_back.push(current);
            }
            self.active_worktree_index = Some(target);
            self.active_diff_session_id = None;
            self.sync_active_repository_from_selected_worktree();
            self.refresh_changed_files(cx);
            if self.ensure_selected_worktree_terminal(cx) {
                self.sync_daemon_session_store(cx);
            }
            self.sync_navigation_ui_state_store(cx);
            self.terminal_scroll_handle.scroll_to_bottom();
            window.focus(&self.terminal_focus);
            self.focus_terminal_on_next_render = false;
            cx.notify();
        }
    }

    fn action_collapse_all_repositories(
        &mut self,
        _: &CollapseAllRepositories,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let all_collapsed =
            (0..self.repositories.len()).all(|i| self.collapsed_repositories.contains(&i));
        if all_collapsed {
            self.collapsed_repositories.clear();
        } else {
            self.collapsed_repositories = (0..self.repositories.len()).collect();
        }
        self.sync_collapsed_repositories_store(cx);
        cx.notify();
    }

    fn action_request_quit(&mut self, _: &RequestQuit, _: &mut Window, cx: &mut Context<Self>) {
        self.quit_overlay_until = if self.quit_overlay_until.is_some() {
            self.quit_after_persistence_flush = false;
            None
        } else {
            Some(Instant::now())
        };
        cx.notify();
    }

    fn action_confirm_quit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit_after_persistence_flush(cx);
    }

    fn action_dismiss_quit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.quit_overlay_until = None;
        self.quit_after_persistence_flush = false;
        cx.notify();
    }

    fn action_immediate_quit(&mut self, _: &ImmediateQuit, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit_after_persistence_flush(cx);
    }

    fn action_view_logs(&mut self, _: &ViewLogs, _: &mut Window, cx: &mut Context<Self>) {
        self.logs_tab_open = true;
        self.logs_tab_active = true;
        self.active_diff_session_id = None;
        self.sync_navigation_ui_state_store(cx);
        cx.notify();
    }

    fn action_show_about(&mut self, _: &ShowAbout, _: &mut Window, cx: &mut Context<Self>) {
        self.show_about = true;
        cx.notify();
    }

    fn action_open_theme_picker(
        &mut self,
        _: &OpenThemePicker,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_theme_picker_modal(cx);
    }

    fn action_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_modal(cx);
    }

    fn action_open_manage_hosts(
        &mut self,
        _: &OpenManageHosts,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_manage_hosts_modal(cx);
    }

    fn action_connect_to_lan_daemon(
        &mut self,
        action: &ConnectToLanDaemon,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_discovered_daemon(action.index, cx);
    }

    fn action_connect_to_host(
        &mut self,
        _: &ConnectToHost,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connect_to_host_modal = Some(ConnectToHostModal {
            address: String::new(),
            address_cursor: 0,
            error: None,
        });
        cx.notify();
    }

    fn spawn_terminal_session_inner(
        &mut self,
        show_notice_on_missing_worktree: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cwd) = self.selected_worktree_path().map(Path::to_path_buf) else {
            if show_notice_on_missing_worktree {
                self.notice = Some("select a worktree before opening a terminal tab".to_owned());
            }
            return false;
        };

        tracing::info!(cwd = %cwd.display(), "spawning terminal session");
        let backend_kind = self.active_backend_kind;
        let session_id = self.next_terminal_id;
        self.next_terminal_id += 1;
        self.active_terminal_by_worktree
            .insert(cwd.clone(), session_id);
        let title = format!("term-{session_id}");
        self.terminals.push(TerminalSession {
            id: session_id,
            daemon_session_id: session_id.to_string(),
            worktree_path: cwd.clone(),
            managed_process_id: None,
            title: title.clone(),
            last_command: None,
            pending_command: String::new(),
            command: String::new(),
            agent_preset: None,
            execution_mode: None,
            state: TerminalState::Running,
            exit_code: None,
            updated_at_unix_ms: current_unix_timestamp_millis(),
            root_pid: None,
            cols: 120,
            rows: 35,
            generation: 0,
            output: String::new(),
            styled_output: Vec::new(),
            cursor: None,
            modes: TerminalModes::default(),
            last_runtime_sync_at: None,
            queued_input: Vec::new(),
            is_initializing: true,
            runtime: None,
        });

        let daemon = self.terminal_daemon.clone();
        let shell = self.embedded_shell();
        let target_grid_size = self.last_terminal_grid_size.unwrap_or((0, 0));
        let poll_tx = self.terminal_poll_tx.clone();
        cx.spawn(async move |this, cx| {
            enum SpawnTerminalOutcome {
                Daemon {
                    daemon: terminal_daemon_http::SharedTerminalDaemonClient,
                    record: DaemonSessionRecord,
                    notice: Option<String>,
                    clear_global_daemon: bool,
                },
                Embedded {
                    runtime: EmbeddedTerminal,
                    notice: Option<String>,
                    clear_global_daemon: bool,
                },
                Failed {
                    error: String,
                    notice: Option<String>,
                    clear_global_daemon: bool,
                },
            }

            let outcome = cx
                .background_spawn(async move {
                    let mut fallback_notice = None;
                    let mut clear_global_daemon = false;

                    if let Some(daemon) = daemon {
                        match daemon.create_or_attach(CreateOrAttachRequest {
                            session_id: String::new().into(),
                            workspace_id: cwd.display().to_string().into(),
                            cwd: cwd.clone(),
                            shell,
                            cols: 120,
                            rows: 35,
                            title: Some(title),
                            command: None,
                        }) {
                            Ok(response) => {
                                return SpawnTerminalOutcome::Daemon {
                                    daemon,
                                    record: response.session,
                                    notice: None,
                                    clear_global_daemon: false,
                                };
                            },
                            Err(error) => {
                                let error_text = error.to_string();
                                tracing::warn!(
                                    %error,
                                    "failed to create daemon terminal session, falling back to local"
                                );
                                clear_global_daemon =
                                    daemon_error_is_connection_refused(&error_text);
                                if !clear_global_daemon {
                                    fallback_notice = Some(format!(
                                        "failed to create daemon terminal session (falling back to local embedded terminal): {error}"
                                    ));
                                }
                            },
                        }
                    }

                    match terminal_backend::launch_backend(
                        backend_kind,
                        &cwd,
                        target_grid_size.0,
                        target_grid_size.1,
                    ) {
                        Ok(TerminalLaunch::Embedded(runtime)) => SpawnTerminalOutcome::Embedded {
                            runtime,
                            notice: fallback_notice,
                            clear_global_daemon,
                        },
                        Err(error) => SpawnTerminalOutcome::Failed {
                            error: error.to_string(),
                            notice: fallback_notice,
                            clear_global_daemon,
                        },
                    }
                })
                .await;

            let orphaned_daemon_session = match &outcome {
                SpawnTerminalOutcome::Daemon { daemon, record, .. } => {
                    Some((daemon.clone(), record.clone()))
                },
                _ => None,
            };
            let orphaned_daemon_session_for_update = orphaned_daemon_session.clone();

            let updated = this.update(cx, |this, cx| {
                let Some(session) = this
                    .terminals
                    .iter_mut()
                    .find(|session| session.id == session_id)
                else {
                    if let Some((daemon, record)) = orphaned_daemon_session_for_update {
                        schedule_orphaned_daemon_session_cleanup(cx, daemon, record);
                    }
                    return;
                };

                match outcome {
                    SpawnTerminalOutcome::Daemon {
                        daemon,
                        record,
                        notice,
                        clear_global_daemon,
                    } => {
                        if clear_global_daemon {
                            this.terminal_daemon = None;
                        }
                        if let Some(notice) = notice {
                            this.notice = Some(notice);
                        }
                        session.daemon_session_id = record.session_id.to_string();
                        session.title = record
                            .title
                            .clone()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_else(|| session.title.clone());
                        session.last_command = record.last_command.clone();
                        session.command = record.shell.clone();
                        session.output = record.output_tail.clone().unwrap_or_default();
                        session.state = terminal_state_from_daemon_record(&record);
                        session.exit_code = record.exit_code;
                        session.updated_at_unix_ms = record.updated_at_unix_ms;
                        session.root_pid = record.root_pid;
                        session.cols = record.cols.max(2);
                        session.rows = record.rows.max(1);
                        session.runtime = Some(local_daemon_runtime(
                            daemon,
                            record.session_id.to_string(),
                            session.rows,
                            session.cols,
                            Some(poll_tx.clone()),
                        ));
                    },
                    SpawnTerminalOutcome::Embedded {
                        runtime,
                        notice,
                        clear_global_daemon,
                    } => {
                        if clear_global_daemon {
                            this.terminal_daemon = None;
                        }
                        if let Some(notice) = notice {
                            this.notice = Some(notice);
                        }
                        session.root_pid = runtime.root_pid();
                        runtime.set_notify(poll_tx.clone());
                        session.command = "embedded shell".to_owned();
                        session.generation = runtime.generation();
                        session.runtime = Some(local_embedded_runtime(runtime));
                        session.output.clear();
                        session.styled_output.clear();
                        session.cursor = None;
                        session.exit_code = None;
                        session.updated_at_unix_ms = current_unix_timestamp_millis();
                    },
                    SpawnTerminalOutcome::Failed {
                        error,
                        notice,
                        clear_global_daemon,
                    } => {
                        if clear_global_daemon {
                            this.terminal_daemon = None;
                        }
                        if let Some(notice) = notice {
                            this.notice = Some(notice);
                        }
                        session.command = "launch backend".to_owned();
                        session.output = error.clone();
                        session.styled_output.clear();
                        session.cursor = None;
                        session.state = TerminalState::Failed;
                        session.updated_at_unix_ms = current_unix_timestamp_millis();
                        this.notice = Some(format!("terminal session failed: {error}"));
                    },
                }

                session.is_initializing = false;
                if let Err(error) = this.flush_queued_input_for_terminal(session_id) {
                    this.notice = Some(format!("failed to write queued terminal input: {error}"));
                }
                this.sync_daemon_session_store(cx);
                cx.notify();
            });

            if updated.is_err()
                && let Some((daemon, record)) = orphaned_daemon_session
            {
                schedule_orphaned_daemon_session_cleanup(cx, daemon, record);
            }
        })
        .detach();
        true
    }

    fn open_editor_in_terminal(&mut self, editor: &str, file_path: &Path, cx: &mut Context<Self>) {
        if !self.spawn_terminal_session_inner(true, cx) {
            cx.notify();
            return;
        }

        // Find the session we just spawned, set its title, and send the editor command
        let session_id = self.next_terminal_id - 1;
        let editor_basename = Path::new(editor)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(editor);
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        if let Some(session) = self.terminals.iter_mut().find(|s| s.id == session_id) {
            session.title = format!("{editor_basename}: {file_name}");
        }
        let cmd = format!(
            "{} {}; exit\n",
            shell_escape(editor),
            shell_escape(&file_path.to_string_lossy()),
        );
        if let Err(error) = self.write_input_to_terminal(session_id, cmd.as_bytes()) {
            self.notice = Some(format!("Failed to send command to terminal: {error}"));
        }

        self.sync_daemon_session_store(cx);
        self.active_diff_session_id = None;
        self.active_file_view_session_id = None;
        self.file_view_editing = false;
        self.logs_tab_active = false;
        self.terminal_scroll_handle.scroll_to_bottom();
        self.focus_terminal_on_next_render = true;
    }

    fn spawn_terminal_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(outpost_index) = self.active_outpost_index {
            self.spawn_outpost_terminal(outpost_index, window, cx);
            return;
        }

        if !self.spawn_terminal_session_inner(true, cx) {
            cx.notify();
            return;
        }

        self.sync_daemon_session_store(cx);
        self.active_diff_session_id = None;
        self.active_file_view_session_id = None;
        self.file_view_editing = false;
        self.logs_tab_active = false;
        self.terminal_scroll_handle.scroll_to_bottom();
        window.focus(&self.terminal_focus);
        self.focus_terminal_on_next_render = false;
        self.sync_ui_state_store(window, cx);
        cx.notify();
    }

    fn spawn_outpost_terminal(
        &mut self,
        outpost_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(outpost) = self.outposts.get(outpost_index) else {
            return;
        };

        let host = self
            .remote_hosts
            .iter()
            .find(|host| host.name == outpost.host_name)
            .cloned();
        let Some(host) = host else {
            self.notice = Some(format!(
                "no remote host config found for `{}`",
                outpost.host_name,
            ));
            cx.notify();
            return;
        };

        let worktree_path = outpost.repo_root.clone();
        let session_id = self.next_terminal_id;
        self.next_terminal_id += 1;
        self.active_terminal_by_worktree
            .insert(worktree_path.clone(), session_id);

        let title = format!("ssh-{}", outpost.label);
        let session = TerminalSession {
            id: session_id,
            daemon_session_id: session_id.to_string(),
            worktree_path,
            managed_process_id: None,
            title,
            last_command: None,
            pending_command: String::new(),
            command: String::new(),
            agent_preset: None,
            execution_mode: None,
            state: TerminalState::Running,
            exit_code: None,
            updated_at_unix_ms: current_unix_timestamp_millis(),
            root_pid: None,
            cols: 120,
            rows: 35,
            generation: 0,
            output: String::new(),
            styled_output: Vec::new(),
            cursor: None,
            modes: TerminalModes::default(),
            last_runtime_sync_at: None,
            queued_input: Vec::new(),
            is_initializing: true,
            runtime: None,
        };
        self.terminals.push(session);
        self.active_diff_session_id = None;
        self.terminal_scroll_handle.scroll_to_bottom();
        window.focus(&self.terminal_focus);
        self.focus_terminal_on_next_render = false;
        let pool = self.ssh_connection_pool.clone();
        let remote_path = outpost.remote_path.clone();
        let poll_tx = self.terminal_poll_tx.clone();
        cx.spawn(async move |this, cx| {
            enum OutpostLaunchOutcome {
                Mosh {
                    mosh: arbor_mosh::MoshShell,
                    notice: Option<String>,
                },
                Ssh {
                    ssh_shell: SshTerminalShell,
                    notice: Option<String>,
                },
            }

            let result = cx
                .background_spawn(async move {
                    let mut fallback_notice = None;
                    if host.mosh == Some(true) && arbor_mosh::detect::local_mosh_client_available()
                    {
                        let conn_slot = pool.get_or_connect(&host).map_err(|error| {
                            format!("SSH connection failed for mosh handshake: {error}")
                        })?;
                        let guard = conn_slot
                            .lock()
                            .map_err(|_| "SSH connection lock poisoned".to_owned())?;
                        let connection = guard
                            .as_ref()
                            .ok_or_else(|| "SSH connection not available".to_owned())?;
                        match arbor_mosh::handshake::start_mosh_server(connection, &host)
                            .map_err(|error| {
                                format!("mosh handshake failed, falling back to SSH: {error}")
                            })
                            .and_then(|handshake| {
                                arbor_mosh::MoshShell::spawn(handshake, 120, 35).map_err(|error| {
                                    format!("mosh-client failed, falling back to SSH: {error}")
                                })
                            }) {
                            Ok(mosh) => {
                                return Ok(OutpostLaunchOutcome::Mosh { mosh, notice: None });
                            },
                            Err(error) => fallback_notice = Some(error),
                        }
                    } else if host.mosh == Some(true) {
                        fallback_notice = Some(
                            "mosh-client not found locally, falling back to SSH shell".to_owned(),
                        );
                    }

                    let conn_slot = pool
                        .get_or_connect(&host)
                        .map_err(|error| format!("SSH connection failed: {error}"))?;
                    let guard = conn_slot
                        .lock()
                        .map_err(|_| "SSH connection lock poisoned".to_owned())?;
                    let connection = guard
                        .as_ref()
                        .ok_or_else(|| "SSH connection not available".to_owned())?;
                    let ssh_shell = SshTerminalShell::open(connection, 120, 35, &remote_path)
                        .map_err(|error| format!("SSH shell failed: {error}"))?;
                    Ok::<OutpostLaunchOutcome, String>(OutpostLaunchOutcome::Ssh {
                        ssh_shell,
                        notice: fallback_notice,
                    })
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                let Some(session) = this
                    .terminals
                    .iter_mut()
                    .find(|session| session.id == session_id)
                else {
                    return;
                };

                match result {
                    Ok(OutpostLaunchOutcome::Mosh { mosh, notice }) => {
                        if let Some(notice) = notice {
                            this.notice = Some(notice);
                        }
                        mosh.set_notify(poll_tx.clone());
                        session.command = "mosh".to_owned();
                        session.generation = mosh.generation();
                        session.runtime = Some(outpost_mosh_runtime(mosh));
                    },
                    Ok(OutpostLaunchOutcome::Ssh { ssh_shell, notice }) => {
                        if let Some(notice) = notice {
                            this.notice = Some(notice);
                        }
                        session.command = "ssh".to_owned();
                        session.generation = ssh_shell.generation();
                        session.runtime = Some(outpost_ssh_runtime(ssh_shell));
                    },
                    Err(error) => {
                        session.state = TerminalState::Failed;
                        session.output = error.clone();
                        this.notice = Some(error);
                    },
                }
                session.is_initializing = false;
                if let Err(error) = this.flush_queued_input_for_terminal(session_id) {
                    this.notice = Some(format!("failed to write queued terminal input: {error}"));
                }
                this.sync_daemon_session_store(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_terminal(&mut self, session_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(worktree_path) = self.selected_worktree_path().map(Path::to_path_buf) else {
            return;
        };

        if self.active_center_tab_for_selected_worktree() == Some(CenterTab::Terminal(session_id)) {
            window.focus(&self.terminal_focus);
            self.focus_terminal_on_next_render = false;
            return;
        }

        self.active_terminal_by_worktree
            .insert(worktree_path, session_id);
        if let Some(session) = self
            .terminals
            .iter()
            .find(|session| session.id == session_id)
        {
            if let Some(preset) = session.agent_preset {
                self.active_preset_tab = Some(preset);
            }
            if let Some(mode) = session.execution_mode {
                self.execution_mode = mode;
            }
        }
        self.active_diff_session_id = None;
        self.active_file_view_session_id = None;
        self.logs_tab_active = false;
        self.sync_navigation_ui_state_store(cx);
        self.terminal_scroll_handle.scroll_to_bottom();
        window.focus(&self.terminal_focus);
        self.focus_terminal_on_next_render = false;
        cx.notify();
    }
}

fn is_command_in_path(command: &str) -> bool {
    use std::env;
    let path_var = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path_var).any(|dir| dir.join(command).is_file())
}

/// Return the set of `AgentPresetKind` variants whose CLI is found in PATH.
/// Cached for the lifetime of the process (the set of installed tools is
/// unlikely to change while the app is running).
fn installed_preset_kinds() -> &'static HashSet<AgentPresetKind> {
    static INSTALLED: OnceLock<HashSet<AgentPresetKind>> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        AgentPresetKind::ORDER
            .iter()
            .copied()
            .filter(|kind| kind.is_installed())
            .collect()
    })
}

fn default_agent_presets() -> Vec<AgentPreset> {
    AgentPresetKind::ORDER
        .iter()
        .copied()
        .map(|kind| AgentPreset {
            kind,
            command: kind.default_command().to_owned(),
        })
        .collect()
}

fn normalize_agent_presets(configured: &[app_config::AgentPresetConfig]) -> Vec<AgentPreset> {
    let mut presets = default_agent_presets();

    for configured_preset in configured {
        let Some(kind) = AgentPresetKind::from_key(&configured_preset.key) else {
            continue;
        };
        let command = configured_preset.command.trim();
        if command.is_empty() {
            continue;
        }
        if let Some(preset) = presets.iter_mut().find(|preset| preset.kind == kind) {
            preset.command = command.to_owned();
        }
    }

    presets
}

impl Drop for ArborWindow {
    fn drop(&mut self) {
        self.stop_active_ssh_daemon_tunnel();
        remove_claude_code_hooks();
        remove_pi_agent_extension();
    }
}

impl WorktreeSummary {
    fn from_worktree(
        entry: &worktree::Worktree,
        repo_root: &Path,
        group_key: &str,
        checkout_kind: CheckoutKind,
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

        let last_activity_unix_ms = worktree::last_git_activity_ms(&entry.path);
        let managed_processes = managed_processes_for_worktree(repo_root, &entry.path);

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
            branch_divergence: branch_divergence_summary(&entry.path),
            diff_summary: None,
            detected_ports: Vec::new(),
            managed_processes,
            recent_turns: Vec::new(),
            stuck_turn_count: 0,
            recent_agent_sessions: Vec::new(),
            agent_state: None,
            agent_task: None,
            last_activity_unix_ms,
        }
    }

    fn apply_cached_pull_request_state(&mut self, cached: &ui_state_store::CachedPullRequestState) {
        self.pr_loaded = true;
        self.pr_number = cached.number;
        self.pr_url = cached.url.clone();
        self.pr_details = cached.details.clone();
    }

    fn cached_pull_request_state(&self) -> Option<ui_state_store::CachedPullRequestState> {
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
    fn from_checkout_roots(
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

    fn contains_checkout_root(&self, root: &Path) -> bool {
        self.checkout_roots
            .iter()
            .any(|checkout_root| checkout_root.path == root)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct WorktreeInventoryRefreshResult {
    rows_changed: bool,
    created_terminal: bool,
}

impl WorktreeInventoryRefreshResult {
    fn visible_change(self) -> bool {
        self.rows_changed || self.created_terminal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorktreeInventoryRefreshMode {
    PreserveTerminalState,
    EnsureSelectedTerminal,
}

impl WorktreeInventoryRefreshMode {
    fn created_terminal<F>(self, ensure_selected_terminal: F) -> bool
    where
        F: FnOnce() -> bool,
    {
        match self {
            Self::PreserveTerminalState => false,
            Self::EnsureSelectedTerminal => ensure_selected_terminal(),
        }
    }
}

#[cfg(test)]
fn selected_worktree_terminal_was_created<F>(
    has_existing_terminal: bool,
    ensure_selected_terminal: F,
) -> bool
where
    F: FnOnce() -> bool,
{
    if has_existing_terminal {
        false
    } else {
        ensure_selected_terminal()
    }
}

impl EntityInputHandler for ArborWindow {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.ime_marked_text.as_ref().map(|text| {
            let len: usize = text.encode_utf16().count();
            0..len
        })
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_marked_text = None;
        if text.is_empty() {
            cx.notify();
            return;
        }
        // Suppress all text input while the quit overlay is showing.
        if self.quit_overlay_until.is_some() {
            return;
        }
        // When a modal with a text field is open, route IME text there instead
        if let Some(ref mut modal) = self.daemon_auth_modal {
            modal.token.push_str(text);
            modal.error = None;
            cx.notify();
            return;
        }
        if let Some(ref mut modal) = self.connect_to_host_modal {
            modal.address.push_str(text);
            modal.error = None;
            cx.notify();
            return;
        }
        if let Some(ref mut modal) = self.command_palette_modal {
            modal.query.push_str(text);
            modal.selected_index = 0;
            self.command_palette_scroll_handle.scroll_to_item(0);
            cx.notify();
            return;
        }
        if let Some(ref mut modal) = self.commit_modal {
            modal.message.push_str(text);
            modal.error = None;
            cx.notify();
            return;
        }
        if self.welcome_clone_url_active {
            self.welcome_clone_url.push_str(text);
            self.welcome_clone_error = None;
            cx.notify();
            return;
        }
        if self.worktree_notes_active && self.right_pane_tab == RightPaneTab::Notes {
            self.insert_text_into_selected_worktree_notes(text, cx);
            cx.notify();
            return;
        }
        let Some(session_id) = self.active_terminal_id_for_selected_worktree() else {
            return;
        };
        self.append_pasted_text_to_pending_command(session_id, text);
        if let Err(error) = self.write_input_to_terminal(session_id, text.as_bytes()) {
            self.notice = Some(format!("failed to write to terminal: {error}"));
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_marked_text = if new_text.is_empty() {
            None
        } else {
            Some(new_text.to_owned())
        };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for ArborWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Update window title to reflect connected daemon
        let title = app_window_title(self.connected_daemon_label.as_deref());
        window.set_window_title(&title);

        self.window_is_active = window.is_window_active();
        if self.focus_terminal_on_next_render && self.active_terminal().is_some() {
            window.focus(&self.terminal_focus);
            self.focus_terminal_on_next_render = false;
        }
        let workspace_width = f32::from(window.window_bounds().get_bounds().size.width);
        self.clamp_pane_widths_for_workspace(workspace_width);
        self.sync_ui_state_store(window, cx);

        let theme = self.theme();
        div()
            .size_full()
            .bg(rgb(theme.app_bg))
            .text_color(rgb(theme.text_primary))
            .font_family(FONT_UI)
            .relative()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::handle_global_key_down))
            .on_action(cx.listener(Self::action_spawn_terminal))
            .on_action(cx.listener(Self::action_close_active_terminal))
            .on_action(cx.listener(Self::action_open_manage_presets))
            .on_action(cx.listener(Self::action_open_manage_repo_presets))
            .on_action(cx.listener(Self::action_open_command_palette))
            .on_action(cx.listener(Self::action_refresh_worktrees))
            .on_action(cx.listener(Self::action_refresh_changes))
            .on_action(cx.listener(Self::action_open_add_repository))
            .on_action(cx.listener(Self::action_open_create_worktree))
            .on_action(cx.listener(Self::action_toggle_left_pane))
            .on_action(cx.listener(Self::action_navigate_worktree_back))
            .on_action(cx.listener(Self::action_navigate_worktree_forward))
            .on_action(cx.listener(Self::action_collapse_all_repositories))
            .on_action(cx.listener(Self::action_view_logs))
            .on_action(cx.listener(Self::action_show_about))
            .on_action(cx.listener(Self::action_open_theme_picker))
            .on_action(cx.listener(Self::action_open_settings))
            .on_action(cx.listener(Self::action_open_manage_hosts))
            .on_action(cx.listener(Self::action_connect_to_lan_daemon))
            .on_action(cx.listener(Self::action_connect_to_host))
            .on_action(cx.listener(Self::action_request_quit))
            .on_action(cx.listener(Self::action_immediate_quit))
            .child(self.render_top_bar(cx))
            .child(div().h(px(1.)).bg(rgb(theme.chrome_border)))
            .when(self.repositories.is_empty(), |this| {
                this.child(self.render_welcome_pane(cx))
            })
            .when(!self.repositories.is_empty(), |this| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .flex()
                        .flex_row()
                        .on_drag_move(cx.listener(Self::handle_pane_divider_drag_move))
                        .child(self.render_left_pane(cx))
                        .when(self.left_pane_visible, |this| {
                            this.child(self.render_pane_resize_handle(
                                "left-pane-resize",
                                DraggedPaneDivider::Left,
                                theme,
                            ))
                        })
                        .child(self.render_center_pane(window, cx))
                        .child(self.render_pane_resize_handle(
                            "right-pane-resize",
                            DraggedPaneDivider::Right,
                            theme,
                        ))
                        .child(self.render_right_pane(cx)),
                )
            })
            .child(self.render_status_bar())
            .child(self.render_top_bar_worktree_quick_actions_menu(cx))
            .child(self.render_notice_toast(cx))
            .child(self.render_issue_details_modal(cx))
            .child(self.render_create_modal(cx))
            .child(self.render_github_auth_modal(cx))
            .child(self.render_repository_context_menu(cx))
            .child(self.render_worktree_context_menu(cx))
            .child(self.render_worktree_hover_popover(cx))
            .child(self.render_outpost_context_menu(cx))
            .child(self.render_delete_modal(cx))
            .child(self.render_manage_hosts_modal(cx))
            .child(self.render_manage_presets_modal(cx))
            .child(self.render_manage_repo_presets_modal(cx))
            .child(self.render_commit_modal(cx))
            .child(self.render_command_palette_modal(cx))
            .child(self.render_about_modal(cx))
            .child(self.render_theme_picker_modal(cx))
            .child(self.render_settings_modal(cx))
            .child(self.render_daemon_auth_modal(cx))
            .child(self.render_start_daemon_modal(cx))
            .child(self.render_connect_to_host_modal(cx))
            .child(div().when_some(self.theme_toast.clone(), |this, toast| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_end()
                        .justify_end()
                        .px_3()
                        .pb(px(34.))
                        .child(
                            div()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(theme.accent))
                                .bg(rgb(theme.panel_active_bg))
                                .px_3()
                                .py_2()
                                .text_xs()
                                .text_color(rgb(theme.text_primary))
                                .child(toast),
                        ),
                )
            }))
            .when(self.quit_overlay_until.is_some(), |this| {
                this.child(
                    div()
                        .id("quit-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(rgb(0x000000))
                        .opacity(0.5)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.action_dismiss_quit(window, cx);
                        })),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .occlude()
                        .child(
                            div()
                                .px_6()
                                .py_4()
                                .rounded_lg()
                                .bg(rgb(theme.chrome_bg))
                                .border_1()
                                .border_color(rgb(theme.border))
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(theme.text_primary))
                                        .child("Are you sure you want to quit Arbor?"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            action_button(
                                                theme,
                                                "quit-cancel",
                                                "Cancel",
                                                ActionButtonStyle::Secondary,
                                                true,
                                            )
                                            .min_w(px(64.))
                                            .flex()
                                            .justify_center()
                                            .on_click(
                                                cx.listener(|this, _, window, cx| {
                                                    this.action_dismiss_quit(window, cx);
                                                }),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .id("quit-confirm")
                                                .cursor_pointer()
                                                .rounded_sm()
                                                .border_1()
                                                .border_color(rgb(0xc94040))
                                                .bg(rgb(0xc94040))
                                                .min_w(px(64.))
                                                .flex()
                                                .justify_center()
                                                .px_2()
                                                .py_1()
                                                .text_xs()
                                                .text_color(rgb(0xffffff))
                                                .child("Quit")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.action_confirm_quit(window, cx);
                                                })),
                                        ),
                                ),
                        ),
                )
            })
    }
}

fn worktree_rows_changed(previous: &[WorktreeSummary], next: &[WorktreeSummary]) -> bool {
    if previous.len() != next.len() {
        return true;
    }

    previous.iter().zip(next.iter()).any(|(left, right)| {
        left.group_key != right.group_key
            || left.checkout_kind != right.checkout_kind
            || left.repo_root != right.repo_root
            || left.path != right.path
            || left.label != right.label
            || left.branch != right.branch
            || left.is_primary_checkout != right.is_primary_checkout
            || left.branch_divergence != right.branch_divergence
            || left.detected_ports != right.detected_ports
            || left.managed_processes != right.managed_processes
    })
}

fn managed_processes_for_worktree(
    repo_root: &Path,
    worktree_path: &Path,
) -> Vec<ManagedWorktreeProcess> {
    let mut processes = Vec::new();

    if paths_equivalent(repo_root, worktree_path) {
        processes.extend(arbor_toml_processes_for_worktree(repo_root, worktree_path));
    }
    processes.extend(procfile_processes_for_worktree(worktree_path));

    processes
}

fn arbor_toml_processes_for_worktree(
    repo_root: &Path,
    worktree_path: &Path,
) -> Vec<ManagedWorktreeProcess> {
    let Some(config) = repo_config::load_repo_config(repo_root) else {
        return Vec::new();
    };

    config
        .processes
        .into_iter()
        .filter(|process| !process.name.trim().is_empty() && !process.command.trim().is_empty())
        .map(|process| ManagedWorktreeProcess {
            id: managed_process_id(ProcessSource::ArborToml, worktree_path, &process.name),
            name: process.name,
            command: process.command,
            working_dir: process
                .working_dir
                .as_deref()
                .map(|dir| repo_root.join(dir))
                .unwrap_or_else(|| repo_root.to_path_buf()),
            source: ProcessSource::ArborToml,
        })
        .collect()
}

fn procfile_processes_for_worktree(worktree_path: &Path) -> Vec<ManagedWorktreeProcess> {
    match procfile::read_procfile(worktree_path) {
        Ok(Some(entries)) => entries
            .into_iter()
            .map(|entry| ManagedWorktreeProcess {
                id: managed_process_id(ProcessSource::Procfile, worktree_path, &entry.name),
                name: entry.name,
                command: entry.command,
                working_dir: worktree_path.to_path_buf(),
                source: ProcessSource::Procfile,
            })
            .collect(),
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(path = %worktree_path.display(), %error, "failed to read Procfile");
            Vec::new()
        },
    }
}

fn managed_process_id(source: ProcessSource, worktree_path: &Path, process_name: &str) -> String {
    format!(
        "{}:{}:{process_name}",
        managed_process_source_label(source),
        worktree_path.display()
    )
}

fn managed_process_source_label(source: ProcessSource) -> &'static str {
    match source {
        ProcessSource::ArborToml => "arbor-toml",
        ProcessSource::Procfile => "procfile",
    }
}

fn managed_process_source_display_name(source: ProcessSource) -> &'static str {
    match source {
        ProcessSource::ArborToml => "arbor.toml",
        ProcessSource::Procfile => "Procfile",
    }
}

fn managed_process_title(source: ProcessSource, process_name: &str) -> String {
    managed_process_session_title(source, process_name)
}

fn managed_process_id_from_title(worktree_path: &Path, title: &str) -> Option<String> {
    managed_process_source_and_name_from_title(title)
        .map(|(source, name)| managed_process_id(source, worktree_path, name))
}

fn managed_process_session_is_active(session: &TerminalSession) -> bool {
    session.is_initializing || session.state == TerminalState::Running
}

fn next_active_worktree_index(
    previous_local_selection: Option<&Path>,
    active_repository_group_key: Option<&str>,
    worktrees: &[WorktreeSummary],
    preserve_non_local_selection: bool,
) -> Option<usize> {
    if preserve_non_local_selection {
        return None;
    }

    previous_local_selection
        .and_then(|path| worktrees.iter().position(|worktree| worktree.path == path))
        .or_else(|| {
            active_repository_group_key.and_then(|group_key| {
                worktrees
                    .iter()
                    .position(|worktree| worktree.group_key == group_key)
            })
        })
        .or_else(|| (!worktrees.is_empty()).then_some(0))
}

fn estimated_worktree_hover_popover_card_height(
    worktree: &WorktreeSummary,
    checks_expanded: bool,
) -> Pixels {
    let mut height = 72.;

    if worktree
        .diff_summary
        .is_some_and(|summary| summary.additions > 0 || summary.deletions > 0)
    {
        height += 18.;
    }

    height += 18.;

    if !worktree.recent_turns.is_empty() {
        height += 24. + worktree.recent_turns.iter().take(3).count() as f32 * 18.;
    }

    if !worktree.detected_ports.is_empty() {
        height += 22.;
    }

    if !worktree.recent_agent_sessions.is_empty() {
        let visible_sessions = worktree.recent_agent_sessions.iter().take(4);
        let provider_headers = visible_sessions
            .clone()
            .fold((None, 0usize), |(previous, count), session| {
                if previous == Some(session.provider) {
                    (previous, count)
                } else {
                    (Some(session.provider), count + 1)
                }
            })
            .1;
        height += 24.
            + worktree.recent_agent_sessions.iter().take(4).count() as f32 * 18.
            + provider_headers as f32 * 16.;
    }

    if let Some(pr) = worktree.pr_details.as_ref() {
        height += 110.;
        if checks_expanded
            && !pr.checks.is_empty()
            && matches!(
                pr.state,
                github_service::PrState::Open | github_service::PrState::Draft
            )
        {
            height += pr.checks.len() as f32 * 18.;
        }
    }

    px(height)
}

fn worktree_hover_popover_zone_bounds(
    left_pane_width: f32,
    popover: &WorktreeHoverPopover,
    worktree: &WorktreeSummary,
) -> Bounds<Pixels> {
    let padding = px(WORKTREE_HOVER_POPOVER_ZONE_PADDING_PX);
    Bounds::new(
        point(
            px(left_pane_width) + px(4.) - padding,
            popover.mouse_y - px(8.) - padding,
        ),
        size(
            px(WORKTREE_HOVER_POPOVER_CARD_WIDTH_PX) + padding * 2.,
            estimated_worktree_hover_popover_card_height(worktree, popover.checks_expanded)
                + padding * 2.,
        ),
    )
}

fn worktree_hover_trigger_zone_bounds(left_pane_width: f32, mouse_y: Pixels) -> Bounds<Pixels> {
    let height = px(WORKTREE_HOVER_TRIGGER_ZONE_HEIGHT_PX);
    Bounds::new(
        point(px(0.), mouse_y - height / 2.),
        size(px(left_pane_width), height),
    )
}

fn worktree_hover_safe_zone_contains(
    left_pane_width: f32,
    popover: &WorktreeHoverPopover,
    worktree: &WorktreeSummary,
    position: gpui::Point<Pixels>,
) -> bool {
    worktree_hover_popover_zone_bounds(left_pane_width, popover, worktree).contains(&position)
        || worktree_hover_trigger_zone_bounds(left_pane_width, popover.mouse_y).contains(&position)
}

fn terminal_tab_title(session: &TerminalSession) -> String {
    if let Some(last_command) = session
        .last_command
        .as_ref()
        .filter(|command| !command.trim().is_empty())
    {
        return truncate_with_ellipsis(last_command.trim(), TERMINAL_TAB_COMMAND_MAX_CHARS);
    }

    if !session.title.is_empty() && !session.title.starts_with("term-") {
        return truncate_with_ellipsis(&session.title, TERMINAL_TAB_COMMAND_MAX_CHARS);
    }

    String::new()
}

fn worktree_pull_request_cache_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn cached_pull_request_state_for_worktree<'a>(
    worktree: &WorktreeSummary,
    cache: &'a HashMap<String, ui_state_store::CachedPullRequestState>,
) -> Option<&'a ui_state_store::CachedPullRequestState> {
    cache
        .get(&worktree_pull_request_cache_key(&worktree.path))
        .filter(|cached| cached.branch == worktree.branch)
}

fn should_refresh_pull_requests_after_worktree_refresh(
    previous: &[WorktreeSummary],
    next: &[WorktreeSummary],
) -> bool {
    let previous_tracked: HashMap<&Path, &str> = previous
        .iter()
        .filter(|worktree| should_lookup_pull_request_for_worktree(worktree))
        .map(|worktree| (worktree.path.as_path(), worktree.branch.as_str()))
        .collect();

    let mut next_tracked_count = 0usize;
    for worktree in next
        .iter()
        .filter(|worktree| should_lookup_pull_request_for_worktree(worktree))
    {
        next_tracked_count += 1;

        if !worktree.pr_loaded {
            return true;
        }

        match previous_tracked.get(worktree.path.as_path()) {
            Some(previous_branch) if previous_branch == &worktree.branch.as_str() => {},
            _ => return true,
        }
    }

    next_tracked_count != previous_tracked.len()
}

fn should_show_worktree_pr_loading_indicator(worktree: &WorktreeSummary) -> bool {
    worktree.pr_loading && !worktree.pr_loaded
}

fn loading_status_text(theme: ThemePalette, text: impl Into<String>) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme.accent))
        .child(text.into())
}

fn loading_spinner_frame(frame: usize) -> &'static str {
    LOADING_SPINNER_FRAMES[frame % LOADING_SPINNER_FRAMES.len()]
}

fn action_button(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    style: ActionButtonStyle,
    enabled: bool,
) -> Stateful<Div> {
    let background = if enabled && style == ActionButtonStyle::Primary {
        theme.panel_active_bg
    } else {
        theme.panel_bg
    };
    let text_color = if enabled {
        theme.text_primary
    } else {
        theme.text_disabled
    };

    div()
        .id(id)
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgb(theme.panel_active_bg)))
        })
        .rounded_sm()
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(background))
        .px_2()
        .py_1()
        .text_xs()
        .text_color(rgb(text_color))
        .child(label.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionButtonStyle {
    Primary,
    Secondary,
}

fn preset_icon_image(kind: AgentPresetKind) -> Arc<Image> {
    static CLAUDE_ICON: OnceLock<Arc<Image>> = OnceLock::new();
    static CODEX_ICON: OnceLock<Arc<Image>> = OnceLock::new();
    static PI_ICON: OnceLock<Arc<Image>> = OnceLock::new();
    static OPENCODE_ICON: OnceLock<Arc<Image>> = OnceLock::new();
    static COPILOT_ICON: OnceLock<Arc<Image>> = OnceLock::new();

    let lock = match kind {
        AgentPresetKind::Codex => &CODEX_ICON,
        AgentPresetKind::Claude => &CLAUDE_ICON,
        AgentPresetKind::Pi => &PI_ICON,
        AgentPresetKind::OpenCode => &OPENCODE_ICON,
        AgentPresetKind::Copilot => &COPILOT_ICON,
    };

    lock.get_or_init(|| {
        tracing::info!(
            preset = kind.key(),
            asset = preset_icon_asset_path(kind),
            bytes = preset_icon_bytes(kind).len(),
            "loading preset icon asset"
        );
        Arc::new(Image::from_bytes(
            preset_icon_format(kind),
            preset_icon_bytes(kind).to_vec(),
        ))
    })
    .clone()
}

fn preset_icon_bytes(kind: AgentPresetKind) -> &'static [u8] {
    match kind {
        AgentPresetKind::Codex => PRESET_ICON_CODEX_SVG,
        AgentPresetKind::Claude => PRESET_ICON_CLAUDE_PNG,
        AgentPresetKind::Pi => PRESET_ICON_PI_SVG,
        AgentPresetKind::OpenCode => PRESET_ICON_OPENCODE_SVG,
        AgentPresetKind::Copilot => PRESET_ICON_COPILOT_SVG,
    }
}

fn preset_icon_format(kind: AgentPresetKind) -> ImageFormat {
    match kind {
        AgentPresetKind::Codex
        | AgentPresetKind::Pi
        | AgentPresetKind::OpenCode
        | AgentPresetKind::Copilot => ImageFormat::Svg,
        AgentPresetKind::Claude => ImageFormat::Png,
    }
}

fn preset_icon_asset_path(kind: AgentPresetKind) -> &'static str {
    match kind {
        AgentPresetKind::Codex => "assets/preset-icons/codex-white.svg",
        AgentPresetKind::Claude => "assets/preset-icons/claude.png",
        AgentPresetKind::Pi => "assets/preset-icons/pi-white.svg",
        AgentPresetKind::OpenCode => "assets/preset-icons/opencode-white.svg",
        AgentPresetKind::Copilot => "assets/preset-icons/copilot-white.svg",
    }
}

fn log_preset_icon_fallback_once(kind: AgentPresetKind, fallback_glyph: &'static str) {
    static CLAUDE_FALLBACK_LOGGED: OnceLock<()> = OnceLock::new();
    static CODEX_FALLBACK_LOGGED: OnceLock<()> = OnceLock::new();
    static PI_FALLBACK_LOGGED: OnceLock<()> = OnceLock::new();
    static OPENCODE_FALLBACK_LOGGED: OnceLock<()> = OnceLock::new();
    static COPILOT_FALLBACK_LOGGED: OnceLock<()> = OnceLock::new();

    let once = match kind {
        AgentPresetKind::Codex => &CODEX_FALLBACK_LOGGED,
        AgentPresetKind::Claude => &CLAUDE_FALLBACK_LOGGED,
        AgentPresetKind::Pi => &PI_FALLBACK_LOGGED,
        AgentPresetKind::OpenCode => &OPENCODE_FALLBACK_LOGGED,
        AgentPresetKind::Copilot => &COPILOT_FALLBACK_LOGGED,
    };

    once.get_or_init(|| {
        tracing::warn!(
            preset = kind.key(),
            asset = preset_icon_asset_path(kind),
            bytes = preset_icon_bytes(kind).len(),
            fallback = fallback_glyph,
            "preset icon asset could not be rendered, using fallback glyph"
        );
        eprintln!(
            "WARN preset icon fallback preset={} asset={} bytes={} fallback={}",
            kind.key(),
            preset_icon_asset_path(kind),
            preset_icon_bytes(kind).len(),
            fallback_glyph
        );
    });
}

fn log_preset_icon_render_once(kind: AgentPresetKind) {
    static CLAUDE_RENDER_LOGGED: OnceLock<()> = OnceLock::new();
    static CODEX_RENDER_LOGGED: OnceLock<()> = OnceLock::new();
    static PI_RENDER_LOGGED: OnceLock<()> = OnceLock::new();
    static OPENCODE_RENDER_LOGGED: OnceLock<()> = OnceLock::new();
    static COPILOT_RENDER_LOGGED: OnceLock<()> = OnceLock::new();

    let once = match kind {
        AgentPresetKind::Codex => &CODEX_RENDER_LOGGED,
        AgentPresetKind::Claude => &CLAUDE_RENDER_LOGGED,
        AgentPresetKind::Pi => &PI_RENDER_LOGGED,
        AgentPresetKind::OpenCode => &OPENCODE_RENDER_LOGGED,
        AgentPresetKind::Copilot => &COPILOT_RENDER_LOGGED,
    };

    once.get_or_init(|| {
        tracing::info!(
            preset = kind.key(),
            asset = preset_icon_asset_path(kind),
            "preset icon render path active"
        );
    });
}

fn preset_icon_render_size_px(kind: AgentPresetKind) -> f32 {
    match kind {
        AgentPresetKind::Codex => 20.,
        AgentPresetKind::Claude
        | AgentPresetKind::Pi
        | AgentPresetKind::OpenCode
        | AgentPresetKind::Copilot => 14.,
    }
}

fn agent_preset_button_content(kind: AgentPresetKind, text_color: u32) -> Div {
    log_preset_icon_render_once(kind);
    let icon = preset_icon_image(kind);
    let icon_size = preset_icon_render_size_px(kind);
    // Use consistent slot size for all icons to ensure vertical alignment
    let icon_slot_size = 20_f32;
    let fallback_color = match kind {
        AgentPresetKind::Claude => 0xD97757,
        AgentPresetKind::Codex
        | AgentPresetKind::Pi
        | AgentPresetKind::OpenCode
        | AgentPresetKind::Copilot => text_color,
    };
    let fallback_glyph = match kind {
        AgentPresetKind::Claude => "C",
        AgentPresetKind::Codex
        | AgentPresetKind::Pi
        | AgentPresetKind::OpenCode
        | AgentPresetKind::Copilot => kind.fallback_icon(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .w(px(icon_slot_size))
                .h(px(icon_slot_size))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(img(icon).size(px(icon_size)).with_fallback(move || {
                    log_preset_icon_fallback_once(kind, fallback_glyph);
                    div()
                        .font_family(FONT_MONO)
                        .text_size(px(12.))
                        .line_height(px(12.))
                        .text_color(rgb(fallback_color))
                        .child(fallback_glyph)
                        .into_any_element()
                })),
        )
        .child(
            div()
                .text_size(px(12.))
                .line_height(px(14.))
                .text_color(rgb(text_color))
                .child(kind.label()),
        )
}

fn git_action_button(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    icon: &'static str,
    label: &'static str,
    enabled: bool,
    active: bool,
) -> Stateful<Div> {
    let background = if active {
        theme.panel_active_bg
    } else {
        theme.panel_bg
    };
    let icon_color = if active {
        theme.accent
    } else if enabled {
        theme.text_muted
    } else {
        theme.text_disabled
    };
    let text_color = if enabled || active {
        theme.text_primary
    } else {
        theme.text_disabled
    };

    div()
        .id(id)
        .h(px(24.))
        .rounded_sm()
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(background))
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgb(theme.panel_active_bg)))
        })
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(px(13.))
                .text_color(rgb(icon_color))
                .child(icon),
        )
        .child(div().text_xs().text_color(rgb(text_color)).child(label))
}

fn modal_backdrop() -> Div {
    div().absolute().inset_0().bg(rgb(0x000000)).opacity(0.28)
}

fn modal_input_field(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    value: &str,
    cursor: usize,
    placeholder: impl Into<String>,
    active: bool,
) -> Stateful<Div> {
    let label = label.into();
    let placeholder = placeholder.into();

    div()
        .id(id)
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme.text_muted))
                .child(label),
        )
        .child(
            div()
                .overflow_hidden()
                .cursor_pointer()
                .rounded_sm()
                .border_1()
                .border_color(rgb(if active {
                    theme.accent
                } else {
                    theme.border
                }))
                .bg(rgb(theme.panel_bg))
                .px_2()
                .py_1()
                .text_sm()
                .font_family(FONT_MONO)
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(if active {
                    if value.is_empty() {
                        active_input_display(theme, "", &placeholder, theme.text_disabled, 0, 48)
                    } else {
                        active_input_display(
                            theme,
                            value,
                            &placeholder,
                            theme.text_primary,
                            cursor,
                            56,
                        )
                    }
                } else if value.is_empty() {
                    div()
                        .text_color(rgb(theme.text_disabled))
                        .child(placeholder)
                        .into_any_element()
                } else {
                    div()
                        .text_color(rgb(theme.text_primary))
                        .child(value.to_owned())
                        .into_any_element()
                }),
        )
}

fn single_line_input_field(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    value: &str,
    cursor: usize,
    placeholder: impl Into<String>,
    active: bool,
) -> Stateful<Div> {
    let placeholder = placeholder.into();

    div()
        .id(id)
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .h(px(30.))
        .cursor_text()
        .rounded_sm()
        .border_1()
        .border_color(rgb(if active {
            theme.accent
        } else {
            theme.border
        }))
        .bg(rgb(theme.panel_bg))
        .px_2()
        .text_sm()
        .font_family(FONT_MONO)
        .flex()
        .items_center()
        .child(if active {
            if value.is_empty() {
                active_input_display(theme, "", &placeholder, theme.text_disabled, 0, 48)
            } else {
                active_input_display(theme, value, &placeholder, theme.text_primary, cursor, 48)
            }
        } else {
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(rgb(if value.is_empty() {
                    theme.text_disabled
                } else {
                    theme.text_primary
                }))
                .child(if value.is_empty() {
                    placeholder
                } else {
                    value.to_owned()
                })
                .into_any_element()
        })
}

fn active_input_display(
    theme: ThemePalette,
    value: &str,
    placeholder: &str,
    text_color: u32,
    cursor: usize,
    max_chars: usize,
) -> AnyElement {
    if value.is_empty() {
        return div()
            .relative()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(
                div()
                    .text_color(rgb(text_color))
                    .child(placeholder.to_owned()),
            )
            .child(
                input_caret(theme)
                    .flex_none()
                    .absolute()
                    .left(px(0.))
                    .top(px(2.)),
            )
            .into_any_element();
    }

    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .flex()
        .items_center()
        .justify_start()
        .gap(px(0.))
        .child({
            let (before_cursor, after_cursor) = visible_input_segments(value, cursor, max_chars);
            div()
                .flex()
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(text_color))
                        .child(before_cursor),
                )
                .child(input_caret(theme).flex_none())
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(text_color))
                        .child(after_cursor),
                )
        })
        .into_any_element()
}

fn visible_input_segments(value: &str, cursor: usize, max_chars: usize) -> (String, String) {
    let chars: Vec<char> = value.chars().collect();
    let len = chars.len();
    let cursor = cursor.min(len);
    if len <= max_chars {
        let before: String = chars[..cursor].iter().collect();
        let after: String = chars[cursor..].iter().collect();
        return (before, after);
    }

    let window = max_chars.max(1);
    let preferred_left = window.saturating_sub(8);
    let mut start = cursor.saturating_sub(preferred_left);
    start = start.min(len.saturating_sub(window));
    let end = (start + window).min(len);

    let mut before: String = chars[start..cursor].iter().collect();
    let mut after: String = chars[cursor..end].iter().collect();
    if start > 0 {
        before.insert(0, '\u{2026}');
    }
    if end < len {
        after.push('\u{2026}');
    }
    (before, after)
}

fn input_caret(theme: ThemePalette) -> Div {
    div().w(px(1.)).h(px(14.)).bg(rgb(theme.accent)).mt(px(1.))
}

fn status_text(theme: ThemePalette, text: impl Into<String>) -> Div {
    div()
        .text_xs()
        .text_color(rgb(theme.text_muted))
        .child(text.into())
}

fn should_preserve_cached_pr_data_on_rate_limit(
    next_num: Option<u64>,
    next_url: Option<&str>,
    next_details: Option<&github_service::PrDetails>,
    rate_limited_until: Option<SystemTime>,
) -> bool {
    rate_limited_until.is_some()
        && next_num.is_none()
        && next_url.is_none()
        && next_details.is_none()
}

fn clear_pull_request_data_for_untracked_worktrees(
    worktrees: &mut [WorktreeSummary],
    tracked_paths: &HashSet<PathBuf>,
) -> bool {
    let mut cleared = false;

    for worktree in worktrees {
        if tracked_paths.contains(&worktree.path) {
            continue;
        }
        let had_pr_number = worktree.pr_number.take().is_some();
        let had_pr_url = worktree.pr_url.take().is_some();
        let had_pr_details = worktree.pr_details.take().is_some();
        if had_pr_number || had_pr_url || had_pr_details {
            cleared = true;
        }
    }

    cleared
}

#[derive(Clone, Copy)]
struct WorktreeAttentionIndicator {
    label: &'static str,
    short_label: &'static str,
    color: u32,
}

fn worktree_attention_indicator(worktree: &WorktreeSummary) -> WorktreeAttentionIndicator {
    if worktree.stuck_turn_count >= 2 {
        return WorktreeAttentionIndicator {
            label: "Stuck",
            short_label: "Stuck",
            color: 0xeb6f92,
        };
    }
    if worktree.agent_state == Some(AgentState::Working) {
        return WorktreeAttentionIndicator {
            label: "Working",
            short_label: "Run",
            color: 0xe5c07b,
        };
    }
    if worktree.agent_state == Some(AgentState::Waiting)
        && worktree
            .recent_turns
            .first()
            .and_then(|snapshot| snapshot.diff_summary)
            .is_some_and(|summary| summary.additions > 0 || summary.deletions > 0)
    {
        return WorktreeAttentionIndicator {
            label: "Needs review",
            short_label: "Review",
            color: 0x61afef,
        };
    }
    if worktree.agent_state == Some(AgentState::Waiting) {
        return WorktreeAttentionIndicator {
            label: "Waiting",
            short_label: "Wait",
            color: 0x61afef,
        };
    }
    if !worktree.detected_ports.is_empty() {
        return WorktreeAttentionIndicator {
            label: "Serving",
            short_label: "Ports",
            color: 0x72d69c,
        };
    }
    if worktree.last_activity_unix_ms.is_some_and(|timestamp| {
        current_unix_timestamp_millis()
            .unwrap_or(0)
            .saturating_sub(timestamp)
            <= 15 * 60 * 1000
    }) {
        return WorktreeAttentionIndicator {
            label: "Recent",
            short_label: "Recent",
            color: 0xc0caf5,
        };
    }

    WorktreeAttentionIndicator {
        label: "Idle",
        short_label: "Idle",
        color: 0x7f8490,
    }
}

fn worktree_activity_sparkline(worktree: &WorktreeSummary) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if worktree.recent_turns.is_empty() {
        return String::new();
    }

    let values: Vec<usize> = worktree
        .recent_turns
        .iter()
        .take(5)
        .rev()
        .map(|snapshot| {
            snapshot
                .diff_summary
                .map(|summary| summary.additions + summary.deletions)
                .unwrap_or(0)
        })
        .collect();
    let max_value = values.iter().copied().max().unwrap_or(0);
    if max_value == 0 {
        return "▁▁▁".to_owned();
    }

    values
        .into_iter()
        .map(|value| {
            let index = value.saturating_mul(BARS.len() - 1) / max_value.max(1);
            BARS[index]
        })
        .collect()
}

fn parse_terminal_backend_kind(
    terminal_backend: Option<&str>,
) -> Result<TerminalBackendKind, ConfigParseError> {
    let Some(value) = terminal_backend
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(TerminalBackendKind::Embedded);
    };

    match value.to_ascii_lowercase().as_str() {
        "embedded" => Ok(TerminalBackendKind::Embedded),
        "alacritty" | "ghostty" => Err(ConfigParseError::InvalidValue(format!(
            "terminal_backend `{value}` is no longer supported; Arbor terminals are embedded-only. Using the embedded terminal instead. Configure `embedded_terminal_engine` to choose `alacritty` or `ghostty-vt-experimental`."
        ))),
        _ => Err(ConfigParseError::InvalidValue(format!(
            "invalid terminal_backend `{value}` in config, expected `embedded`"
        ))),
    }
}

fn parse_theme_kind(theme: Option<&str>) -> Result<ThemeKind, ConfigParseError> {
    let Some(value) = theme.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(ThemeKind::One);
    };

    match value.to_ascii_lowercase().as_str() {
        "one-dark" | "onedark" => Ok(ThemeKind::One),
        "ayu-dark" | "ayu" => Ok(ThemeKind::Ayu),
        "gruvbox-dark" | "gruvbox" => Ok(ThemeKind::Gruvbox),
        "dracula" => Ok(ThemeKind::Dracula),
        "solarized-light" | "solarized" => Ok(ThemeKind::SolarizedLight),
        "everforest-dark" | "everforest" => Ok(ThemeKind::Everforest),
        "catppuccin" => Ok(ThemeKind::Catppuccin),
        "catppuccin-latte" => Ok(ThemeKind::CatppuccinLatte),
        "ethereal" => Ok(ThemeKind::Ethereal),
        "flexoki-light" | "flexoki" => Ok(ThemeKind::FlexokiLight),
        "hackerman" => Ok(ThemeKind::Hackerman),
        "kanagawa" => Ok(ThemeKind::Kanagawa),
        "matte-black" | "matteblack" => Ok(ThemeKind::MatteBlack),
        "miasma" => Ok(ThemeKind::Miasma),
        "nord" => Ok(ThemeKind::Nord),
        "osaka-jade" | "osakajade" => Ok(ThemeKind::OsakaJade),
        "ristretto" => Ok(ThemeKind::Ristretto),
        "rose-pine" | "rosepine" => Ok(ThemeKind::RosePine),
        "tokyo-night" | "tokyonight" => Ok(ThemeKind::TokyoNight),
        "vantablack" => Ok(ThemeKind::Vantablack),
        "white" => Ok(ThemeKind::White),
        "atom-one-light" | "atomonelight" => Ok(ThemeKind::AtomOneLight),
        "github-light-default" | "githublightdefault" => Ok(ThemeKind::GitHubLightDefault),
        "github-light-high-contrast" | "githublighthighcontrast" => {
            Ok(ThemeKind::GitHubLightHighContrast)
        },
        "github-light-colorblind" | "githublightcolorblind" => Ok(ThemeKind::GitHubLightColorblind),
        "github-light" | "githublight" => Ok(ThemeKind::GitHubLight),
        "github-dark-default" | "githubdarkdefault" => Ok(ThemeKind::GitHubDarkDefault),
        "github-dark-high-contrast" | "githubdarkhighcontrast" => {
            Ok(ThemeKind::GitHubDarkHighContrast)
        },
        "github-dark-colorblind" | "githubdarkcolorblind" => Ok(ThemeKind::GitHubDarkColorblind),
        "github-dark-dimmed" | "githubdarkdimmed" => Ok(ThemeKind::GitHubDarkDimmed),
        "github-dark" | "githubdark" => Ok(ThemeKind::GitHubDark),
        "retrobox-classic" | "retrobox" => Ok(ThemeKind::RetroboxClassic),
        "tokyonight-day" | "tokionight-day" => Ok(ThemeKind::TokyoNightDay),
        "tokyonight-classic" | "tokionight-classic" => Ok(ThemeKind::TokyoNightClassic),
        "zellner" => Ok(ThemeKind::Zellner),
        _ => Err(ConfigParseError::InvalidValue(format!(
            "invalid theme `{value}` in config, expected one-dark/ayu-dark/gruvbox-dark/dracula/solarized-light/everforest-dark/catppuccin/catppuccin-latte/ethereal/flexoki-light/hackerman/kanagawa/matte-black/miasma/nord/osaka-jade/ristretto/rose-pine/tokyo-night/vantablack/white/atom-one-light/github-light-default/github-light-high-contrast/github-light-colorblind/github-light/github-dark-default/github-dark-high-contrast/github-dark-colorblind/github-dark-dimmed/github-dark/retrobox-classic/tokyonight-day/tokyonight-classic/zellner"
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use {
        crate::{
            DaemonTerminalRuntime, DaemonTerminalWsState, PendingSave, TerminalRuntimeHandle,
            TerminalRuntimeKind, TerminalSession, TerminalState, WorktreeHoverPopover,
            WorktreeSummary, apply_daemon_snapshot,
            checkout::CheckoutKind,
            estimated_worktree_hover_popover_card_height, extract_first_url,
            parse_terminal_backend_kind, prioritized_pr_checks_for_display,
            resolve_github_access_token_from_sources, styled_lines_for_session,
            terminal_backend::{
                TerminalBackendKind, TerminalCursor, TerminalModes, TerminalStyledCell,
                TerminalStyledLine, TerminalStyledRun,
            },
            terminal_daemon_http::{HttpTerminalDaemon, WebsocketConnectConfig},
            theme::ThemeKind,
            track_terminal_command_keystroke, ui_state_store, worktree_hover_popover_zone_bounds,
            worktree_hover_safe_zone_contains,
        },
        arbor_core::{agent::AgentState, changes::DiffLineSummary, daemon, process::ProcessSource},
        gpui::{Keystroke, point, px},
        std::{
            cell::Cell,
            collections::HashSet,
            env, fs,
            path::{Path, PathBuf},
            sync::Arc,
            time::{Instant, SystemTime},
        },
    };

    fn session_with_styled_line(
        text: &str,
        fg: u32,
        bg: u32,
        cursor: Option<TerminalCursor>,
    ) -> TerminalSession {
        TerminalSession {
            id: 1,
            daemon_session_id: "daemon-test-1".to_owned(),
            worktree_path: PathBuf::from("/tmp/worktree"),
            managed_process_id: None,
            title: "term-1".to_owned(),
            last_command: None,
            pending_command: String::new(),
            command: "zsh".to_owned(),
            agent_preset: None,
            execution_mode: None,
            state: TerminalState::Running,
            exit_code: None,
            updated_at_unix_ms: None,
            root_pid: None,
            cols: 120,
            rows: 35,
            generation: 0,
            output: text.to_owned(),
            styled_output: vec![TerminalStyledLine {
                cells: text
                    .chars()
                    .enumerate()
                    .map(|(column, character)| TerminalStyledCell {
                        column,
                        text: character.to_string(),
                        fg,
                        bg,
                    })
                    .collect(),
                runs: vec![TerminalStyledRun {
                    text: text.to_owned(),
                    fg,
                    bg,
                }],
            }],
            cursor,
            modes: TerminalModes::default(),
            last_runtime_sync_at: None,
            queued_input: Vec::new(),
            is_initializing: false,
            runtime: None,
        }
    }

    fn sample_worktree_summary() -> WorktreeSummary {
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
            agent_state: Some(AgentState::Working),
            agent_task: Some("Investigating hover".to_owned()),
            last_activity_unix_ms: None,
        }
    }

    #[test]
    fn parse_terminal_backend_defaults_to_embedded() {
        assert_eq!(
            parse_terminal_backend_kind(None),
            Ok(TerminalBackendKind::Embedded),
        );
        assert_eq!(
            parse_terminal_backend_kind(Some("")),
            Ok(TerminalBackendKind::Embedded),
        );
    }

    #[test]
    fn parse_terminal_backend_rejects_external_backends() {
        let alacritty = parse_terminal_backend_kind(Some("alacritty"));
        let ghostty = parse_terminal_backend_kind(Some("ghostty"));

        assert!(alacritty.is_err());
        assert!(ghostty.is_err());
    }

    fn daemon_runtime_for_test() -> DaemonTerminalRuntime {
        let daemon = match HttpTerminalDaemon::new("http://127.0.0.1:1") {
            Ok(daemon) => daemon,
            Err(error) => panic!("failed to create daemon client: {error}"),
        };

        DaemonTerminalRuntime {
            daemon: Arc::new(daemon),
            ws_state: Arc::new(DaemonTerminalWsState::default()),
            last_synced_ws_generation: std::sync::atomic::AtomicU64::new(0),
            snapshot_request_in_flight: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            kind: TerminalRuntimeKind::Local,
            resize_error_label: "resize",
            exit_labels: None,
            clear_global_daemon_on_connection_refused: false,
        }
    }

    #[test]
    fn attention_indicator_prefers_stuck_state() {
        let mut worktree = sample_worktree_summary();
        worktree.agent_state = Some(AgentState::Waiting);
        worktree.stuck_turn_count = 2;

        let attention = crate::worktree_attention_indicator(&worktree);
        assert_eq!(attention.label, "Stuck");
    }

    #[test]
    fn worktree_rows_changed_detects_external_worktree_addition() {
        let previous = sample_worktree_summary();
        let current = sample_worktree_summary();
        let mut external = sample_worktree_summary();
        external.path = "/tmp/repo/wt-external".into();
        external.label = "wt-external".to_owned();
        external.branch = "feature/external".to_owned();

        assert!(crate::worktree_rows_changed(&[previous], &[
            current, external
        ]));
    }

    #[test]
    fn selected_worktree_terminal_existing_session_is_not_reported_as_created() {
        let spawn_called = Cell::new(false);

        let created = crate::selected_worktree_terminal_was_created(true, || {
            spawn_called.set(true);
            true
        });

        assert!(!created);
        assert!(!spawn_called.get());
    }

    #[test]
    fn selected_worktree_terminal_reports_spawn_result_when_missing() {
        assert!(crate::selected_worktree_terminal_was_created(false, || {
            true
        }));
        assert!(!crate::selected_worktree_terminal_was_created(
            false,
            || false
        ));
    }

    #[test]
    fn background_inventory_refresh_does_not_recreate_selected_terminal() {
        let ensure_called = Cell::new(false);

        let created =
            crate::WorktreeInventoryRefreshMode::PreserveTerminalState.created_terminal(|| {
                ensure_called.set(true);
                true
            });

        assert!(!created);
        assert!(!ensure_called.get());
    }

    #[test]
    fn explicit_inventory_refresh_reports_selected_terminal_creation() {
        assert!(
            crate::WorktreeInventoryRefreshMode::EnsureSelectedTerminal.created_terminal(|| true)
        );
        assert!(
            !crate::WorktreeInventoryRefreshMode::EnsureSelectedTerminal.created_terminal(|| false)
        );
    }

    #[test]
    fn next_active_worktree_index_preserves_non_local_selection() {
        let worktree = sample_worktree_summary();
        let group_key = worktree.group_key.clone();

        assert_eq!(
            crate::next_active_worktree_index(None, Some(group_key.as_str()), &[worktree], true),
            None
        );
    }

    #[test]
    fn next_active_worktree_index_restores_previous_local_selection() {
        let first = sample_worktree_summary();
        let mut second = sample_worktree_summary();
        second.path = "/tmp/repo/wt-two".into();
        second.label = "wt-two".to_owned();
        second.branch = "feature/two".to_owned();
        let second_path = second.path.clone();
        let first_group_key = first.group_key.clone();

        assert_eq!(
            crate::next_active_worktree_index(
                Some(second_path.as_path()),
                Some(first_group_key.as_str()),
                &[first, second],
                false,
            ),
            Some(1)
        );
    }

    #[test]
    fn pull_request_refresh_only_restarts_when_tracked_worktrees_change() {
        let mut previous = sample_worktree_summary();
        previous.pr_loaded = true;

        let mut next = sample_worktree_summary();
        next.pr_loaded = true;

        assert!(!crate::should_refresh_pull_requests_after_worktree_refresh(
            &[previous],
            &[next]
        ));
    }

    #[test]
    fn pull_request_refresh_restarts_for_unresolved_or_changed_worktrees() {
        let mut previous = sample_worktree_summary();
        previous.pr_loaded = true;

        let unresolved = sample_worktree_summary();
        assert!(crate::should_refresh_pull_requests_after_worktree_refresh(
            &[previous.clone()],
            &[unresolved]
        ));

        let mut changed_branch = sample_worktree_summary();
        changed_branch.pr_loaded = true;
        changed_branch.branch = "feature/other".to_owned();
        assert!(crate::should_refresh_pull_requests_after_worktree_refresh(
            &[previous],
            &[changed_branch]
        ));
    }

    #[test]
    fn normalized_sidebar_order_keeps_saved_items_and_appends_new_ones() {
        let saved = vec![
            crate::SidebarItemId::Outpost("outpost-1".to_owned()),
            crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-2")),
        ];
        let worktrees = vec![
            crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-1")),
            crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-2")),
        ];
        let outposts = vec![crate::SidebarItemId::Outpost("outpost-1".to_owned())];

        assert_eq!(
            crate::sidebar::normalized_sidebar_order(Some(saved.as_slice()), worktrees, outposts),
            vec![
                crate::SidebarItemId::Outpost("outpost-1".to_owned()),
                crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-2")),
                crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-1")),
            ]
        );
    }

    #[test]
    fn reordered_sidebar_items_moves_dragged_item_to_requested_slot() {
        let items = vec![
            crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-1")),
            crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-2")),
            crate::SidebarItemId::Outpost("outpost-1".to_owned()),
        ];

        assert_eq!(
            crate::sidebar::reordered_sidebar_items(
                &items,
                &crate::SidebarItemId::Outpost("outpost-1".to_owned()),
                0,
            ),
            Some(vec![
                crate::SidebarItemId::Outpost("outpost-1".to_owned()),
                crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-1")),
                crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-2")),
            ])
        );

        assert_eq!(
            crate::sidebar::reordered_sidebar_items(
                &items,
                &crate::SidebarItemId::Worktree(PathBuf::from("/tmp/repo/wt-1")),
                1,
            ),
            None
        );
    }

    #[test]
    fn active_terminal_sync_is_prioritized() {
        let mut first = session_with_styled_line("one", 0xffffff, 0x000000, None);
        first.id = 10;
        let mut second = session_with_styled_line("two", 0xffffff, 0x000000, None);
        second.id = 20;
        let mut third = session_with_styled_line("three", 0xffffff, 0x000000, None);
        third.id = 30;

        let indices = crate::ordered_terminal_sync_indices(&[first, second, third], Some(30));

        assert_eq!(indices, vec![2, 0, 1]);
    }

    #[test]
    fn daemon_terminal_sync_interval_uses_active_fallback() {
        assert_eq!(
            crate::daemon_terminal_sync_interval(true, TerminalState::Running),
            crate::ACTIVE_DAEMON_TERMINAL_SYNC_INTERVAL
        );
        assert_eq!(
            crate::daemon_terminal_sync_interval(false, TerminalState::Running),
            crate::INACTIVE_DAEMON_TERMINAL_SYNC_INTERVAL
        );
        assert_eq!(
            crate::daemon_terminal_sync_interval(false, TerminalState::Completed),
            crate::IDLE_DAEMON_TERMINAL_SYNC_INTERVAL
        );
        assert_eq!(
            crate::daemon_terminal_sync_interval(false, TerminalState::Failed),
            crate::IDLE_DAEMON_TERMINAL_SYNC_INTERVAL
        );
    }

    #[test]
    fn daemon_runtime_syncs_active_session_immediately_on_ws_event() {
        let runtime = daemon_runtime_for_test();
        let mut session = session_with_styled_line("prompt", 0xffffff, 0x000000, None);
        let now = Instant::now();
        session.last_runtime_sync_at = Some(now);

        assert!(!runtime.should_sync(&session, true, None, now));

        runtime.ws_state.note_event();

        assert!(runtime.should_sync(&session, true, None, now));
    }

    #[test]
    fn daemon_runtime_throttles_inactive_sessions_even_when_ws_is_dirty() {
        let runtime = daemon_runtime_for_test();
        let mut session = session_with_styled_line("background", 0xffffff, 0x000000, None);
        let now = Instant::now();
        session.last_runtime_sync_at = Some(now);

        runtime.ws_state.note_event();

        assert!(!runtime.should_sync(&session, false, None, now));
        assert!(runtime.should_sync(
            &session,
            false,
            None,
            now + crate::INACTIVE_DAEMON_TERMINAL_SYNC_INTERVAL
        ));
    }

    #[test]
    fn daemon_runtime_syncs_active_resize_without_waiting_for_ws() {
        let runtime = daemon_runtime_for_test();
        let mut session = session_with_styled_line("prompt", 0xffffff, 0x000000, None);
        let now = Instant::now();
        session.last_runtime_sync_at = Some(now);

        assert!(runtime.should_sync(
            &session,
            true,
            Some((session.rows + 1, session.cols, 0, 0)),
            now
        ));
    }

    #[test]
    fn orphaned_daemon_session_cleanup_kills_only_running_sessions() {
        let mut record = daemon::DaemonSessionRecord {
            session_id: "daemon-test-1".into(),
            workspace_id: "/tmp/worktree".into(),
            cwd: PathBuf::from("/tmp/worktree"),
            shell: "zsh".to_owned(),
            ..Default::default()
        };

        assert!(crate::orphaned_daemon_session_should_kill(&record));

        record.state = Some(daemon::TerminalSessionState::Completed);
        assert!(!crate::orphaned_daemon_session_should_kill(&record));

        record.state = Some(daemon::TerminalSessionState::Failed);
        assert!(!crate::orphaned_daemon_session_should_kill(&record));
    }

    #[test]
    fn background_config_save_has_work_when_count_is_nonzero() {
        assert!(!crate::background_config_save_has_work(0));
        assert!(crate::background_config_save_has_work(1));
        assert!(crate::background_config_save_has_work(3));
    }

    #[test]
    fn terminal_input_buffers_only_while_session_is_initializing() {
        let mut session = session_with_styled_line("prompt", 0xffffff, 0x000000, None);

        session.is_initializing = true;
        assert!(crate::terminal_interaction::should_queue_terminal_input(
            &session
        ));

        session.is_initializing = false;
        assert!(!crate::terminal_interaction::should_queue_terminal_input(
            &session
        ));

        session.is_initializing = true;
        session.runtime = Some(Arc::new(daemon_runtime_for_test()));
        assert!(!crate::terminal_interaction::should_queue_terminal_input(
            &session
        ));
    }

    #[test]
    fn daemon_runtime_without_cached_snapshot_returns_without_sync_error() {
        let runtime = daemon_runtime_for_test();
        let mut session = session_with_styled_line("", 0xffffff, 0x000000, None);
        session.output.clear();
        session.styled_output.clear();
        session.cursor = None;
        session.last_runtime_sync_at = Some(Instant::now());

        let outcome = runtime.sync(&mut session, true, None);

        assert!(!outcome.changed);
        assert!(outcome.notice.is_none());
        assert_eq!(session.state, TerminalState::Running);
        assert!(session.output.is_empty());
    }

    #[test]
    fn daemon_ws_state_rehydrates_trimmed_snapshot_from_ansi_output() {
        let ws_state = DaemonTerminalWsState::default();
        ws_state.apply_snapshot_text("hello\r\nworld\r\n", TerminalState::Running, None, Some(42));

        let snapshot = ws_state
            .snapshot()
            .unwrap_or_else(|| panic!("expected websocket snapshot to be available"));

        assert_eq!(snapshot.state, TerminalState::Running);
        assert_eq!(snapshot.updated_at_unix_ms, Some(42));
        assert!(snapshot.terminal.output.contains("hello"));
        assert!(snapshot.terminal.output.contains("world"));
        assert_eq!(snapshot.terminal.styled_lines.len(), 2);
    }

    #[test]
    fn daemon_runtime_sync_applies_cached_ws_snapshot_without_http_roundtrip() {
        let runtime = daemon_runtime_for_test();
        let mut session = session_with_styled_line("", 0xffffff, 0x000000, None);
        session.output.clear();
        session.styled_output.clear();
        session.cursor = None;
        session.exit_code = None;
        runtime.ws_state.apply_snapshot_text(
            "codex> working\r\n",
            TerminalState::Running,
            None,
            Some(99),
        );

        let outcome = runtime.sync(&mut session, true, None);

        assert!(outcome.changed);
        assert_eq!(session.state, TerminalState::Running);
        assert_eq!(session.updated_at_unix_ms, Some(99));
        assert!(session.output.contains("codex> working"));
        assert_eq!(session.exit_code, None);
    }

    #[test]
    fn daemon_websocket_request_adds_bearer_auth_header() {
        let request = match crate::daemon_websocket_request(&WebsocketConnectConfig {
            url: "ws://127.0.0.1:8787/api/v1/agent/activity/ws".to_owned(),
            auth_token: Some("secret-token".to_owned()),
        }) {
            Ok(request) => request,
            Err(error) => panic!("failed to build websocket request: {error}"),
        };

        assert_eq!(
            request
                .headers()
                .get(tungstenite::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer secret-token")
        );
    }

    #[test]
    fn worktree_hover_safe_zone_covers_trigger_row_and_popover() {
        let worktree = sample_worktree_summary();
        let popover = WorktreeHoverPopover {
            worktree_index: 0,
            mouse_y: px(100.),
            checks_expanded: false,
        };

        assert!(worktree_hover_safe_zone_contains(
            290.,
            &popover,
            &worktree,
            point(px(40.), px(100.)),
        ));
        assert!(worktree_hover_safe_zone_contains(
            290.,
            &popover,
            &worktree,
            point(px(320.), px(112.)),
        ));
        assert!(!worktree_hover_safe_zone_contains(
            290.,
            &popover,
            &worktree,
            point(px(700.), px(100.)),
        ));
    }

    #[test]
    fn expanded_checks_increase_worktree_hover_popover_height() {
        let mut worktree = sample_worktree_summary();
        worktree.pr_details = Some(crate::github_service::PrDetails {
            number: 42,
            title: "Improve hover stability".to_owned(),
            url: "https://example.com/pr/42".to_owned(),
            state: crate::github_service::PrState::Open,
            additions: 12,
            deletions: 4,
            review_decision: crate::github_service::ReviewDecision::Pending,
            mergeable: crate::github_service::MergeableState::Mergeable,
            merge_state_status: crate::github_service::MergeStateStatus::Clean,
            passed_checks: 1,
            checks_status: crate::github_service::CheckStatus::Pending,
            checks: vec![
                ("ci".to_owned(), crate::github_service::CheckStatus::Pending),
                (
                    "lint".to_owned(),
                    crate::github_service::CheckStatus::Success,
                ),
            ],
        });

        let collapsed = estimated_worktree_hover_popover_card_height(&worktree, false);
        let expanded = estimated_worktree_hover_popover_card_height(&worktree, true);
        let collapsed_bounds = worktree_hover_popover_zone_bounds(
            290.,
            &WorktreeHoverPopover {
                worktree_index: 0,
                mouse_y: px(120.),
                checks_expanded: false,
            },
            &worktree,
        );
        let expanded_bounds = worktree_hover_popover_zone_bounds(
            290.,
            &WorktreeHoverPopover {
                worktree_index: 0,
                mouse_y: px(120.),
                checks_expanded: true,
            },
            &worktree,
        );

        assert!(expanded > collapsed);
        assert!(expanded_bounds.size.height > collapsed_bounds.size.height);
    }

    #[test]
    fn prioritized_pr_checks_show_failures_before_pending_before_successes() {
        let pr = crate::github_service::PrDetails {
            number: 7,
            title: "Sort checks".to_owned(),
            url: "https://example.com/pr/7".to_owned(),
            state: crate::github_service::PrState::Open,
            additions: 1,
            deletions: 1,
            review_decision: crate::github_service::ReviewDecision::Pending,
            mergeable: crate::github_service::MergeableState::Mergeable,
            merge_state_status: crate::github_service::MergeStateStatus::Clean,
            passed_checks: 2,
            checks_status: crate::github_service::CheckStatus::Pending,
            checks: vec![
                (
                    "b-failure".to_owned(),
                    crate::github_service::CheckStatus::Failure,
                ),
                (
                    "a-pending".to_owned(),
                    crate::github_service::CheckStatus::Pending,
                ),
                (
                    "a-success".to_owned(),
                    crate::github_service::CheckStatus::Success,
                ),
                (
                    "z-success".to_owned(),
                    crate::github_service::CheckStatus::Success,
                ),
            ],
        };

        let checks = prioritized_pr_checks_for_display(&pr);

        assert_eq!(checks, &[
            (
                "b-failure".to_owned(),
                crate::github_service::CheckStatus::Failure
            ),
            (
                "a-pending".to_owned(),
                crate::github_service::CheckStatus::Pending
            ),
            (
                "a-success".to_owned(),
                crate::github_service::CheckStatus::Success
            ),
            (
                "z-success".to_owned(),
                crate::github_service::CheckStatus::Success
            ),
        ]);
    }

    #[test]
    fn shift_enter_does_not_submit_pending_terminal_command() {
        let mut session = session_with_styled_line("", 0xffffff, 0x000000, None);
        session.pending_command = "hello".to_owned();

        track_terminal_command_keystroke(
            &mut session,
            &Keystroke::parse("shift-enter").expect("valid keystroke"),
        );

        assert_eq!(session.pending_command, "hello\n");
        assert_eq!(session.last_command, None);
    }

    #[test]
    fn daemon_snapshot_applies_structured_terminal_state() {
        let mut session = session_with_styled_line("", 0xffffff, 0x000000, None);
        session.output.clear();
        session.styled_output.clear();
        session.cursor = None;
        session.modes = TerminalModes::default();

        let changed = apply_daemon_snapshot(&mut session, &daemon::TerminalSnapshot {
            session_id: "daemon-test-1".to_owned().into(),
            output_tail: "READY".to_owned(),
            styled_lines: vec![daemon::DaemonTerminalStyledLine {
                cells: vec![daemon::DaemonTerminalStyledCell {
                    column: 0,
                    text: "READY".to_owned(),
                    fg: 0x123456,
                    bg: 0x654321,
                }],
                runs: vec![daemon::DaemonTerminalStyledRun {
                    text: "READY".to_owned(),
                    fg: 0x123456,
                    bg: 0x654321,
                }],
            }],
            cursor: Some(daemon::DaemonTerminalCursor { line: 0, column: 5 }),
            modes: daemon::DaemonTerminalModes {
                app_cursor: true,
                alt_screen: true,
            },
            exit_code: None,
            state: daemon::TerminalSessionState::Running,
            updated_at_unix_ms: Some(1),
        });

        assert!(changed);
        assert_eq!(session.output, "READY");
        assert_eq!(session.cursor, Some(TerminalCursor { line: 0, column: 5 }));
        assert_eq!(session.modes, TerminalModes {
            app_cursor: true,
            alt_screen: true,
        });
        assert_eq!(session.styled_output.len(), 1);
        assert_eq!(session.styled_output[0].runs[0].text, "READY");
        assert_eq!(session.styled_output[0].runs[0].fg, 0x123456);
        assert_eq!(session.styled_output[0].runs[0].bg, 0x654321);
    }

    #[test]
    fn auto_follow_requires_new_output_and_bottom_position() {
        assert!(crate::should_auto_follow_terminal_output(true, true));
        assert!(!crate::should_auto_follow_terminal_output(true, false));
        assert!(!crate::should_auto_follow_terminal_output(false, true));
    }

    #[test]
    fn auto_follow_is_disabled_without_new_output() {
        assert!(!crate::should_auto_follow_terminal_output(false, false));
    }

    #[test]
    fn parse_theme_kind_supports_solarized_light_aliases() {
        assert_eq!(
            crate::parse_theme_kind(Some("solarized-light")).ok(),
            Some(ThemeKind::SolarizedLight)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("solarized")).ok(),
            Some(ThemeKind::SolarizedLight)
        );
    }

    #[test]
    fn parse_theme_kind_supports_everforest_aliases() {
        assert_eq!(
            crate::parse_theme_kind(Some("everforest-dark")).ok(),
            Some(ThemeKind::Everforest)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("everforest")).ok(),
            Some(ThemeKind::Everforest)
        );
    }

    #[test]
    fn parse_theme_kind_supports_omarchy_and_custom_aliases() {
        assert_eq!(
            crate::parse_theme_kind(Some("catppuccin")).ok(),
            Some(ThemeKind::Catppuccin)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("catppuccin-latte")).ok(),
            Some(ThemeKind::CatppuccinLatte)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("ethereal")).ok(),
            Some(ThemeKind::Ethereal)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("flexoki-light")).ok(),
            Some(ThemeKind::FlexokiLight)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("hackerman")).ok(),
            Some(ThemeKind::Hackerman)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("kanagawa")).ok(),
            Some(ThemeKind::Kanagawa)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("matte-black")).ok(),
            Some(ThemeKind::MatteBlack)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("miasma")).ok(),
            Some(ThemeKind::Miasma)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("nord")).ok(),
            Some(ThemeKind::Nord)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("osaka-jade")).ok(),
            Some(ThemeKind::OsakaJade)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("ristretto")).ok(),
            Some(ThemeKind::Ristretto)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("rose-pine")).ok(),
            Some(ThemeKind::RosePine)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("tokyo-night")).ok(),
            Some(ThemeKind::TokyoNight)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("vantablack")).ok(),
            Some(ThemeKind::Vantablack)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("white")).ok(),
            Some(ThemeKind::White)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("atom-one-light")).ok(),
            Some(ThemeKind::AtomOneLight)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-light-default")).ok(),
            Some(ThemeKind::GitHubLightDefault)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-light-high-contrast")).ok(),
            Some(ThemeKind::GitHubLightHighContrast)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-light-colorblind")).ok(),
            Some(ThemeKind::GitHubLightColorblind)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-light")).ok(),
            Some(ThemeKind::GitHubLight)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-dark-default")).ok(),
            Some(ThemeKind::GitHubDarkDefault)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-dark-high-contrast")).ok(),
            Some(ThemeKind::GitHubDarkHighContrast)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-dark-colorblind")).ok(),
            Some(ThemeKind::GitHubDarkColorblind)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-dark-dimmed")).ok(),
            Some(ThemeKind::GitHubDarkDimmed)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("github-dark")).ok(),
            Some(ThemeKind::GitHubDark)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("retrobox-classic")).ok(),
            Some(ThemeKind::RetroboxClassic)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("retrobox")).ok(),
            Some(ThemeKind::RetroboxClassic)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("tokyonight-day")).ok(),
            Some(ThemeKind::TokyoNightDay)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("tokionight-day")).ok(),
            Some(ThemeKind::TokyoNightDay)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("tokyonight-classic")).ok(),
            Some(ThemeKind::TokyoNightClassic)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("tokionight-classic")).ok(),
            Some(ThemeKind::TokyoNightClassic)
        );
        assert_eq!(
            crate::parse_theme_kind(Some("zellner")).ok(),
            Some(ThemeKind::Zellner)
        );
    }

    #[test]
    fn computes_terminal_grid_size_from_viewport() {
        let result = crate::terminal_grid_size_for_viewport(
            900.,
            380.,
            crate::TERMINAL_CELL_WIDTH_PX,
            crate::TERMINAL_CELL_HEIGHT_PX,
        );
        assert_eq!(result, Some((20, 100)));
    }

    #[test]
    fn cursor_is_painted_at_terminal_column_instead_of_line_end() {
        let theme = ThemeKind::One.palette();
        let session = session_with_styled_line(
            "abcdef",
            0x112233,
            0x445566,
            Some(TerminalCursor { line: 0, column: 2 }),
        );

        let lines = styled_lines_for_session(&session, theme, true, None, None);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 3);
        assert_eq!(lines[0].runs[0].text, "ab");
        assert_eq!(lines[0].runs[1].text, "c");
        assert_eq!(lines[0].runs[1].fg, 0x112233);
        assert_eq!(lines[0].runs[1].bg, theme.terminal_cursor);
        assert_eq!(lines[0].runs[2].text, "def");
    }

    #[test]
    fn cursor_pads_to_column_when_it_is_after_line_content() {
        let theme = ThemeKind::One.palette();
        let session = session_with_styled_line(
            "abc",
            0x112233,
            0x445566,
            Some(TerminalCursor { line: 0, column: 5 }),
        );

        let lines = styled_lines_for_session(&session, theme, true, None, None);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 2);
        assert_eq!(lines[0].runs[0].text, "abc");
        assert_eq!(lines[0].runs[1].text, " ");
        assert_eq!(lines[0].runs[1].fg, theme.text_primary);
        assert_eq!(lines[0].runs[1].bg, theme.terminal_cursor);
        assert!(lines[0].cells.iter().any(|cell| {
            cell.column == 5 && cell.text == " " && cell.bg == theme.terminal_cursor
        }));
    }

    #[test]
    fn positioned_runs_split_cells_with_zero_width_sequences() {
        let cells = vec![
            TerminalStyledCell {
                column: 0,
                text: "A".to_owned(),
                fg: 0x112233,
                bg: 0x445566,
            },
            TerminalStyledCell {
                column: 1,
                text: "☀️".to_owned(),
                fg: 0x112233,
                bg: 0x445566,
            },
            TerminalStyledCell {
                column: 2,
                text: "B".to_owned(),
                fg: 0x112233,
                bg: 0x445566,
            },
        ];

        let runs = crate::positioned_runs_from_cells(&cells);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "A");
        assert_eq!(runs[0].start_column, 0);
        assert_eq!(runs[0].cell_count, 1);
        assert!(runs[0].force_cell_width);
        assert_eq!(runs[1].text, "☀️");
        assert_eq!(runs[1].start_column, 1);
        assert_eq!(runs[1].cell_count, 1);
        assert!(!runs[1].force_cell_width);
        assert_eq!(runs[2].text, "B");
        assert_eq!(runs[2].start_column, 2);
        assert_eq!(runs[2].cell_count, 1);
        assert!(runs[2].force_cell_width);
    }

    #[test]
    fn positioned_runs_do_not_force_cell_width_for_powerline_symbols() {
        let cells = vec![
            TerminalStyledCell {
                column: 0,
                text: "\u{e0b0}".to_owned(),
                fg: 0xaabbcc,
                bg: 0x112233,
            },
            TerminalStyledCell {
                column: 1,
                text: "X".to_owned(),
                fg: 0xaabbcc,
                bg: 0x112233,
            },
        ];

        let runs = crate::positioned_runs_from_cells(&cells);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "\u{e0b0}");
        assert!(!runs[0].force_cell_width);
        assert_eq!(runs[1].text, "X");
        assert!(runs[1].force_cell_width);
    }

    #[test]
    fn positioned_runs_keep_cell_width_for_box_drawing_symbols() {
        let cells = vec![
            TerminalStyledCell {
                column: 0,
                text: "│".to_owned(),
                fg: 0xaabbcc,
                bg: 0x112233,
            },
            TerminalStyledCell {
                column: 1,
                text: "X".to_owned(),
                fg: 0xaabbcc,
                bg: 0x112233,
            },
        ];

        let runs = crate::positioned_runs_from_cells(&cells);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "│X");
        assert!(runs[0].force_cell_width);
    }

    #[test]
    fn powerline_glyph_is_forced_to_cell_width() {
        let run = crate::PositionedTerminalRun {
            text: "\u{e0b6}".to_owned(),
            fg: 0,
            bg: 0,
            start_column: 7,
            cell_count: 1,
            force_cell_width: false,
        };

        assert!(crate::should_force_powerline(&run));
    }

    #[test]
    fn token_bounds_capture_full_url() {
        let lines = vec!["visit https://example.com/path?q=1 please".to_owned()];
        let point = crate::TerminalGridPosition {
            line: 0,
            column: 12,
        };

        let bounds = crate::terminal_token_bounds(&lines, point);
        assert!(bounds.is_some());
        let (start, end) = bounds.expect("token bounds");
        let selection = crate::TerminalSelection {
            session_id: 1,
            anchor: start,
            head: end,
        };
        let selected = crate::terminal_selection_text(&lines, &selection);
        assert_eq!(selected, "https://example.com/path?q=1");
    }

    #[test]
    fn selection_text_spans_multiple_lines() {
        let lines = vec!["abc".to_owned(), "def".to_owned(), "ghi".to_owned()];
        let selection = crate::TerminalSelection {
            session_id: 1,
            anchor: crate::TerminalGridPosition { line: 0, column: 1 },
            head: crate::TerminalGridPosition { line: 2, column: 2 },
        };

        let selected = crate::terminal_selection_text(&lines, &selection);
        assert_eq!(selected, "bc\ndef\ngh");
    }

    #[test]
    fn line_bounds_capture_entire_line_on_triple_click() {
        let lines = vec!["hello world".to_owned()];
        let point = crate::TerminalGridPosition { line: 0, column: 3 };

        let bounds = crate::terminal_line_bounds(&lines, point);
        assert!(bounds.is_some());
        let (start, end) = bounds.expect("line bounds");
        assert_eq!(start.line, 0);
        assert_eq!(start.column, 0);
        assert_eq!(end.line, 0);
        assert_eq!(end.column, 11);
    }

    #[test]
    fn styled_lines_remap_embedded_default_palette_to_active_theme() {
        let theme = ThemeKind::Gruvbox.palette();
        let session = session_with_styled_line(
            "abc",
            crate::terminal_backend::EMBEDDED_TERMINAL_DEFAULT_FG,
            crate::terminal_backend::EMBEDDED_TERMINAL_DEFAULT_BG,
            None,
        );

        let lines = styled_lines_for_session(&session, theme, false, None, None);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0]
                .cells
                .iter()
                .all(|cell| cell.bg == theme.terminal_bg)
        );
        assert!(
            lines[0]
                .cells
                .iter()
                .all(|cell| cell.fg == theme.text_primary)
        );
    }

    #[test]
    fn extract_first_url_ignores_punctuation() {
        let url = extract_first_url("created PR: https://github.com/acme/repo/pull/42.");
        assert_eq!(url.as_deref(), Some("https://github.com/acme/repo/pull/42"));
    }

    #[test]
    fn issue_markdown_to_text_produces_readable_plain_text() {
        let markdown = r#"# Summary

- [x] shipped **bold** change
- see [docs](https://example.com/docs)

> quoted _note_

```rs
let answer = 42;
```
"#;

        let plain_text = crate::issue_markdown_to_text(markdown);
        assert_eq!(
            plain_text,
            "Summary\n\nshipped bold change\nsee docs (https://example.com/docs)\n\nquoted note\n\nlet answer = 42;"
        );
    }

    #[test]
    fn issue_body_text_falls_back_when_body_is_missing_or_empty() {
        assert_eq!(
            crate::issue_body_text(None),
            crate::ISSUE_DESCRIPTION_FALLBACK
        );
        assert_eq!(
            crate::issue_body_text(Some("   \n\n")),
            crate::ISSUE_DESCRIPTION_FALLBACK
        );
    }

    #[test]
    fn github_token_resolution_prefers_saved_token() {
        let token =
            resolve_github_access_token_from_sources(Some(" saved-token "), Some("env-token"));
        assert_eq!(token.as_deref(), Some("saved-token"));
    }

    #[test]
    fn github_token_resolution_falls_back_to_environment_token() {
        let token = resolve_github_access_token_from_sources(Some(""), Some(" env-token "));
        assert_eq!(token.as_deref(), Some("env-token"));
    }

    #[test]
    fn preserve_cached_pr_data_only_when_rate_limited_without_fresh_pr_data() {
        let pr = crate::github_service::PrDetails {
            number: 42,
            title: "Keep the old PR metadata".to_owned(),
            url: "https://github.com/penso/arbor/pull/42".to_owned(),
            state: crate::github_service::PrState::Open,
            additions: 1,
            deletions: 1,
            review_decision: crate::github_service::ReviewDecision::Pending,
            mergeable: crate::github_service::MergeableState::Mergeable,
            merge_state_status: crate::github_service::MergeStateStatus::Clean,
            passed_checks: 0,
            checks_status: crate::github_service::CheckStatus::Pending,
            checks: Vec::new(),
        };

        assert!(crate::should_preserve_cached_pr_data_on_rate_limit(
            None,
            None,
            None,
            Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(60)),
        ));
        assert!(!crate::should_preserve_cached_pr_data_on_rate_limit(
            Some(pr.number),
            Some(pr.url.as_str()),
            Some(&pr),
            Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(60)),
        ));
        assert!(!crate::should_preserve_cached_pr_data_on_rate_limit(
            None, None, None, None,
        ));
    }

    #[test]
    fn clear_pull_request_data_for_untracked_worktrees_only_clears_stale_rows() {
        let mut tracked = sample_worktree_summary();
        tracked.pr_number = Some(7);
        tracked.pr_url = Some("https://github.com/penso/arbor/pull/7".to_owned());
        tracked.pr_details = Some(crate::github_service::PrDetails {
            number: 7,
            title: "Tracked".to_owned(),
            url: "https://github.com/penso/arbor/pull/7".to_owned(),
            state: crate::github_service::PrState::Open,
            additions: 1,
            deletions: 1,
            review_decision: crate::github_service::ReviewDecision::Pending,
            mergeable: crate::github_service::MergeableState::Mergeable,
            merge_state_status: crate::github_service::MergeStateStatus::Clean,
            passed_checks: 0,
            checks_status: crate::github_service::CheckStatus::Pending,
            checks: Vec::new(),
        });

        let mut stale = sample_worktree_summary();
        stale.path = "/tmp/repo/wt-stale".into();
        stale.label = "wt-stale".to_owned();
        stale.branch = "main".to_owned();
        stale.pr_number = Some(8);
        stale.pr_url = Some("https://github.com/penso/arbor/pull/8".to_owned());
        stale.pr_details = Some(crate::github_service::PrDetails {
            number: 8,
            title: "Stale".to_owned(),
            url: "https://github.com/penso/arbor/pull/8".to_owned(),
            state: crate::github_service::PrState::Open,
            additions: 1,
            deletions: 1,
            review_decision: crate::github_service::ReviewDecision::Pending,
            mergeable: crate::github_service::MergeableState::Mergeable,
            merge_state_status: crate::github_service::MergeStateStatus::Clean,
            passed_checks: 0,
            checks_status: crate::github_service::CheckStatus::Pending,
            checks: Vec::new(),
        });

        let tracked_path = tracked.path.clone();
        let mut worktrees = vec![tracked, stale];
        let tracked_paths = HashSet::from([tracked_path]);

        assert!(crate::clear_pull_request_data_for_untracked_worktrees(
            &mut worktrees,
            &tracked_paths,
        ));
        assert_eq!(worktrees[0].pr_number, Some(7));
        assert!(worktrees[0].pr_details.is_some());
        assert_eq!(worktrees[1].pr_number, None);
        assert_eq!(worktrees[1].pr_url, None);
        assert_eq!(worktrees[1].pr_details, None);
    }

    #[test]
    fn parse_connect_host_target_normalizes_bare_http_host() {
        let target = crate::parse_connect_host_target("10.0.0.5")
            .expect("bare host should parse as http daemon target");

        match target {
            crate::ConnectHostTarget::Http { url, auth_key } => {
                assert_eq!(url, "http://10.0.0.5:8787");
                assert_eq!(auth_key, url);
            },
            crate::ConnectHostTarget::Ssh { .. } => panic!("expected http target"),
        }
    }

    #[test]
    fn parse_connect_host_target_supports_ssh_scheme() {
        let target = crate::parse_connect_host_target("ssh://dev@example.com:2222/9001")
            .expect("ssh address should parse");

        match target {
            crate::ConnectHostTarget::Ssh { target, auth_key } => {
                assert_eq!(target.user.as_deref(), Some("dev"));
                assert_eq!(target.host, "example.com");
                assert_eq!(target.ssh_port, 2222);
                assert_eq!(target.daemon_port, 9001);
                assert_eq!(auth_key, "ssh://dev@example.com:2222/9001");
            },
            crate::ConnectHostTarget::Http { .. } => panic!("expected ssh target"),
        }
    }

    #[test]
    fn parse_launch_mode_supports_daemon_bind() {
        let mode = crate::parse_launch_mode(vec![
            "--daemon".to_owned(),
            "--bind".to_owned(),
            "0.0.0.0:8787".to_owned(),
        ])
        .expect("daemon args should parse");

        match mode {
            crate::LaunchMode::Daemon { bind_addr } => {
                assert_eq!(bind_addr.as_deref(), Some("0.0.0.0:8787"));
            },
            crate::LaunchMode::Gui => panic!("expected daemon launch mode"),
            crate::LaunchMode::Help => panic!("expected daemon launch mode"),
        }
    }

    #[test]
    fn pending_save_coalesces_to_latest_value_after_inflight_write() {
        let mut pending = PendingSave::default();

        pending.queue("first");
        assert_eq!(pending.begin_next(), Some("first"));
        assert!(pending.has_work());

        pending.queue("second");
        pending.queue("third");
        assert!(pending.begin_next().is_none());

        pending.finish();

        assert_eq!(pending.begin_next(), Some("third"));
        pending.finish();
        assert!(!pending.has_work());
    }

    #[test]
    fn pending_save_reports_work_for_pending_and_inflight_states() {
        let mut pending = PendingSave::default();
        assert!(!pending.has_work());

        pending.queue(1_u8);
        assert!(pending.has_work());

        let _ = pending.begin_next();
        assert!(pending.has_work());

        pending.finish();
        assert!(!pending.has_work());
    }

    #[test]
    fn ui_state_save_has_work_for_pending_and_inflight_states() {
        let state = ui_state_store::UiState::default();

        assert!(!crate::ui_state_save_has_work(None, None));
        assert!(crate::ui_state_save_has_work(Some(&state), None));
        assert!(crate::ui_state_save_has_work(None, Some(&state)));
    }

    #[test]
    fn next_pending_ui_state_save_keeps_reverted_state_queued_while_other_save_is_in_flight() {
        let persisted = ui_state_store::UiState {
            left_pane_width: Some(240),
            ..ui_state_store::UiState::default()
        };
        let in_flight = ui_state_store::UiState {
            left_pane_width: Some(320),
            ..ui_state_store::UiState::default()
        };

        assert_eq!(
            crate::next_pending_ui_state_save(&persisted, None, Some(&in_flight), &persisted),
            Some(persisted),
        );
    }

    #[test]
    fn next_pending_ui_state_save_does_not_duplicate_inflight_state() {
        let state = ui_state_store::UiState {
            left_pane_width: Some(320),
            ..ui_state_store::UiState::default()
        };

        assert_eq!(
            crate::next_pending_ui_state_save(
                &ui_state_store::UiState::default(),
                None,
                Some(&state),
                &state,
            ),
            None,
        );
    }

    #[test]
    fn managed_process_title_round_trips_to_process_id() {
        let worktree_path = Path::new("/tmp/repo");
        assert_eq!(
            crate::managed_process_id_from_title(
                worktree_path,
                &crate::managed_process_title(ProcessSource::Procfile, "web"),
            ),
            Some(crate::managed_process_id(
                ProcessSource::Procfile,
                worktree_path,
                "web",
            ))
        );
        assert_eq!(
            crate::managed_process_id_from_title(
                worktree_path,
                &crate::managed_process_title(ProcessSource::ArborToml, "worker"),
            ),
            Some(crate::managed_process_id(
                ProcessSource::ArborToml,
                worktree_path,
                "worker",
            ))
        );
    }

    #[test]
    fn managed_processes_for_primary_worktree_include_arbor_toml_processes() {
        let unique_suffix = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos(),
            Err(error) => panic!("current time should be after the unix epoch: {error}"),
        };
        let repo_root = env::temp_dir().join(format!("arbor-managed-processes-{unique_suffix}"));
        let linked_worktree = repo_root.join("worktrees").join("feature");

        if let Err(error) = fs::create_dir_all(&linked_worktree) {
            panic!("linked worktree dir should be created: {error}");
        }
        if let Err(error) = fs::write(
            repo_root.join("arbor.toml"),
            "[[processes]]\nname = \"worker\"\ncommand = \"cargo run -- worker\"\nworking_dir = \"backend\"\n",
        ) {
            panic!("arbor.toml should be written: {error}");
        }

        let primary_processes = crate::managed_processes_for_worktree(&repo_root, &repo_root);
        assert!(primary_processes.iter().any(|process| {
            process.source == ProcessSource::ArborToml
                && process.name == "worker"
                && process.working_dir == repo_root.join("backend")
        }));

        let linked_processes = crate::managed_processes_for_worktree(&repo_root, &linked_worktree);
        assert!(
            !linked_processes
                .iter()
                .any(|process| process.source == ProcessSource::ArborToml)
        );

        if let Err(error) = fs::remove_dir_all(&repo_root) {
            panic!("temp repo root should be removed: {error}");
        }
    }

    #[test]
    fn completed_managed_process_sessions_are_not_active() {
        let mut session = session_with_styled_line("", 0xffffff, 0x000000, None);
        session.managed_process_id = Some("procfile:/tmp/worktree:web".to_owned());
        session.is_initializing = false;
        session.state = TerminalState::Completed;
        assert!(!crate::managed_process_session_is_active(&session));

        session.state = TerminalState::Running;
        assert!(crate::managed_process_session_is_active(&session));

        session.is_initializing = true;
        session.state = TerminalState::Completed;
        assert!(crate::managed_process_session_is_active(&session));
    }
}
