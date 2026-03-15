use {
    crate::{
        ExecutionMode, RepositorySidebarTab, SidebarItemId, StoreError, checkout::CheckoutKind,
        github_service,
    },
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        env, fs,
        path::{Path, PathBuf},
        sync::Arc,
    },
};

const UI_STATE_STORE_RELATIVE_PATH: &str = ".arbor/ui-state.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UiState {
    pub left_pane_width: Option<i32>,
    pub right_pane_width: Option<i32>,
    pub window: Option<WindowGeometry>,
    pub left_pane_visible: Option<bool>,
    pub compact_sidebar: Option<bool>,
    pub execution_mode: Option<ExecutionMode>,
    pub preferred_checkout_kind: Option<CheckoutKind>,
    /// Repository groups collapsed in the left sidebar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collapsed_repository_group_keys: Vec<String>,
    /// Custom sidebar item display order per repository group.
    /// Key: group_key, Value: ordered list of SidebarItemIds.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sidebar_order: HashMap<String, Vec<SidebarItemId>>,
    /// Selected left-pane subtab per repository group.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub repository_sidebar_tabs: HashMap<String, RepositorySidebarTab>,
    pub selected_sidebar_selection: Option<PersistedSidebarSelection>,
    pub right_pane_tab: Option<PersistedRightPaneTab>,
    pub logs_tab_open: Option<bool>,
    pub logs_tab_active: Option<bool>,
    /// Resolved pull request state by worktree path for fast startup rendering.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub pull_request_cache: HashMap<String, CachedPullRequestState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PersistedSidebarSelection {
    Repository {
        root: String,
    },
    Worktree {
        repo_root: String,
        path: String,
    },
    Outpost {
        repo_root: String,
        outpost_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PersistedRightPaneTab {
    Changes,
    FileTree,
    Procfile,
    Notes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CachedPullRequestState {
    pub branch: String,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub details: Option<github_service::PrDetails>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub trait UiStateStore: Send + Sync {
    fn load(&self) -> Result<UiState, StoreError>;
    fn save(&self, state: &UiState) -> Result<(), StoreError>;
}

#[derive(Debug, Clone)]
pub struct JsonUiStateStore {
    path: PathBuf,
}

impl JsonUiStateStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Default for JsonUiStateStore {
    fn default() -> Self {
        Self::new(default_ui_state_store_path())
    }
}

impl UiStateStore for JsonUiStateStore {
    fn load(&self) -> Result<UiState, StoreError> {
        if !self.path.exists() {
            return Ok(UiState::default());
        }

        let raw = fs::read_to_string(&self.path).map_err(|source| StoreError::Read {
            path: self.path.display().to_string(),
            source,
        })?;

        serde_json::from_str(&raw).map_err(|source| StoreError::JsonParse {
            path: self.path.display().to_string(),
            source,
        })
    }

    fn save(&self, state: &UiState) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let payload =
            serde_json::to_string_pretty(state).map_err(|source| StoreError::JsonSerialize {
                path: self.path.display().to_string(),
                source,
            })?;

        fs::write(&self.path, payload).map_err(|source| StoreError::Write {
            path: self.path.display().to_string(),
            source,
        })
    }
}

pub fn default_ui_state_store() -> Arc<dyn UiStateStore> {
    Arc::new(JsonUiStateStore::default())
}

pub fn load_startup_state() -> UiState {
    let store = JsonUiStateStore::default();
    store.load().unwrap_or_default()
}

fn default_ui_state_store_path() -> PathBuf {
    resolve_home_relative(UI_STATE_STORE_RELATIVE_PATH)
}

fn resolve_home_relative(relative_path: &str) -> PathBuf {
    home_dir().join(relative_path)
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(".").to_path_buf())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::{
            CachedPullRequestState, JsonUiStateStore, PersistedRightPaneTab,
            PersistedSidebarSelection, UiState, UiStateStore,
        },
        crate::{
            RepositorySidebarTab,
            github_service::{CheckStatus, PrDetails, PrState, ReviewDecision},
        },
        std::{
            collections::HashMap,
            env, fs,
            time::{SystemTime, UNIX_EPOCH},
        },
    };

    #[test]
    fn json_ui_state_store_round_trips_pull_request_cache() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("arbor-ui-state-{unique}.json"));
        let store = JsonUiStateStore::new(path.clone());

        let mut state = UiState::default();
        state.pull_request_cache =
            HashMap::from([("/tmp/repo/wt".to_owned(), CachedPullRequestState {
                branch: "feature/cache".to_owned(),
                number: Some(42),
                url: Some("https://github.com/acme/repo/pull/42".to_owned()),
                details: Some(PrDetails {
                    number: 42,
                    title: "Cache PR details".to_owned(),
                    url: "https://github.com/acme/repo/pull/42".to_owned(),
                    state: PrState::Open,
                    additions: 7,
                    deletions: 2,
                    review_decision: ReviewDecision::Approved,
                    mergeable: crate::github_service::MergeableState::Mergeable,
                    merge_state_status: crate::github_service::MergeStateStatus::Clean,
                    passed_checks: 1,
                    checks_status: CheckStatus::Success,
                    checks: vec![("test".to_owned(), CheckStatus::Success)],
                }),
            })]);

        store.save(&state).expect("save ui state");
        let loaded = store.load().expect("load ui state");

        assert_eq!(loaded.pull_request_cache, state.pull_request_cache);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn json_ui_state_store_round_trips_repository_sidebar_tabs() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("arbor-ui-state-tabs-{unique}.json"));
        let store = JsonUiStateStore::new(path.clone());

        let state = UiState {
            repository_sidebar_tabs: HashMap::from([
                ("repo-a".to_owned(), RepositorySidebarTab::Issues),
                ("repo-b".to_owned(), RepositorySidebarTab::Worktrees),
            ]),
            ..UiState::default()
        };

        store.save(&state).expect("save ui state");
        let loaded = store.load().expect("load ui state");

        assert_eq!(
            loaded.repository_sidebar_tabs,
            state.repository_sidebar_tabs
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn json_ui_state_store_round_trips_collapsed_repository_groups() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("arbor-ui-state-collapsed-{unique}.json"));
        let store = JsonUiStateStore::new(path.clone());

        let state = UiState {
            collapsed_repository_group_keys: vec!["repo-a".to_owned(), "repo-b".to_owned()],
            ..UiState::default()
        };

        store.save(&state).expect("save ui state");
        let loaded = store.load().expect("load ui state");

        assert_eq!(
            loaded.collapsed_repository_group_keys,
            state.collapsed_repository_group_keys
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn json_ui_state_store_round_trips_navigation_selection_and_tabs() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("arbor-ui-state-nav-{unique}.json"));
        let store = JsonUiStateStore::new(path.clone());

        let state = UiState {
            selected_sidebar_selection: Some(PersistedSidebarSelection::Worktree {
                repo_root: "/tmp/repo".to_owned(),
                path: "/tmp/repo/issue-42".to_owned(),
            }),
            right_pane_tab: Some(PersistedRightPaneTab::Notes),
            logs_tab_open: Some(true),
            logs_tab_active: Some(false),
            ..UiState::default()
        };

        store.save(&state).expect("save ui state");
        let loaded = store.load().expect("load ui state");

        assert_eq!(
            loaded.selected_sidebar_selection,
            state.selected_sidebar_selection
        );
        assert_eq!(loaded.right_pane_tab, state.right_pane_tab);
        assert_eq!(loaded.logs_tab_open, state.logs_tab_open);
        assert_eq!(loaded.logs_tab_active, state.logs_tab_active);

        let _ = fs::remove_file(path);
    }
}
