mod terminal_backend;
mod theme;

use {
    arbor_core::{
        changes::{self, ChangeKind, ChangedFile},
        worktree,
    },
    gpui::{
        App, Application, Bounds, Context, Div, ElementId, FocusHandle, FontWeight, KeyBinding,
        KeyDownEvent, Keystroke, Menu, MenuItem, MouseButton, MouseDownEvent, ScrollHandle,
        Stateful, SystemMenuType, TitlebarOptions, Window, WindowBounds, WindowControlArea,
        WindowDecorations, WindowOptions, actions, div, point, prelude::*, px, rgb, size,
    },
    std::{
        env, fs,
        path::{Path, PathBuf},
        sync::Mutex,
        time::{Duration, Instant},
    },
    terminal_backend::{
        EmbeddedTerminal, TerminalBackendKind, TerminalLaunch, TerminalStyledLine,
        TerminalStyledRun,
    },
    theme::{ThemeKind, ThemePalette},
};

const FONT_UI: &str = ".ZedSans";
const FONT_MONO: &str = "Menlo";

const TITLEBAR_HEIGHT: f32 = 34.;
const TRAFFIC_LIGHT_PADDING: f32 = 71.;
const QUIT_ARM_WINDOW: Duration = Duration::from_millis(1200);

static QUIT_ARMED_AT: Mutex<Option<Instant>> = Mutex::new(None);

actions!(arbor, [
    RequestQuit,
    SpawnTerminal,
    RefreshWorktrees,
    RefreshChanges,
    OpenCreateWorktree,
    UseOneDarkTheme,
    UseAyuDarkTheme,
    UseEmbeddedBackend,
    UseAlacrittyBackend,
    UseGhosttyBackend
]);

#[derive(Debug, Clone)]
struct WorktreeSummary {
    path: PathBuf,
    label: String,
    branch: String,
    state: String,
    diff_summary: Option<changes::DiffLineSummary>,
}

#[derive(Clone)]
struct TerminalSession {
    id: u64,
    title: String,
    command: String,
    state: TerminalState,
    output: String,
    styled_output: Vec<TerminalStyledLine>,
    runtime: Option<EmbeddedTerminal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalState {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateWorktreeField {
    RepositoryPath,
    WorktreeName,
}

#[derive(Debug, Clone)]
struct CreateWorktreeModal {
    repository_path: String,
    worktree_name: String,
    active_field: CreateWorktreeField,
    is_creating: bool,
    error: Option<String>,
}

enum ModalInputEvent {
    SetActiveField(CreateWorktreeField),
    MoveActiveField(bool),
    Backspace,
    Append(String),
    ClearError,
}

struct CreatedWorktree {
    worktree_name: String,
    branch_name: String,
    worktree_path: PathBuf,
}

struct ArborWindow {
    repo_root: PathBuf,
    worktrees: Vec<WorktreeSummary>,
    worktree_stats_loading: bool,
    active_worktree_index: Option<usize>,
    changed_files: Vec<ChangedFile>,
    terminals: Vec<TerminalSession>,
    active_terminal_id: Option<u64>,
    next_terminal_id: u64,
    active_backend_kind: TerminalBackendKind,
    theme_kind: ThemeKind,
    terminal_focus: FocusHandle,
    terminal_scroll_handle: ScrollHandle,
    create_worktree_modal: Option<CreateWorktreeModal>,
    notice: Option<String>,
}

impl ArborWindow {
    fn load(cx: &mut Context<Self>) -> Self {
        let cwd = match env::current_dir() {
            Ok(path) => path,
            Err(error) => {
                return Self {
                    repo_root: PathBuf::from("."),
                    worktrees: Vec::new(),
                    worktree_stats_loading: false,
                    active_worktree_index: None,
                    changed_files: Vec::new(),
                    terminals: Vec::new(),
                    active_terminal_id: None,
                    next_terminal_id: 1,
                    active_backend_kind: TerminalBackendKind::Embedded,
                    theme_kind: ThemeKind::OneDark,
                    terminal_focus: cx.focus_handle(),
                    terminal_scroll_handle: ScrollHandle::new(),
                    create_worktree_modal: None,
                    notice: Some(format!("failed to read current directory: {error}")),
                };
            },
        };

        let repo_root = match worktree::repo_root(&cwd) {
            Ok(path) => path,
            Err(error) => {
                return Self {
                    repo_root: cwd,
                    worktrees: Vec::new(),
                    worktree_stats_loading: false,
                    active_worktree_index: None,
                    changed_files: Vec::new(),
                    terminals: Vec::new(),
                    active_terminal_id: None,
                    next_terminal_id: 1,
                    active_backend_kind: TerminalBackendKind::Embedded,
                    theme_kind: ThemeKind::OneDark,
                    terminal_focus: cx.focus_handle(),
                    terminal_scroll_handle: ScrollHandle::new(),
                    create_worktree_modal: None,
                    notice: Some(format!("failed to resolve git repository root: {error}")),
                };
            },
        };

        let mut app = Self {
            repo_root,
            worktrees: Vec::new(),
            worktree_stats_loading: false,
            active_worktree_index: None,
            changed_files: Vec::new(),
            terminals: Vec::new(),
            active_terminal_id: None,
            next_terminal_id: 1,
            active_backend_kind: TerminalBackendKind::Embedded,
            theme_kind: ThemeKind::OneDark,
            terminal_focus: cx.focus_handle(),
            terminal_scroll_handle: ScrollHandle::new(),
            create_worktree_modal: None,
            notice: None,
        };

        app.refresh_worktrees(cx);
        let _ = app.spawn_terminal_session_inner(false);
        app.start_terminal_poller(cx);
        app
    }

    fn start_terminal_poller(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_spawn(async move {
                    std::thread::sleep(Duration::from_millis(45));
                })
                .await;

                let updated = this.update(cx, |this, cx| this.sync_running_terminals(cx));
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn sync_running_terminals(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let follow_output = terminal_scroll_is_near_bottom(&self.terminal_scroll_handle);

        for session in &mut self.terminals {
            let Some(runtime) = session.runtime.as_ref() else {
                continue;
            };

            let snapshot = runtime.snapshot();
            let output = snapshot.output;
            let styled_output = snapshot.styled_lines;
            if output != session.output || styled_output != session.styled_output {
                session.output = output;
                session.styled_output = styled_output;
                changed = true;
            }

            if let Some(exit_code) = snapshot.exit_code
                && session.state == TerminalState::Running
            {
                session.state = if exit_code == 0 {
                    TerminalState::Completed
                } else {
                    TerminalState::Failed
                };
                session.runtime = None;
                changed = true;

                if exit_code != 0 {
                    self.notice = Some(format!(
                        "terminal tab `{}` exited with code {exit_code}",
                        session.title,
                    ));
                }
            }
        }

        if changed {
            if should_auto_follow_terminal_output(changed, follow_output) {
                self.terminal_scroll_handle.scroll_to_bottom();
            }
            cx.notify();
        }
    }

    fn refresh_worktrees(&mut self, cx: &mut Context<Self>) {
        let previously_selected = self.selected_worktree_path().map(Path::to_path_buf);

        match worktree::list(&self.repo_root) {
            Ok(entries) => {
                self.worktrees = entries.iter().map(WorktreeSummary::from_worktree).collect();
                self.worktree_stats_loading = true;
                self.active_worktree_index = previously_selected
                    .and_then(|path| {
                        self.worktrees
                            .iter()
                            .position(|worktree| worktree.path == path)
                    })
                    .or_else(|| (!self.worktrees.is_empty()).then_some(0));
                self.notice = None;
                self.refresh_worktree_diff_summaries(cx);
                self.reload_changed_files();
            },
            Err(error) => {
                self.worktree_stats_loading = false;
                self.notice = Some(format!("failed to refresh worktrees: {error}"));
            },
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
                for (path, summary_result) in summaries {
                    if let Some(worktree) = this
                        .worktrees
                        .iter_mut()
                        .find(|worktree| worktree.path == path)
                    {
                        worktree.diff_summary = summary_result.ok();
                    }
                }
                this.worktree_stats_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn selected_worktree_path(&self) -> Option<&Path> {
        self.active_worktree_index
            .and_then(|index| self.worktrees.get(index))
            .map(|worktree| worktree.path.as_path())
    }

    fn active_worktree(&self) -> Option<&WorktreeSummary> {
        self.active_worktree_index
            .and_then(|index| self.worktrees.get(index))
    }

    fn active_backend_descriptor(&self) -> terminal_backend::TerminalBackendDescriptor {
        terminal_backend::descriptor_for_kind(self.active_backend_kind)
    }

    fn theme(&self) -> ThemePalette {
        self.theme_kind.palette()
    }

    fn select_worktree(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.active_worktree_index == Some(index) {
            return;
        }

        self.active_worktree_index = Some(index);
        self.reload_changed_files();
        cx.notify();
    }

    fn reload_changed_files(&mut self) {
        let Some(path) = self.selected_worktree_path() else {
            self.changed_files.clear();
            return;
        };

        match changes::changed_files(path) {
            Ok(files) => {
                self.changed_files = files;
                self.notice = None;
            },
            Err(error) => {
                self.changed_files.clear();
                self.notice = Some(format!("failed to load changed files with gix: {error}"));
            },
        }
    }

    fn switch_terminal_backend(
        &mut self,
        backend_kind: TerminalBackendKind,
        cx: &mut Context<Self>,
    ) {
        if self.active_backend_kind == backend_kind {
            return;
        }

        self.active_backend_kind = backend_kind;
        self.notice = None;
        cx.notify();
    }

    fn switch_theme(&mut self, theme_kind: ThemeKind, cx: &mut Context<Self>) {
        if self.theme_kind == theme_kind {
            return;
        }

        self.theme_kind = theme_kind;
        self.notice = Some(format!("theme switched to {}", theme_kind.label()));
        cx.notify();
    }

    fn open_create_worktree_modal(&mut self, cx: &mut Context<Self>) {
        self.create_worktree_modal = Some(CreateWorktreeModal {
            repository_path: self.repo_root.display().to_string(),
            worktree_name: String::new(),
            active_field: CreateWorktreeField::WorktreeName,
            is_creating: false,
            error: None,
        });
        cx.notify();
    }

    fn close_create_worktree_modal(&mut self, cx: &mut Context<Self>) {
        self.create_worktree_modal = None;
        cx.notify();
    }

    fn update_create_worktree_modal_input(
        &mut self,
        input: ModalInputEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.create_worktree_modal.as_mut() else {
            return;
        };

        if modal.is_creating {
            return;
        }

        match input {
            ModalInputEvent::SetActiveField(field) => {
                modal.active_field = field;
            },
            ModalInputEvent::MoveActiveField(reverse) => {
                modal.active_field = match (modal.active_field, reverse) {
                    (CreateWorktreeField::RepositoryPath, false) => {
                        CreateWorktreeField::WorktreeName
                    },
                    (CreateWorktreeField::WorktreeName, false) => {
                        CreateWorktreeField::RepositoryPath
                    },
                    (CreateWorktreeField::RepositoryPath, true) => {
                        CreateWorktreeField::WorktreeName
                    },
                    (CreateWorktreeField::WorktreeName, true) => {
                        CreateWorktreeField::RepositoryPath
                    },
                };
            },
            ModalInputEvent::Backspace => {
                let field_value = match modal.active_field {
                    CreateWorktreeField::RepositoryPath => &mut modal.repository_path,
                    CreateWorktreeField::WorktreeName => &mut modal.worktree_name,
                };
                let _ = field_value.pop();
            },
            ModalInputEvent::Append(text) => {
                let field_value = match modal.active_field {
                    CreateWorktreeField::RepositoryPath => &mut modal.repository_path,
                    CreateWorktreeField::WorktreeName => &mut modal.worktree_name,
                };
                field_value.push_str(&text);
            },
            ModalInputEvent::ClearError => {
                modal.error = None;
            },
        }

        cx.notify();
    }

    fn submit_create_worktree_modal(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.create_worktree_modal.as_mut() else {
            return;
        };
        if modal.is_creating {
            return;
        }

        modal.error = None;
        let repository_input = modal.repository_path.trim().to_owned();
        let worktree_input = modal.worktree_name.trim().to_owned();

        if repository_input.is_empty() {
            modal.error = Some("Repository path is required.".to_owned());
            cx.notify();
            return;
        }

        if worktree_input.is_empty() {
            modal.error = Some("Worktree name is required.".to_owned());
            cx.notify();
            return;
        }

        modal.is_creating = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let creation = cx
                .background_spawn(async move {
                    create_managed_worktree(repository_input, worktree_input)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                match creation {
                    Ok(created) => {
                        this.notice = Some(format!(
                            "created worktree `{}` on branch `{}`",
                            created.worktree_name, created.branch_name
                        ));
                        this.create_worktree_modal = None;
                        this.refresh_worktrees(cx);
                        if let Some(index) = this
                            .worktrees
                            .iter()
                            .position(|worktree| worktree.path == created.worktree_path)
                        {
                            this.active_worktree_index = Some(index);
                            this.reload_changed_files();
                        }
                    },
                    Err(error) => {
                        if let Some(modal) = this.create_worktree_modal.as_mut() {
                            modal.is_creating = false;
                            modal.error = Some(error);
                        } else {
                            this.notice = Some(error);
                        }
                    },
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn handle_global_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_worktree_modal.is_none() || event.is_held {
            return;
        }

        if event.keystroke.modifiers.platform {
            return;
        }

        match event.keystroke.key.as_str() {
            "escape" => {
                self.close_create_worktree_modal(cx);
                cx.stop_propagation();
                return;
            },
            "tab" => {
                self.update_create_worktree_modal_input(
                    ModalInputEvent::MoveActiveField(event.keystroke.modifiers.shift),
                    cx,
                );
                cx.stop_propagation();
                return;
            },
            "enter" | "return" => {
                self.submit_create_worktree_modal(cx);
                cx.stop_propagation();
                return;
            },
            "backspace" => {
                self.update_create_worktree_modal_input(ModalInputEvent::Backspace, cx);
                cx.stop_propagation();
                return;
            },
            _ => {},
        }

        if event.keystroke.modifiers.control || event.keystroke.modifiers.alt {
            return;
        }

        if let Some(key_char) = event.keystroke.key_char.as_ref() {
            self.update_create_worktree_modal_input(ModalInputEvent::ClearError, cx);
            self.update_create_worktree_modal_input(
                ModalInputEvent::Append(key_char.to_owned()),
                cx,
            );
            cx.stop_propagation();
        }
    }

    fn action_open_create_worktree(
        &mut self,
        _: &OpenCreateWorktree,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_create_worktree_modal(cx);
    }

    fn action_spawn_terminal(
        &mut self,
        _: &SpawnTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_terminal_session(window, cx);
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
        self.reload_changed_files();
        cx.notify();
    }

    fn action_use_one_dark_theme(
        &mut self,
        _: &UseOneDarkTheme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_theme(ThemeKind::OneDark, cx);
    }

    fn action_use_ayu_dark_theme(
        &mut self,
        _: &UseAyuDarkTheme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_theme(ThemeKind::AyuDark, cx);
    }

    fn action_use_embedded_backend(
        &mut self,
        _: &UseEmbeddedBackend,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_terminal_backend(TerminalBackendKind::Embedded, cx);
    }

    fn action_use_alacritty_backend(
        &mut self,
        _: &UseAlacrittyBackend,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_terminal_backend(TerminalBackendKind::Alacritty, cx);
    }

    fn action_use_ghostty_backend(
        &mut self,
        _: &UseGhosttyBackend,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_terminal_backend(TerminalBackendKind::Ghostty, cx);
    }

    fn spawn_terminal_session_inner(&mut self, show_notice_on_missing_worktree: bool) -> bool {
        let Some(cwd) = self.selected_worktree_path().map(Path::to_path_buf) else {
            if show_notice_on_missing_worktree {
                self.notice = Some("select a worktree before opening a terminal tab".to_owned());
            }
            return false;
        };

        let backend_kind = self.active_backend_kind;
        let session_id = self.next_terminal_id;
        self.next_terminal_id += 1;
        self.active_terminal_id = Some(session_id);

        let mut session = TerminalSession {
            id: session_id,
            title: format!("term-{session_id}"),
            command: String::new(),
            state: TerminalState::Running,
            output: String::new(),
            styled_output: Vec::new(),
            runtime: None,
        };

        match terminal_backend::launch_backend(backend_kind, &cwd) {
            Ok(TerminalLaunch::Embedded(runtime)) => {
                session.command = "embedded shell".to_owned();
                session.runtime = Some(runtime);
                session.output = String::new();
                session.styled_output = Vec::new();
            },
            Ok(TerminalLaunch::External(result)) => {
                session.command = result.command;
                session.output = trim_to_last_lines(result.output, 120);
                session.styled_output = Vec::new();
                session.state = if result.success {
                    TerminalState::Completed
                } else {
                    TerminalState::Failed
                };
                if !result.success {
                    self.notice = Some(format!(
                        "terminal backend launch failed with code {:?}",
                        result.code,
                    ));
                }
            },
            Err(error) => {
                session.command = "launch backend".to_owned();
                session.output = error.clone();
                session.styled_output = Vec::new();
                session.state = TerminalState::Failed;
                self.notice = Some(format!("terminal session failed: {error}"));
            },
        }

        self.terminals.push(session);
        true
    }

    fn spawn_terminal_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.spawn_terminal_session_inner(true) {
            cx.notify();
            return;
        }

        self.terminal_scroll_handle.scroll_to_bottom();
        window.focus(&self.terminal_focus);
        cx.notify();
    }

    fn select_terminal(&mut self, session_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_terminal_id == Some(session_id) {
            window.focus(&self.terminal_focus);
            return;
        }

        self.active_terminal_id = Some(session_id);
        self.terminal_scroll_handle.scroll_to_bottom();
        window.focus(&self.terminal_focus);
        cx.notify();
    }

    fn active_terminal(&self) -> Option<&TerminalSession> {
        let session_id = self.active_terminal_id?;
        self.terminals
            .iter()
            .find(|session| session.id == session_id)
    }

    fn active_terminal_runtime(&self) -> Option<EmbeddedTerminal> {
        self.active_terminal()
            .and_then(|session| session.runtime.clone())
    }

    fn handle_terminal_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_worktree_modal.is_some() {
            return;
        }

        if event.keystroke.modifiers.platform {
            return;
        }

        if event.is_held {
            return;
        }

        let Some(runtime) = self.active_terminal_runtime() else {
            return;
        };

        let Some(input) = terminal_bytes_from_keystroke(&event.keystroke) else {
            return;
        };

        if let Err(error) = runtime.write_input(&input) {
            self.notice = Some(format!("failed to write to terminal: {error}"));
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn focus_terminal_panel(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.focus(&self.terminal_focus);
    }

    fn render_top_bar(&self) -> impl IntoElement {
        let theme = self.theme();
        let repo_name = self
            .repo_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.repo_root.display().to_string());
        let branch = self
            .active_worktree()
            .map(|worktree| worktree.branch.clone())
            .unwrap_or_else(|| "no-worktree".to_owned());
        let leading_padding = if cfg!(target_os = "macos") {
            px(TRAFFIC_LIGHT_PADDING)
        } else {
            px(12.)
        };

        div()
            .h(px(TITLEBAR_HEIGHT))
            .bg(rgb(theme.chrome_bg))
            .window_control_area(WindowControlArea::Drag)
            .pl(leading_padding)
            .pr_3()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme.text_muted))
                            .child("arbor"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme.text_primary))
                            .child(repo_name),
                    )
                    .child(status_chip(theme, "branch", branch)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme.text_muted))
                    .child(format!(
                        "backend {}",
                        self.active_backend_descriptor().label
                    )),
            )
    }

    fn render_left_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        div()
            .w(px(300.))
            .h_full()
            .bg(rgb(theme.sidebar_bg))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(36.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Workspaces"),
                    )
                    .child(
                        action_button(theme, "refresh-worktrees", "Refresh", false, false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_worktrees(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(div().h(px(1.)).bg(rgb(theme.border)))
            .child(
                div()
                    .id("worktrees-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_2()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(self.worktrees.iter().enumerate().map(|(index, worktree)| {
                        let is_active = self.active_worktree_index == Some(index);
                        let diff_summary = worktree.diff_summary;
                        div()
                            .id(("worktree-row", index))
                            .cursor_pointer()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(if is_active {
                                theme.accent
                            } else {
                                theme.border
                            }))
                            .bg(rgb(theme.panel_bg))
                            .p_2()
                            .when(is_active, |this| this.bg(rgb(theme.panel_active_bg)))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_worktree(index, cx)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(theme.text_muted))
                                                    .child("⎇"),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(worktree.label.clone()),
                                            ),
                                    )
                                    .child({
                                        if self.worktree_stats_loading && diff_summary.is_none() {
                                            div()
                                                .text_xs()
                                                .text_color(rgb(theme.text_muted))
                                                .child("loading...")
                                        } else if let Some(summary) = diff_summary {
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0x72d69c))
                                                        .child(format!("+{}", summary.additions)),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0xeb6f92))
                                                        .child(format!("-{}", summary.deletions)),
                                                )
                                        } else {
                                            div()
                                                .text_xs()
                                                .text_color(rgb(theme.text_muted))
                                                .child("+0 -0")
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme.text_muted))
                                    .child(worktree.branch.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme.text_disabled))
                                    .child(worktree.state.clone()),
                            )
                    })),
            )
            .child(div().h(px(1.)).bg(rgb(theme.border)))
            .child(
                div().h(px(36.)).px_3().flex().items_center().child(
                    action_button(
                        theme,
                        "open-create-worktree",
                        "+ Add Worktree",
                        false,
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_create_worktree_modal(cx);
                        cx.notify();
                    })),
                ),
            )
    }

    fn render_terminal_panel(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme();
        let active_terminal = self.active_terminal().cloned();
        let active_terminal_index = self
            .active_terminal_id
            .and_then(|session_id| self.terminals.iter().position(|term| term.id == session_id));
        let terminal_is_focused = self.terminal_focus.is_focused(window);

        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(rgb(theme.center_bg))
            .border_l_1()
            .border_r_1()
            .border_color(rgb(if terminal_is_focused {
                theme.accent
            } else {
                theme.border
            }))
            .flex()
            .flex_col()
            .track_focus(&self.terminal_focus)
            .on_any_mouse_down(cx.listener(Self::focus_terminal_panel))
            .on_key_down(cx.listener(Self::handle_terminal_key_down))
            .child(
                div()
                    .h(px(32.))
                    .bg(rgb(theme.tab_bg))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .h_full()
                            .flex_1()
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .when(self.terminals.is_empty(), |this| {
                                this.child(
                                    div()
                                        .px_3()
                                        .text_xs()
                                        .text_color(rgb(theme.text_muted))
                                        .child("No terminal tabs"),
                                )
                            })
                            .children(self.terminals.iter().enumerate().map(|(index, session)| {
                                let is_active = self.active_terminal_id == Some(session.id);
                                let session_id = session.id;
                                let terminal_count = self.terminals.len();
                                let relation = active_terminal_index
                                    .map(|active_index| index.cmp(&active_index));

                                div()
                                    .id(("terminal-tab", session.id))
                                    .h_full()
                                    .cursor_pointer()
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .border_color(rgb(theme.border))
                                    .bg(rgb(if is_active {
                                        theme.tab_active_bg
                                    } else {
                                        theme.tab_bg
                                    }))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(theme.text_muted))
                                            .child("▸"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(if is_active {
                                                theme.text_primary
                                            } else {
                                                theme.text_muted
                                            }))
                                            .child(session.title.clone()),
                                    )
                                    .when(index == 0, |this| this.border_l_1())
                                    .when(index + 1 == terminal_count, |this| this.border_r_1())
                                    .map(|this| match relation {
                                        Some(std::cmp::Ordering::Equal) => {
                                            this.border_l_1().border_r_1()
                                        },
                                        Some(std::cmp::Ordering::Less) => {
                                            this.border_l_1().border_b_1()
                                        },
                                        Some(std::cmp::Ordering::Greater) => {
                                            this.border_r_1().border_b_1()
                                        },
                                        None => this.border_b_1(),
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select_terminal(session_id, window, cx)
                                    }))
                            })),
                    )
                    .child(
                        div()
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if terminal_is_focused {
                                        theme.accent
                                    } else {
                                        theme.text_disabled
                                    }))
                                    .child(if terminal_is_focused {
                                        "INPUT"
                                    } else {
                                        "CLICK TO TYPE"
                                    }),
                            )
                            .gap_1()
                            .px_2()
                            .border_l_1()
                            .border_color(rgb(theme.border))
                            .border_b_1()
                            .child(
                                div()
                                    .id("terminal-tab-new")
                                    .size(px(20.))
                                    .cursor_pointer()
                                    .rounded_sm()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(rgb(theme.text_muted))
                                    .child("+")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                            this.spawn_terminal_session(window, cx)
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("terminal-tab-split")
                                    .size(px(20.))
                                    .cursor_pointer()
                                    .rounded_sm()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_xs()
                                    .text_color(rgb(theme.text_muted))
                                    .child("◫")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                            this.notice =
                                                Some("Split pane will be wired next.".to_owned());
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("terminal-tab-zoom")
                                    .size(px(20.))
                                    .cursor_pointer()
                                    .rounded_sm()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_xs()
                                    .text_color(rgb(theme.text_muted))
                                    .child("↗")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                            this.notice = Some(
                                                "Zoom for terminal pane is pending.".to_owned(),
                                            );
                                            cx.notify();
                                        }),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(rgb(theme.center_bg))
                    .when(active_terminal.is_none(), |this| {
                        this.child(
                            div()
                                .h_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .text_center()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(theme.text_primary))
                                        .child("Terminal workspace"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(theme.text_muted))
                                        .child("Press Cmd-T to open a terminal tab."),
                                )
                                .child(
                                    action_button(
                                        theme,
                                        "spawn-terminal-empty-state",
                                        "Open Terminal Tab",
                                        false,
                                        false,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            this.spawn_terminal_session(window, cx)
                                        },
                                    )),
                                ),
                        )
                    })
                    .when_some(active_terminal, |this, session| {
                        let styled_lines =
                            styled_lines_for_session(&session, theme, terminal_is_focused);

                        this.child(
                            div()
                                .h_full()
                                .w_full()
                                .min_w_0()
                                .min_h_0()
                                .overflow_hidden()
                                .font_family(FONT_MONO)
                                .text_sm()
                                .px_2()
                                .pt_1()
                                .flex()
                                .flex_col()
                                .gap_0()
                                .child(
                                    div()
                                        .id("terminal-output-scroll")
                                        .flex_1()
                                        .w_full()
                                        .min_w_0()
                                        .min_h_0()
                                        .overflow_x_hidden()
                                        .overflow_y_scroll()
                                        .scrollbar_width(px(12.))
                                        .track_scroll(&self.terminal_scroll_handle)
                                        .child(
                                            div()
                                                .w_full()
                                                .min_w_0()
                                                .flex_none()
                                                .flex()
                                                .flex_col()
                                                .gap_0()
                                                .children(
                                                    styled_lines.into_iter().map(|line| {
                                                        render_terminal_line(line, theme)
                                                    }),
                                                ),
                                        ),
                                ),
                        )
                    }),
            )
    }

    fn render_center_pane(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(rgb(theme.app_bg))
            .flex()
            .flex_col()
            .child(self.render_terminal_panel(window, cx))
    }

    fn render_right_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        div()
            .w(px(340.))
            .h_full()
            .min_h_0()
            .bg(rgb(theme.sidebar_bg))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Changes"),
                    )
                    .child(
                        action_button(theme, "refresh-files", "Refresh", false, false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.reload_changed_files();
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(action_button(theme, "changes-tab", "CHANGES", true, false))
                    .child(action_button(theme, "files-tab", "FILES", false, true)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme.text_muted))
                    .child("Native git status via gix"),
            )
            .child(div().h(px(1.)).bg(rgb(theme.border)))
            .child(
                div()
                    .id("changes-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .scrollbar_width(px(10.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().when(self.changed_files.is_empty(), |this| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(rgb(theme.text_muted))
                                .child("No local changes in selected worktree."),
                        )
                    }))
                    .children(self.changed_files.iter().map(|change| {
                        let status_color = match change.kind {
                            ChangeKind::Added => 0xa6e3a1,
                            ChangeKind::Modified => 0xf9e2af,
                            ChangeKind::Removed => 0xf38ba8,
                            ChangeKind::Renamed => 0x89dceb,
                            ChangeKind::Copied => 0x74c7ec,
                            ChangeKind::TypeChange => 0xcba6f7,
                            ChangeKind::Conflict => 0xf38ba8,
                            ChangeKind::IntentToAdd => 0x94e2d5,
                        };

                        div()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(theme.border))
                            .bg(rgb(theme.panel_bg))
                            .p_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(status_color))
                                    .child(change_code(change.kind)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(theme.text_primary))
                                    .child(change.path.display().to_string()),
                            )
                    })),
            )
    }

    fn render_status_bar(&self) -> impl IntoElement {
        let theme = self.theme();
        let repo_name = self
            .repo_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.repo_root.display().to_string());
        let worktree = self
            .active_worktree()
            .map(|entry| entry.label.clone())
            .unwrap_or_else(|| "none".to_owned());
        let backend = self.active_backend_descriptor().label;

        div()
            .h(px(26.))
            .bg(rgb(theme.chrome_bg))
            .border_t_1()
            .border_color(rgb(theme.chrome_border))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_text(theme, "●"))
                    .child(status_text(theme, format!("repo {repo_name}")))
                    .child(status_text(theme, "•"))
                    .child(status_text(theme, format!("worktree {worktree}"))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_text(theme, format!("backend {backend}")))
                    .child(status_text(theme, "•"))
                    .child(status_text(
                        theme,
                        format!("changes {}", self.changed_files.len()),
                    ))
                    .child(status_text(theme, "•"))
                    .child(status_text(
                        theme,
                        format!("terminals {}", self.terminals.len()),
                    ))
                    .child(status_text(theme, "ready")),
            )
    }

    fn render_create_worktree_modal(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(modal) = self.create_worktree_modal.clone() else {
            return div();
        };

        let theme = self.theme();
        let branch_name = derive_branch_name(&modal.worktree_name);
        let target_path_preview =
            preview_managed_worktree_path(modal.repository_path.trim(), modal.worktree_name.trim())
                .unwrap_or_else(|_| "-".to_owned());

        let repository_active = modal.active_field == CreateWorktreeField::RepositoryPath;
        let worktree_active = modal.active_field == CreateWorktreeField::WorktreeName;
        let create_disabled = modal.is_creating
            || modal.repository_path.trim().is_empty()
            || modal.worktree_name.trim().is_empty();

        div()
            .absolute()
            .inset_0()
            .bg(rgb(0x10131a))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(620.))
                    .max_w(px(620.))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.sidebar_bg))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme.text_primary))
                                    .child("Create Worktree"),
                            )
                            .child(
                                action_button(theme, "close-create-worktree", "Close", false, true)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_create_worktree_modal(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme.text_muted))
                            .child("Target base: ~/.arbor/worktrees/<repo>/<worktree>/"),
                    )
                    .child(
                        modal_input_field(
                            theme,
                            "create-worktree-repo-input",
                            "Repository",
                            &modal.repository_path,
                            "Path to git repository",
                            repository_active,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_create_worktree_modal_input(
                                ModalInputEvent::SetActiveField(
                                    CreateWorktreeField::RepositoryPath,
                                ),
                                cx,
                            );
                        })),
                    )
                    .child(
                        modal_input_field(
                            theme,
                            "create-worktree-name-input",
                            "Worktree Name",
                            &modal.worktree_name,
                            "e.g. remote-ssh",
                            worktree_active,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_create_worktree_modal_input(
                                ModalInputEvent::SetActiveField(CreateWorktreeField::WorktreeName),
                                cx,
                            );
                        })),
                    )
                    .child(
                        div()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(theme.border))
                            .bg(rgb(theme.panel_bg))
                            .p_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme.text_muted))
                                    .child("Branch"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_family(FONT_MONO)
                                    .text_color(rgb(theme.text_primary))
                                    .child(branch_name),
                            ),
                    )
                    .child(
                        div()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(theme.border))
                            .bg(rgb(theme.panel_bg))
                            .p_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme.text_muted))
                                    .child("Path"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_family(FONT_MONO)
                                    .text_color(rgb(theme.text_primary))
                                    .child(target_path_preview),
                            ),
                    )
                    .child(div().when_some(modal.error.clone(), |this, error| {
                        this.rounded_sm()
                            .border_1()
                            .border_color(rgb(0xa44949))
                            .bg(rgb(0x4d2a2a))
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(rgb(0xffd7d7))
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                action_button(
                                    theme,
                                    "cancel-create-worktree",
                                    "Cancel",
                                    false,
                                    true,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.close_create_worktree_modal(cx);
                                    },
                                )),
                            )
                            .child(
                                action_button(
                                    theme,
                                    "submit-create-worktree",
                                    if modal.is_creating {
                                        "Creating..."
                                    } else {
                                        "Create Worktree"
                                    },
                                    !create_disabled,
                                    create_disabled,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.submit_create_worktree_modal(cx);
                                    },
                                )),
                            ),
                    ),
            )
    }
}

impl WorktreeSummary {
    fn from_worktree(entry: &worktree::Worktree) -> Self {
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

        let mut state_parts = Vec::new();
        if entry.is_bare {
            state_parts.push("bare".to_owned());
        }
        if entry.is_detached {
            state_parts.push("detached".to_owned());
        }
        if let Some(reason) = &entry.lock_reason {
            state_parts.push(format!("locked ({reason})"));
        }
        if let Some(reason) = &entry.prune_reason {
            state_parts.push(format!("prunable ({reason})"));
        }
        if state_parts.is_empty() {
            state_parts.push("clean".to_owned());
        }

        Self {
            path: entry.path.clone(),
            label,
            branch,
            state: state_parts.join(", "),
            diff_summary: None,
        }
    }
}

impl Render for ArborWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_action(cx.listener(Self::action_refresh_worktrees))
            .on_action(cx.listener(Self::action_refresh_changes))
            .on_action(cx.listener(Self::action_open_create_worktree))
            .on_action(cx.listener(Self::action_use_one_dark_theme))
            .on_action(cx.listener(Self::action_use_ayu_dark_theme))
            .on_action(cx.listener(Self::action_use_embedded_backend))
            .on_action(cx.listener(Self::action_use_alacritty_backend))
            .on_action(cx.listener(Self::action_use_ghostty_backend))
            .child(self.render_top_bar())
            .child(div().h(px(1.)).bg(rgb(theme.chrome_border)))
            .child(div().when_some(self.notice.clone(), |this, notice| {
                this.px_3()
                    .py_2()
                    .bg(rgb(theme.notice_bg))
                    .text_color(rgb(theme.notice_text))
                    .text_xs()
                    .child(notice)
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .flex_row()
                    .child(self.render_left_pane(cx))
                    .child(div().w(px(1.)).bg(rgb(theme.border)))
                    .child(self.render_center_pane(window, cx))
                    .child(div().w(px(1.)).bg(rgb(theme.border)))
                    .child(self.render_right_pane(cx)),
            )
            .child(self.render_status_bar())
            .child(self.render_create_worktree_modal(cx))
    }
}

fn action_button(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    active: bool,
    muted: bool,
) -> Stateful<Div> {
    let background = if active {
        theme.panel_active_bg
    } else {
        theme.panel_bg
    };
    let text_color = if muted {
        theme.text_disabled
    } else {
        theme.text_primary
    };

    div()
        .id(id)
        .cursor_pointer()
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

fn modal_input_field(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    value: &str,
    placeholder: impl Into<String>,
    active: bool,
) -> Stateful<Div> {
    let label = label.into();
    let placeholder = placeholder.into();

    div()
        .id(id)
        .cursor_pointer()
        .rounded_sm()
        .border_1()
        .border_color(rgb(if active {
            theme.accent
        } else {
            theme.border
        }))
        .bg(rgb(theme.panel_bg))
        .p_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(theme.text_muted))
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_family(FONT_MONO)
                .text_color(rgb(if value.is_empty() {
                    theme.text_disabled
                } else {
                    theme.text_primary
                }))
                .child(if value.is_empty() {
                    placeholder
                } else {
                    value.to_owned()
                }),
        )
}

fn status_chip(theme: ThemePalette, label: &str, value: String) -> Div {
    div()
        .rounded_sm()
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(theme.panel_bg))
        .px_2()
        .py_1()
        .text_xs()
        .text_color(rgb(theme.accent))
        .child(format!("{label}: {value}"))
}

fn status_text(theme: ThemePalette, text: impl Into<String>) -> Div {
    div()
        .text_xs()
        .text_color(rgb(theme.text_muted))
        .child(text.into())
}

fn change_code(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Removed => "D",
        ChangeKind::Renamed => "R",
        ChangeKind::Copied => "C",
        ChangeKind::TypeChange => "T",
        ChangeKind::Conflict => "U",
        ChangeKind::IntentToAdd => "I",
    }
}

fn short_branch(value: &str) -> String {
    value
        .strip_prefix("refs/heads/")
        .unwrap_or(value)
        .to_owned()
}

fn expand_home_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("repository path cannot be empty".to_owned());
    }

    if trimmed == "~" {
        return user_home_dir();
    }

    if let Some(suffix) = trimmed.strip_prefix("~/") {
        return user_home_dir().map(|home| home.join(suffix));
    }

    Ok(PathBuf::from(trimmed))
}

fn user_home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME environment variable is not set".to_owned())
}

fn sanitize_worktree_name(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_dash = false;

    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            sanitized.push(character.to_ascii_lowercase());
            previous_dash = false;
            continue;
        }

        if character == '-' || character == '_' || character == '.' {
            sanitized.push(character);
            previous_dash = false;
            continue;
        }

        if !previous_dash && !sanitized.is_empty() {
            sanitized.push('-');
            previous_dash = true;
        }
    }

    while sanitized.ends_with('-') {
        let _ = sanitized.pop();
    }

    sanitized
}

fn derive_branch_name(worktree_name: &str) -> String {
    let sanitized = sanitize_worktree_name(worktree_name);
    if sanitized.is_empty() {
        "worktree".to_owned()
    } else {
        sanitized
    }
}

fn build_managed_worktree_path(repo_name: &str, worktree_name: &str) -> Result<PathBuf, String> {
    let home_dir = user_home_dir()?;
    Ok(home_dir
        .join(".arbor")
        .join("worktrees")
        .join(repo_name)
        .join(worktree_name))
}

fn preview_managed_worktree_path(
    repository_path: &str,
    worktree_name: &str,
) -> Result<String, String> {
    let repository_path = expand_home_path(repository_path)?;
    let repository_name = repository_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "repository name cannot be determined".to_owned())?;
    let sanitized_worktree = sanitize_worktree_name(worktree_name);
    if sanitized_worktree.is_empty() {
        return Err("invalid worktree name".to_owned());
    }

    let path = build_managed_worktree_path(repository_name, &sanitized_worktree)?;
    Ok(path.display().to_string())
}

fn create_managed_worktree(
    repository_path_input: String,
    worktree_name_input: String,
) -> Result<CreatedWorktree, String> {
    let repository_path = expand_home_path(&repository_path_input)?;
    if !repository_path.exists() {
        return Err(format!(
            "repository path does not exist: {}",
            repository_path.display()
        ));
    }

    let repository_root = worktree::repo_root(&repository_path)
        .map_err(|error| format!("failed to resolve repository root: {error}"))?;
    let repository_name = repository_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "repository root has no terminal directory name".to_owned())?;

    let sanitized_worktree_name = sanitize_worktree_name(&worktree_name_input);
    if sanitized_worktree_name.is_empty() {
        return Err("worktree name contains no usable characters".to_owned());
    }

    let branch_name = derive_branch_name(&worktree_name_input);
    let worktree_path = build_managed_worktree_path(repository_name, &sanitized_worktree_name)?;
    if worktree_path.exists() {
        return Err(format!(
            "worktree path already exists: {}",
            worktree_path.display()
        ));
    }

    let Some(parent_directory) = worktree_path.parent() else {
        return Err("invalid worktree path".to_owned());
    };
    fs::create_dir_all(parent_directory).map_err(|error| {
        format!(
            "failed to create worktree parent directory `{}`: {error}",
            parent_directory.display()
        )
    })?;

    worktree::add(
        &repository_root,
        &worktree_path,
        worktree::AddWorktreeOptions {
            branch: Some(&branch_name),
            detach: false,
            force: false,
        },
    )
    .map_err(|error| format!("failed to create worktree: {error}"))?;

    Ok(CreatedWorktree {
        worktree_name: sanitized_worktree_name,
        branch_name,
        worktree_path,
    })
}

fn styled_lines_for_session(
    session: &TerminalSession,
    theme: ThemePalette,
    show_cursor: bool,
) -> Vec<TerminalStyledLine> {
    let mut lines = if !session.styled_output.is_empty() {
        session.styled_output.clone()
    } else {
        plain_lines_to_styled(lines_for_display(&session.output), theme)
    };

    if show_cursor && session.state == TerminalState::Running {
        let cursor_run = TerminalStyledRun {
            text: "█".to_owned(),
            fg: theme.accent,
        };

        if let Some(last_line) = lines.last_mut() {
            if !last_line.runs.is_empty() {
                last_line.runs.push(TerminalStyledRun {
                    text: " ".to_owned(),
                    fg: theme.text_primary,
                });
            }
            last_line.runs.push(cursor_run);
        } else {
            lines.push(TerminalStyledLine {
                runs: vec![cursor_run],
            });
        }
    }

    lines
}

fn plain_lines_to_styled(lines: Vec<String>, theme: ThemePalette) -> Vec<TerminalStyledLine> {
    lines
        .into_iter()
        .map(|line| TerminalStyledLine {
            runs: vec![TerminalStyledRun {
                text: line,
                fg: theme.text_primary,
            }],
        })
        .collect()
}

fn render_terminal_line(line: TerminalStyledLine, theme: ThemePalette) -> Div {
    if line.runs.is_empty() {
        return div()
            .flex_none()
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .whitespace_nowrap()
            .font_family(FONT_MONO)
            .text_sm()
            .text_color(rgb(theme.text_primary))
            .child(" ");
    }

    div()
        .flex_none()
        .w_full()
        .min_w_0()
        .overflow_x_hidden()
        .whitespace_nowrap()
        .font_family(FONT_MONO)
        .flex()
        .items_center()
        .gap_0()
        .children(
            line.runs
                .into_iter()
                .map(|run| div().text_sm().text_color(rgb(run.fg)).child(run.text)),
        )
}

fn lines_for_display(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec!["<no output yet>".to_owned()];
    }

    text.lines().map(ToOwned::to_owned).collect()
}

fn trim_to_last_lines(text: String, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text;
    }

    let mut trimmed = String::new();
    let start = lines.len().saturating_sub(max_lines);
    for line in lines.iter().skip(start) {
        trimmed.push_str(line);
        trimmed.push('\n');
    }
    trimmed
}

fn terminal_scroll_is_near_bottom(scroll_handle: &ScrollHandle) -> bool {
    let max_offset = scroll_handle.max_offset();
    if max_offset.height <= px(0.) {
        return true;
    }

    let offset = scroll_handle.offset();
    let distance_from_bottom = (offset.y + max_offset.height).abs();
    distance_from_bottom <= px(6.)
}

fn should_auto_follow_terminal_output(changed: bool, was_near_bottom: bool) -> bool {
    changed && was_near_bottom
}

fn terminal_bytes_from_keystroke(keystroke: &Keystroke) -> Option<Vec<u8>> {
    if keystroke.modifiers.platform {
        return None;
    }

    let key = keystroke.key.as_str();

    if keystroke.modifiers.control {
        if key.len() == 1 {
            let byte = key.as_bytes().first().copied()?;
            let lower = byte.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                return Some(vec![lower & 0x1f]);
            }
        }

        if key == "space" {
            return Some(vec![0]);
        }
    }

    match key {
        "enter" | "return" => Some(vec![b'\r']),
        "tab" => Some(vec![b'\t']),
        "backspace" => Some(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        _ => {
            if !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && let Some(key_char) = keystroke.key_char.as_ref()
            {
                return Some(key_char.as_bytes().to_vec());
            }

            if !keystroke.modifiers.control && !keystroke.modifiers.alt && key.len() == 1 {
                return Some(key.as_bytes().to_vec());
            }

            None
        },
    }
}

fn request_quit(_: &RequestQuit, cx: &mut App) {
    let now = Instant::now();
    let mut guard = match QUIT_ARMED_AT.lock() {
        Ok(lock) => lock,
        Err(poisoned) => poisoned.into_inner(),
    };

    let should_quit = guard
        .as_ref()
        .is_some_and(|armed_at| now.duration_since(*armed_at) <= QUIT_ARM_WINDOW);

    if should_quit {
        *guard = None;
        cx.quit();
        return;
    }

    *guard = Some(now);
    eprintln!(
        "press Cmd-Q again within {}ms to quit Arbor",
        QUIT_ARM_WINDOW.as_millis(),
    );
}

fn install_app_menu_and_keys(cx: &mut App) {
    cx.on_action(request_quit);
    cx.bind_keys([
        KeyBinding::new("cmd-q", RequestQuit, None),
        KeyBinding::new("cmd-t", SpawnTerminal, None),
        KeyBinding::new("cmd-shift-n", OpenCreateWorktree, None),
        KeyBinding::new("cmd-shift-r", RefreshWorktrees, None),
        KeyBinding::new("cmd-alt-r", RefreshChanges, None),
        KeyBinding::new("cmd-shift-1", UseOneDarkTheme, None),
        KeyBinding::new("cmd-shift-2", UseAyuDarkTheme, None),
        KeyBinding::new("cmd-1", UseEmbeddedBackend, None),
        KeyBinding::new("cmd-2", UseAlacrittyBackend, None),
        KeyBinding::new("cmd-3", UseGhosttyBackend, None),
    ]);
    cx.set_menus(vec![
        Menu {
            name: "Arbor".into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit Arbor", RequestQuit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Terminal Tab", SpawnTerminal),
                MenuItem::action("New Worktree", OpenCreateWorktree),
            ],
        },
        Menu {
            name: "Terminal".into(),
            items: vec![
                MenuItem::action("New Terminal Tab", SpawnTerminal),
                MenuItem::separator(),
                MenuItem::action("Use Embedded Backend", UseEmbeddedBackend),
                MenuItem::action("Use Alacritty Backend", UseAlacrittyBackend),
                MenuItem::action("Use Ghostty Backend", UseGhosttyBackend),
            ],
        },
        Menu {
            name: "Theme".into(),
            items: vec![
                MenuItem::action("Use One Dark", UseOneDarkTheme),
                MenuItem::action("Use Ayu Dark", UseAyuDarkTheme),
            ],
        },
        Menu {
            name: "Worktree".into(),
            items: vec![
                MenuItem::action("New Worktree", OpenCreateWorktree),
                MenuItem::separator(),
                MenuItem::action("Refresh Worktrees", RefreshWorktrees),
                MenuItem::action("Refresh Changes", RefreshChanges),
            ],
        },
    ]);
}

fn main() {
    Application::new().run(|cx: &mut App| {
        install_app_menu_and_keys(cx);
        let bounds = Bounds::centered(None, size(px(1460.), px(900.)), cx);

        if let Err(error) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(1180.), px(760.))),
                app_id: Some("info.penso.arbor".to_owned()),
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(9.), px(9.))),
                }),
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            |_, cx| cx.new(ArborWindow::load),
        ) {
            eprintln!("failed to open Arbor window: {error:#}");
            cx.quit();
            return;
        }

        cx.activate(true);
    });
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    #[test]
    fn sanitizes_worktree_name_for_branch_and_path() {
        let sanitized = crate::sanitize_worktree_name("  Remote SSH / Demo  ");
        assert_eq!(sanitized, "remote-ssh-demo");
    }

    #[test]
    fn derives_default_branch_name_when_empty() {
        let branch = crate::derive_branch_name(" !!! ");
        assert_eq!(branch, "worktree");
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
}
