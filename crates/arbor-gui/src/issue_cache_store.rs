use {
    crate::{
        IssueListState, IssueTarget, ManagedDaemonTarget, RepositorySummary, StoreError,
        terminal_daemon_http,
    },
    serde::{Deserialize, Serialize},
    std::{
        collections::{HashMap, HashSet},
        env, fs,
        path::{Path, PathBuf},
        sync::Arc,
    },
};

const ISSUE_CACHE_STORE_RELATIVE_PATH: &str = ".arbor/issues-cache.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct IssueCache {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lists: Vec<CachedIssueList>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedIssueList {
    pub target: IssueTarget,
    pub source: Option<terminal_daemon_http::IssueSourceDto>,
    pub issues: Vec<terminal_daemon_http::IssueDto>,
    pub notice: Option<String>,
}

pub trait IssueCacheStore: Send + Sync {
    fn load(&self) -> Result<IssueCache, StoreError>;
    fn save(&self, cache: &IssueCache) -> Result<(), StoreError>;
}

#[derive(Debug, Clone)]
pub struct JsonIssueCacheStore {
    path: PathBuf,
}

impl JsonIssueCacheStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Default for JsonIssueCacheStore {
    fn default() -> Self {
        Self::new(default_issue_cache_store_path())
    }
}

impl IssueCacheStore for JsonIssueCacheStore {
    fn load(&self) -> Result<IssueCache, StoreError> {
        if !self.path.exists() {
            return Ok(IssueCache::default());
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

    fn save(&self, cache: &IssueCache) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let payload =
            serde_json::to_string_pretty(cache).map_err(|source| StoreError::JsonSerialize {
                path: self.path.display().to_string(),
                source,
            })?;

        fs::write(&self.path, payload).map_err(|source| StoreError::Write {
            path: self.path.display().to_string(),
            source,
        })
    }
}

pub fn default_issue_cache_store() -> Arc<dyn IssueCacheStore> {
    Arc::new(JsonIssueCacheStore::default())
}

pub fn issue_lists_from_cache(
    repositories: &[RepositorySummary],
    cache: &IssueCache,
) -> HashMap<IssueTarget, IssueListState> {
    let available_primary_repo_roots: HashSet<String> = repositories
        .iter()
        .map(|repository| repository.root.display().to_string())
        .collect();

    cache
        .lists
        .iter()
        .filter(|entry| match entry.target.daemon_target {
            ManagedDaemonTarget::Primary => {
                available_primary_repo_roots.contains(entry.target.repo_root.as_str())
            },
            ManagedDaemonTarget::Remote(_) => false,
        })
        .map(|entry| {
            (entry.target.clone(), IssueListState {
                issues: entry.issues.clone(),
                source: entry.source.clone(),
                notice: entry.notice.clone(),
                error: None,
                loading: false,
                loaded: false,
                refresh_generation: 0,
            })
        })
        .collect()
}

pub fn issue_cache_snapshot(
    repositories: &[RepositorySummary],
    base_cache: &IssueCache,
    issue_lists: &HashMap<IssueTarget, IssueListState>,
) -> IssueCache {
    let available_primary_repo_roots: HashSet<String> = repositories
        .iter()
        .map(|repository| repository.root.display().to_string())
        .collect();

    let mut lists_by_target: HashMap<IssueTarget, CachedIssueList> = base_cache
        .lists
        .iter()
        .filter(|entry| match entry.target.daemon_target {
            ManagedDaemonTarget::Primary => {
                available_primary_repo_roots.contains(entry.target.repo_root.as_str())
            },
            ManagedDaemonTarget::Remote(_) => true,
        })
        .map(|entry| (entry.target.clone(), entry.clone()))
        .collect();

    for (target, state) in issue_lists {
        if !state.loaded || state.error.is_some() {
            continue;
        }

        lists_by_target.insert(target.clone(), CachedIssueList {
            target: target.clone(),
            source: state.source.clone(),
            issues: state.issues.clone(),
            notice: state.notice.clone(),
        });
    }

    let mut lists: Vec<_> = lists_by_target.into_values().collect();
    lists.sort_by(|left, right| {
        issue_cache_target_sort_key(&left.target).cmp(&issue_cache_target_sort_key(&right.target))
    });
    IssueCache { lists }
}

fn issue_cache_target_sort_key(target: &IssueTarget) -> (u8, String, usize) {
    match target.daemon_target {
        ManagedDaemonTarget::Primary => (0, target.repo_root.clone(), 0),
        ManagedDaemonTarget::Remote(index) => (1, target.repo_root.clone(), index),
    }
}

fn default_issue_cache_store_path() -> PathBuf {
    resolve_home_relative(ISSUE_CACHE_STORE_RELATIVE_PATH)
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
            IssueCache, IssueCacheStore, JsonIssueCacheStore, issue_cache_snapshot,
            issue_lists_from_cache,
        },
        crate::{
            IssueListState, IssueTarget, ManagedDaemonTarget, RepositorySummary,
            checkout::CheckoutKind,
            repository_store,
            terminal_daemon_http::{IssueDto, IssueSourceDto},
        },
        std::{
            collections::HashMap,
            env, fs,
            path::PathBuf,
            time::{SystemTime, UNIX_EPOCH},
        },
    };

    #[test]
    fn json_issue_cache_store_round_trips_issue_lists() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("arbor-issue-cache-{unique}.json"));
        let store = JsonIssueCacheStore::new(path.clone());

        let cache = IssueCache {
            lists: vec![super::CachedIssueList {
                target: IssueTarget {
                    daemon_target: ManagedDaemonTarget::Primary,
                    repo_root: "/tmp/repo".to_owned(),
                },
                source: Some(IssueSourceDto {
                    provider: "github".to_owned(),
                    label: "GitHub".to_owned(),
                    repository: "acme/repo".to_owned(),
                    url: Some("https://github.com/acme/repo".to_owned()),
                }),
                issues: vec![IssueDto {
                    id: "1".to_owned(),
                    display_id: "#1".to_owned(),
                    title: "Cached issue".to_owned(),
                    state: "open".to_owned(),
                    url: Some("https://github.com/acme/repo/issues/1".to_owned()),
                    body: Some("hello".to_owned()),
                    suggested_worktree_name: "issue-1".to_owned(),
                    updated_at: Some("2026-03-14T00:00:00Z".to_owned()),
                    labels: Vec::new(),
                    issue_type: None,
                    linked_branch: None,
                    linked_review: None,
                }],
                notice: None,
            }],
        };

        store.save(&cache).expect("save issue cache");
        let loaded = store.load().expect("load issue cache");

        assert_eq!(loaded, cache);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn issue_lists_from_cache_hydrates_matching_primary_repositories() {
        let repositories = vec![RepositorySummary::from_checkout_roots(
            PathBuf::from("/tmp/repo"),
            "repo".to_owned(),
            vec![repository_store::RepositoryCheckoutRoot {
                path: PathBuf::from("/tmp/repo"),
                kind: CheckoutKind::LinkedWorktree,
            }],
        )];
        let cache = IssueCache {
            lists: vec![
                super::CachedIssueList {
                    target: IssueTarget {
                        daemon_target: ManagedDaemonTarget::Primary,
                        repo_root: "/tmp/repo".to_owned(),
                    },
                    source: None,
                    issues: vec![IssueDto {
                        id: "1".to_owned(),
                        display_id: "#1".to_owned(),
                        title: "Cached".to_owned(),
                        state: "open".to_owned(),
                        url: None,
                        body: Some("body".to_owned()),
                        suggested_worktree_name: "issue-1".to_owned(),
                        updated_at: None,
                        labels: Vec::new(),
                        issue_type: None,
                        linked_branch: None,
                        linked_review: None,
                    }],
                    notice: None,
                },
                super::CachedIssueList {
                    target: IssueTarget {
                        daemon_target: ManagedDaemonTarget::Primary,
                        repo_root: "/tmp/missing".to_owned(),
                    },
                    source: None,
                    issues: Vec::new(),
                    notice: None,
                },
            ],
        };

        let issue_lists = issue_lists_from_cache(&repositories, &cache);

        assert_eq!(issue_lists.len(), 1);
        let state = issue_lists
            .get(&IssueTarget {
                daemon_target: ManagedDaemonTarget::Primary,
                repo_root: "/tmp/repo".to_owned(),
            })
            .expect("cached issue state should exist");
        assert_eq!(state.issues.len(), 1);
        assert!(!state.loading);
        assert!(!state.loaded);
    }

    #[test]
    fn issue_cache_snapshot_updates_successful_entries_and_prunes_removed_repositories() {
        let repositories = vec![RepositorySummary::from_checkout_roots(
            PathBuf::from("/tmp/repo"),
            "repo".to_owned(),
            vec![repository_store::RepositoryCheckoutRoot {
                path: PathBuf::from("/tmp/repo"),
                kind: CheckoutKind::LinkedWorktree,
            }],
        )];
        let base_cache = IssueCache {
            lists: vec![
                super::CachedIssueList {
                    target: IssueTarget {
                        daemon_target: ManagedDaemonTarget::Primary,
                        repo_root: "/tmp/repo".to_owned(),
                    },
                    source: None,
                    issues: Vec::new(),
                    notice: Some("old".to_owned()),
                },
                super::CachedIssueList {
                    target: IssueTarget {
                        daemon_target: ManagedDaemonTarget::Primary,
                        repo_root: "/tmp/removed".to_owned(),
                    },
                    source: None,
                    issues: vec![IssueDto {
                        id: "stale".to_owned(),
                        display_id: "#9".to_owned(),
                        title: "stale".to_owned(),
                        state: "open".to_owned(),
                        url: None,
                        body: None,
                        suggested_worktree_name: "stale".to_owned(),
                        updated_at: None,
                        labels: Vec::new(),
                        issue_type: None,
                        linked_branch: None,
                        linked_review: None,
                    }],
                    notice: None,
                },
            ],
        };
        let issue_lists = HashMap::from([(
            IssueTarget {
                daemon_target: ManagedDaemonTarget::Primary,
                repo_root: "/tmp/repo".to_owned(),
            },
            IssueListState {
                issues: vec![IssueDto {
                    id: "1".to_owned(),
                    display_id: "#1".to_owned(),
                    title: "fresh".to_owned(),
                    state: "open".to_owned(),
                    url: None,
                    body: Some("body".to_owned()),
                    suggested_worktree_name: "issue-1".to_owned(),
                    updated_at: None,
                    labels: Vec::new(),
                    issue_type: None,
                    linked_branch: None,
                    linked_review: None,
                }],
                source: None,
                notice: None,
                error: None,
                loading: false,
                loaded: true,
                refresh_generation: 1,
            },
        )]);

        let next_cache = issue_cache_snapshot(&repositories, &base_cache, &issue_lists);

        assert_eq!(next_cache.lists.len(), 1);
        assert_eq!(next_cache.lists[0].target.repo_root, "/tmp/repo");
        assert_eq!(next_cache.lists[0].issues[0].title, "fresh");
    }
}
