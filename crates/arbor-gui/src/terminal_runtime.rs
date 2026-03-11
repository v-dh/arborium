use {
    crate::{
        helpers::{
            apply_daemon_snapshot, apply_terminal_emulator_snapshot, current_unix_timestamp_millis,
            daemon_error_is_connection_refused, daemon_terminal_sync_interval,
            runtime_sync_interval_elapsed,
        },
        terminal_backend::EmbeddedTerminal,
        terminal_daemon_http,
        types::{TerminalSession, TerminalState},
    },
    arbor_core::daemon::{
        DetachRequest, KillRequest, ResizeRequest, SignalRequest, SnapshotRequest,
        TerminalSessionState, TerminalSignal, WriteRequest,
    },
    std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    },
};

pub(crate) type SharedTerminalRuntime = Arc<dyn TerminalRuntimeHandle>;

#[cfg(feature = "ssh")]
/// SSH terminal shell wrapper that provides Clone (via Arc<Mutex>) and
/// a terminal emulator for rendering. Polling is done from the GUI timer
/// since libssh's Channel is not Send/Sync.
#[derive(Clone)]
pub(crate) struct SshTerminalShell {
    shell: Arc<Mutex<arbor_ssh::shell::SshShell>>,
    emulator: Arc<Mutex<arbor_terminal_emulator::TerminalEmulator>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(feature = "ssh")]
impl SshTerminalShell {
    pub(crate) fn open(
        connection: &arbor_ssh::connection::SshConnection,
        cols: u16,
        rows: u16,
        remote_path: &str,
    ) -> Result<Self, String> {
        let shell = arbor_ssh::shell::SshShell::open(
            connection.session(),
            u32::from(cols),
            u32::from(rows),
        )
        .map_err(|e| format!("failed to open SSH shell: {e}"))?;

        // Send cd command to navigate to the outpost directory
        shell
            .write_input(format!("cd {remote_path} && clear\n").as_bytes())
            .map_err(|e| format!("failed to send cd command: {e}"))?;

        Ok(Self {
            shell: Arc::new(Mutex::new(shell)),
            emulator: Arc::new(Mutex::new(
                arbor_terminal_emulator::TerminalEmulator::with_size(rows, cols),
            )),
            generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        })
    }

    pub(crate) fn write_input(&self, bytes: &[u8]) -> Result<(), String> {
        let shell = self
            .shell
            .lock()
            .map_err(|_| "SSH shell lock poisoned".to_owned())?;
        shell
            .write_input(bytes)
            .map_err(|e| format!("failed to write to SSH shell: {e}"))
    }

    /// Poll the shell for new output and feed it to the terminal emulator.
    /// Called from the GUI polling timer. Returns true if new data was processed.
    pub(crate) fn poll(&self) -> bool {
        let shell = match self.shell.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        match shell.read_available() {
            Ok(data) if !data.is_empty() => {
                drop(shell);
                let mut emulator = match self.emulator.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                emulator.process(&data);
                self.generation
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                true
            },
            _ => false,
        }
    }

    pub(crate) fn snapshot(&self) -> arbor_terminal_emulator::TerminalSnapshot {
        let mut snapshot = match self.emulator.lock() {
            Ok(emulator) => emulator.snapshot(),
            Err(poisoned) => poisoned.into_inner().snapshot(),
        };

        let is_closed = self
            .shell
            .lock()
            .map(|s| s.is_closed() || s.is_eof())
            .unwrap_or(true);

        snapshot.exit_code = if is_closed {
            Some(0)
        } else {
            None
        };
        snapshot
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        let shell = self
            .shell
            .lock()
            .map_err(|_| "SSH shell lock poisoned".to_owned())?;
        shell
            .resize(u32::from(cols), u32::from(rows))
            .map_err(|e| format!("failed to resize SSH shell: {e}"))?;
        drop(shell);

        if let Ok(mut emulator) = self.emulator.lock() {
            emulator.resize(rows, cols);
        }
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn close(&self) {
        if let Ok(shell) = self.shell.lock() {
            let _ = shell.close();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalRuntimeKind {
    Local,
    Outpost,
}

pub(crate) struct RuntimeNotification {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) play_sound: bool,
}

#[derive(Default)]
pub(crate) struct TerminalRuntimeSyncOutcome {
    pub(crate) changed: bool,
    pub(crate) close_session: bool,
    pub(crate) clear_global_daemon: bool,
    pub(crate) notice: Option<String>,
    pub(crate) notification: Option<RuntimeNotification>,
}

pub(crate) trait EmulatorRuntimeBackend: Clone {
    fn poll(&self);
    fn write_input(&self, input: &[u8]) -> Result<(), String>;
    fn snapshot(&self) -> arbor_terminal_emulator::TerminalSnapshot;
    fn resize(
        &self,
        rows: u16,
        cols: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<(), String>;
    fn generation(&self) -> u64;
    fn close(&self);
}

pub(crate) trait TerminalRuntimeHandle {
    fn kind(&self) -> TerminalRuntimeKind;
    fn sync_interval(&self, is_active: bool, session_state: TerminalState) -> Duration;
    fn should_sync(
        &self,
        session: &TerminalSession,
        is_active: bool,
        _target_grid_size: Option<(u16, u16, u16, u16)>,
        now: Instant,
    ) -> bool {
        runtime_sync_interval_elapsed(
            session.last_runtime_sync_at,
            self.sync_interval(is_active, session.state),
            now,
        )
    }
    fn write_input(&self, session: &TerminalSession, input: &[u8]) -> Result<(), String>;
    fn sync(
        &self,
        session: &mut TerminalSession,
        is_active: bool,
        target_grid_size: Option<(u16, u16, u16, u16)>,
    ) -> TerminalRuntimeSyncOutcome;
    fn close(&self, session: &TerminalSession) -> Result<(), String>;
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeExitLabels {
    pub(crate) completed_title: &'static str,
    pub(crate) failed_title: &'static str,
    pub(crate) failed_notice_prefix: &'static str,
}

#[derive(Clone)]
pub(crate) struct EmulatorTerminalRuntime<B> {
    pub(crate) backend: B,
    pub(crate) kind: TerminalRuntimeKind,
    pub(crate) resize_error_label: &'static str,
    pub(crate) exit_labels: RuntimeExitLabels,
}

pub(crate) struct DaemonTerminalRuntime {
    pub(crate) daemon: terminal_daemon_http::SharedTerminalDaemonClient,
    pub(crate) ws_state: Arc<DaemonTerminalWsState>,
    pub(crate) last_synced_ws_generation: std::sync::atomic::AtomicU64,
    pub(crate) kind: TerminalRuntimeKind,
    pub(crate) resize_error_label: &'static str,
    pub(crate) snapshot_error_label: &'static str,
    pub(crate) exit_labels: Option<RuntimeExitLabels>,
    pub(crate) clear_global_daemon_on_connection_refused: bool,
}

pub(crate) struct DaemonTerminalWsState {
    event_generation: std::sync::atomic::AtomicU64,
    closed: std::sync::atomic::AtomicBool,
    /// Channel to send keystroke bytes to the WS thread for low-latency binary transmission.
    ws_writer: Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>,
    /// Channel to wake the terminal poller when new data arrives.
    poll_notify: Option<std::sync::mpsc::Sender<()>>,
}

impl Default for DaemonTerminalWsState {
    fn default() -> Self {
        Self::new(None)
    }
}

impl DaemonTerminalWsState {
    pub(crate) fn new(poll_notify: Option<std::sync::mpsc::Sender<()>>) -> Self {
        Self {
            event_generation: std::sync::atomic::AtomicU64::new(0),
            closed: std::sync::atomic::AtomicBool::new(false),
            ws_writer: Mutex::new(None),
            poll_notify,
        }
    }

    pub(crate) fn note_event(&self) {
        self.event_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(ref tx) = self.poll_notify {
            let _ = tx.send(());
        }
    }

    pub(crate) fn event_generation(&self) -> u64 {
        self.event_generation
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn close(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Try to send keystroke bytes through the WebSocket channel.
    /// Returns true if sent, false if WS writer is not available.
    pub(crate) fn try_write(&self, bytes: Vec<u8>) -> bool {
        if let Ok(guard) = self.ws_writer.lock()
            && let Some(ref sender) = *guard
        {
            return sender.send(bytes).is_ok();
        }
        false
    }

    pub(crate) fn set_writer(&self, sender: Option<std::sync::mpsc::Sender<Vec<u8>>>) {
        if let Ok(mut guard) = self.ws_writer.lock() {
            *guard = sender;
        }
    }
}

impl EmulatorRuntimeBackend for EmbeddedTerminal {
    fn poll(&self) {}

    fn write_input(&self, input: &[u8]) -> Result<(), String> {
        EmbeddedTerminal::write_input(self, input)
    }

    fn snapshot(&self) -> arbor_terminal_emulator::TerminalSnapshot {
        EmbeddedTerminal::snapshot(self)
    }

    fn resize(
        &self,
        rows: u16,
        cols: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<(), String> {
        EmbeddedTerminal::resize(self, rows, cols, pixel_width, pixel_height)
    }

    fn generation(&self) -> u64 {
        EmbeddedTerminal::generation(self)
    }

    fn close(&self) {
        EmbeddedTerminal::close(self);
    }
}

#[cfg(feature = "ssh")]
impl EmulatorRuntimeBackend for SshTerminalShell {
    fn poll(&self) {
        let _ = SshTerminalShell::poll(self);
    }

    fn write_input(&self, input: &[u8]) -> Result<(), String> {
        SshTerminalShell::write_input(self, input)
    }

    fn snapshot(&self) -> arbor_terminal_emulator::TerminalSnapshot {
        SshTerminalShell::snapshot(self)
    }

    fn resize(
        &self,
        rows: u16,
        cols: u16,
        _pixel_width: u16,
        _pixel_height: u16,
    ) -> Result<(), String> {
        SshTerminalShell::resize(self, rows, cols)
    }

    fn generation(&self) -> u64 {
        SshTerminalShell::generation(self)
    }

    fn close(&self) {
        SshTerminalShell::close(self);
    }
}

#[cfg(feature = "mosh")]
impl EmulatorRuntimeBackend for arbor_mosh::MoshShell {
    fn poll(&self) {}

    fn write_input(&self, input: &[u8]) -> Result<(), String> {
        arbor_mosh::MoshShell::write_input(self, input)
    }

    fn snapshot(&self) -> arbor_terminal_emulator::TerminalSnapshot {
        arbor_mosh::MoshShell::snapshot(self)
    }

    fn resize(
        &self,
        rows: u16,
        cols: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<(), String> {
        arbor_mosh::MoshShell::resize(self, rows, cols, pixel_width, pixel_height)
    }

    fn generation(&self) -> u64 {
        arbor_mosh::MoshShell::generation(self)
    }

    fn close(&self) {
        arbor_mosh::MoshShell::close(self);
    }
}

impl<B> TerminalRuntimeHandle for EmulatorTerminalRuntime<B>
where
    B: EmulatorRuntimeBackend + 'static,
{
    fn kind(&self) -> TerminalRuntimeKind {
        self.kind
    }

    fn sync_interval(&self, _is_active: bool, _session_state: TerminalState) -> Duration {
        Duration::ZERO
    }

    fn write_input(&self, _session: &TerminalSession, input: &[u8]) -> Result<(), String> {
        self.backend.write_input(input)
    }

    fn sync(
        &self,
        session: &mut TerminalSession,
        is_active: bool,
        target_grid_size: Option<(u16, u16, u16, u16)>,
    ) -> TerminalRuntimeSyncOutcome {
        let mut outcome = TerminalRuntimeSyncOutcome::default();

        if is_active
            && let Some((rows, cols, pixel_width, pixel_height)) = target_grid_size
            && let Err(error) = self.backend.resize(rows, cols, pixel_width, pixel_height)
        {
            outcome.notice = Some(format!("{}: {error}", self.resize_error_label));
        }

        self.backend.poll();

        let generation = self.backend.generation();
        if generation == session.generation {
            return outcome;
        }

        let snapshot = self.backend.snapshot();
        if apply_terminal_emulator_snapshot(session, snapshot) {
            outcome.changed = true;
        }
        session.generation = generation;

        if let Some(exit_code) = session.exit_code
            && session.state == TerminalState::Running
        {
            session.updated_at_unix_ms = current_unix_timestamp_millis();
            if exit_code == 0 {
                outcome.notification = Some(RuntimeNotification {
                    title: self.exit_labels.completed_title.to_owned(),
                    body: format!("`{}` completed successfully", session.title),
                    play_sound: true,
                });
                outcome.close_session = true;
            } else {
                session.state = TerminalState::Failed;
                session.runtime = None;
                outcome.changed = true;
                outcome.notification = Some(RuntimeNotification {
                    title: self.exit_labels.failed_title.to_owned(),
                    body: format!("`{}` failed with code {exit_code}", session.title),
                    play_sound: false,
                });
                outcome.notice = Some(format!(
                    "{} `{}` exited with code {exit_code}",
                    self.exit_labels.failed_notice_prefix, session.title
                ));
            }
        }

        outcome
    }

    fn close(&self, _session: &TerminalSession) -> Result<(), String> {
        self.backend.close();
        Ok(())
    }
}

impl TerminalRuntimeHandle for DaemonTerminalRuntime {
    fn kind(&self) -> TerminalRuntimeKind {
        self.kind
    }

    fn sync_interval(&self, is_active: bool, session_state: TerminalState) -> Duration {
        daemon_terminal_sync_interval(is_active, session_state)
    }

    fn should_sync(
        &self,
        session: &TerminalSession,
        is_active: bool,
        target_grid_size: Option<(u16, u16, u16, u16)>,
        now: Instant,
    ) -> bool {
        if is_active
            && let Some((rows, cols, ..)) = target_grid_size
            && (cols != session.cols || rows != session.rows)
        {
            return true;
        }

        let current_generation = self.ws_state.event_generation();
        let last_synced_generation = self
            .last_synced_ws_generation
            .load(std::sync::atomic::Ordering::Relaxed);
        if current_generation > last_synced_generation {
            return is_active
                || runtime_sync_interval_elapsed(
                    session.last_runtime_sync_at,
                    self.sync_interval(false, session.state),
                    now,
                );
        }

        runtime_sync_interval_elapsed(
            session.last_runtime_sync_at,
            self.sync_interval(is_active, session.state),
            now,
        )
    }

    fn write_input(&self, session: &TerminalSession, input: &[u8]) -> Result<(), String> {
        if input == [0x03] {
            // Ctrl-C: send as signal for reliable delivery
            self.daemon
                .signal(SignalRequest {
                    session_id: session.daemon_session_id.clone(),
                    signal: TerminalSignal::Interrupt,
                })
                .map_err(|error| error.to_string())
        } else if self.ws_state.try_write(input.to_vec()) {
            // Fast path: send via WebSocket binary frame
            tracing::trace!("write_input: sent via WS binary frame");
            Ok(())
        } else {
            // Fallback: HTTP POST (WS not connected yet)
            tracing::trace!("write_input: WS unavailable, falling back to HTTP POST");
            self.daemon
                .write(WriteRequest {
                    session_id: session.daemon_session_id.clone(),
                    bytes: input.to_vec(),
                })
                .map_err(|error| error.to_string())
        }
    }

    fn sync(
        &self,
        session: &mut TerminalSession,
        is_active: bool,
        target_grid_size: Option<(u16, u16, u16, u16)>,
    ) -> TerminalRuntimeSyncOutcome {
        let mut outcome = TerminalRuntimeSyncOutcome::default();
        let observed_ws_generation = self.ws_state.event_generation();

        if is_active
            && let Some((rows, cols, ..)) = target_grid_size
            && (cols != session.cols || rows != session.rows)
        {
            match self.daemon.resize(ResizeRequest {
                session_id: session.daemon_session_id.clone(),
                cols,
                rows,
            }) {
                Ok(()) => {
                    session.cols = cols;
                    session.rows = rows;
                    outcome.changed = true;
                },
                Err(error) => {
                    outcome.notice = Some(format!("{}: {error}", self.resize_error_label));
                },
            }
        }

        match self.daemon.snapshot(SnapshotRequest {
            session_id: session.daemon_session_id.clone(),
            max_lines: 220,
        }) {
            Ok(Some(snapshot)) => {
                self.last_synced_ws_generation
                    .store(observed_ws_generation, std::sync::atomic::Ordering::Relaxed);
                let snapshot_state = terminal_state_from_daemon_state(snapshot.state);
                outcome.changed |= apply_daemon_snapshot(session, &snapshot);
                if session.state != snapshot_state {
                    session.state = snapshot_state;
                    outcome.changed = true;
                }
                if session.exit_code != snapshot.exit_code {
                    session.exit_code = snapshot.exit_code;
                    outcome.changed = true;
                }
                if session.updated_at_unix_ms != snapshot.updated_at_unix_ms {
                    session.updated_at_unix_ms = snapshot.updated_at_unix_ms;
                    outcome.changed = true;
                }

                if let Some(exit_labels) = self.exit_labels
                    && let Some(exit_code) = snapshot.exit_code
                {
                    if exit_code == 0 {
                        outcome.notification = Some(RuntimeNotification {
                            title: exit_labels.completed_title.to_owned(),
                            body: format!("`{}` completed successfully", session.title),
                            play_sound: true,
                        });
                        outcome.close_session = true;
                    } else if session.state == TerminalState::Failed {
                        session.runtime = None;
                        outcome.changed = true;
                        outcome.notification = Some(RuntimeNotification {
                            title: exit_labels.failed_title.to_owned(),
                            body: format!("`{}` failed with code {exit_code}", session.title),
                            play_sound: false,
                        });
                        outcome.notice = Some(format!(
                            "{} `{}` exited with code {exit_code}",
                            exit_labels.failed_notice_prefix, session.title
                        ));
                    }
                }
            },
            Ok(None) => {
                self.last_synced_ws_generation
                    .store(observed_ws_generation, std::sync::atomic::Ordering::Relaxed);
                outcome.close_session = true;
            },
            Err(error) => {
                let error_text = error.to_string();
                if self.clear_global_daemon_on_connection_refused
                    && daemon_error_is_connection_refused(&error_text)
                {
                    session.runtime = None;
                    session.state = TerminalState::Failed;
                    outcome.changed = true;
                    outcome.clear_global_daemon = true;
                } else {
                    outcome.notice = Some(format!(
                        "failed to load {} for terminal `{}`: {error}",
                        self.snapshot_error_label, session.title
                    ));
                }
            },
        }

        outcome
    }

    fn close(&self, session: &TerminalSession) -> Result<(), String> {
        self.ws_state.close();

        let result = if session.state == TerminalState::Running {
            self.daemon.kill(KillRequest {
                session_id: session.daemon_session_id.clone(),
            })
        } else {
            self.daemon.detach(DetachRequest {
                session_id: session.daemon_session_id.clone(),
            })
        };

        result.map_err(|error| error.to_string())
    }
}

pub(crate) fn terminal_state_from_daemon_state(state: TerminalSessionState) -> TerminalState {
    match state {
        TerminalSessionState::Running => TerminalState::Running,
        TerminalSessionState::Completed => TerminalState::Completed,
        TerminalSessionState::Failed => TerminalState::Failed,
    }
}
