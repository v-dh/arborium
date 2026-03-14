use {
    arbor_core::repo_config::{self, RepoBranchPrefixMode},
    std::{
        env,
        path::{Path, PathBuf},
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedWorktreePreview {
    pub(crate) sanitized_worktree_name: String,
    pub(crate) branch_name: String,
    pub(crate) worktree_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedWorktreeNaming {
    pub(crate) sanitized_worktree_name: String,
    pub(crate) branch_name: String,
}

pub(crate) fn preview_managed_worktree(
    repo_root: &Path,
    worktree_name: &str,
) -> Result<ManagedWorktreePreview, String> {
    let repository_name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "repository root has no terminal directory name".to_owned())?;
    let naming = derive_managed_worktree_naming(repo_root, worktree_name)?;
    let worktree_path =
        build_managed_worktree_path(repository_name, &naming.sanitized_worktree_name)?;

    Ok(ManagedWorktreePreview {
        sanitized_worktree_name: naming.sanitized_worktree_name,
        branch_name: naming.branch_name,
        worktree_path,
    })
}

pub(crate) fn sanitize_worktree_name(value: &str) -> String {
    arbor_core::worktree_name::sanitize_worktree_name(value)
}

pub(crate) fn derive_managed_worktree_naming(
    repo_root: &Path,
    worktree_name: &str,
) -> Result<ManagedWorktreeNaming, String> {
    let sanitized_worktree_name = sanitize_worktree_name(worktree_name);
    if sanitized_worktree_name.is_empty() {
        return Err("worktree name contains no usable characters".to_owned());
    }

    let github_login = branch_prefix_github_login_from_env();
    let branch_name =
        derive_branch_name_with_repo_config(repo_root, worktree_name, github_login.as_deref());

    Ok(ManagedWorktreeNaming {
        sanitized_worktree_name,
        branch_name,
    })
}

fn derive_branch_name(worktree_name: &str) -> String {
    let sanitized = sanitize_worktree_name(worktree_name);
    let branch_suffix = if sanitized.is_empty() {
        "worktree".to_owned()
    } else {
        sanitized
    };
    format!("codex/{branch_suffix}")
}

fn derive_branch_name_with_repo_config(
    repo_root: &Path,
    worktree_name: &str,
    github_login: Option<&str>,
) -> String {
    let default_branch_name = derive_branch_name(worktree_name);
    let base_name = default_branch_name
        .split_once('/')
        .map(|(_, suffix)| suffix.to_owned())
        .unwrap_or(default_branch_name.clone());
    let Some(config) = repo_config::load_repo_config(repo_root) else {
        return default_branch_name;
    };

    let prefix = match config.branch.prefix_mode {
        Some(RepoBranchPrefixMode::None) => None,
        Some(RepoBranchPrefixMode::GitAuthor) => git_branch_prefix_from_author(repo_root),
        Some(RepoBranchPrefixMode::GithubUser) => github_login
            .map(sanitize_worktree_name)
            .filter(|value| !value.is_empty()),
        Some(RepoBranchPrefixMode::Custom) => config
            .branch
            .prefix
            .as_deref()
            .map(sanitize_worktree_name)
            .filter(|value| !value.is_empty()),
        None => Some("codex".to_owned()),
    };

    match prefix {
        Some(prefix) => format!("{prefix}/{base_name}"),
        None => base_name,
    }
}

fn git_branch_prefix_from_author(repo_root: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_root).ok()?;
    let config = repo.config().ok()?;
    let author = config.get_string("user.name").ok()?;
    let sanitized = sanitize_worktree_name(author.trim());
    (!sanitized.is_empty()).then_some(sanitized)
}

fn build_managed_worktree_path(repo_name: &str, worktree_name: &str) -> Result<PathBuf, String> {
    let home_dir = user_home_dir()?;
    Ok(home_dir
        .join(".arbor")
        .join("worktrees")
        .join(repo_name)
        .join(worktree_name))
}

fn branch_prefix_github_login_from_env() -> Option<String> {
    env::var("ARBOR_GITHUB_USER")
        .ok()
        .or_else(|| env::var("GITHUB_USER").ok())
        .and_then(|value| non_empty_trimmed_str(&value).map(str::to_owned))
}

fn non_empty_trimmed_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn user_home_dir() -> Result<PathBuf, String> {
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home));
    }

    if let Some(home) = env::var_os("USERPROFILE") {
        return Ok(PathBuf::from(home));
    }

    let home = match (env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH")) {
        (Some(drive), Some(path)) => {
            let mut home = PathBuf::from(drive);
            home.push(path);
            home
        },
        _ => return Err("user home directory environment variables are not set".to_owned()),
    };

    Ok(home)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{fs, path::Path},
        tempfile::tempdir,
    };

    #[test]
    fn sanitize_worktree_name_normalizes_symbols() {
        assert_eq!(
            sanitize_worktree_name("  Fix auth / callback race!  "),
            "fix-auth-callback-race"
        );
        assert_eq!(sanitize_worktree_name("ARB-42_bugfix"), "arb-42_bugfix");
        assert_eq!(
            sanitize_worktree_name("Issue 123 Fix parser... now."),
            "issue-123-fix-parser-now"
        );
    }

    #[test]
    fn preview_managed_worktree_uses_repo_name_and_issue_slug() {
        let repo_root = Path::new("/tmp/arbor");
        let preview = preview_managed_worktree(repo_root, "Issue 42 Fix auth")
            .unwrap_or_else(|error| panic!("preview should succeed: {error}"));

        assert_eq!(preview.sanitized_worktree_name, "issue-42-fix-auth");
        assert_eq!(preview.branch_name, "codex/issue-42-fix-auth");
        assert!(
            preview
                .worktree_path
                .ends_with(Path::new("arbor/issue-42-fix-auth"))
        );
    }

    #[test]
    fn derive_branch_name_with_repo_config_uses_custom_prefix() {
        let dir = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        if let Err(error) = fs::write(
            dir.path().join("arbor.toml"),
            "[branch]\nprefix_mode = \"custom\"\nprefix = \"penso\"\n",
        ) {
            panic!("failed to write arbor.toml: {error}");
        }

        assert_eq!(
            derive_branch_name_with_repo_config(dir.path(), "Issue 42", None),
            "penso/issue-42"
        );
    }

    #[test]
    fn derive_branch_name_with_repo_config_uses_git_author_prefix() {
        let dir = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let repo = git2::Repository::init(dir.path())
            .unwrap_or_else(|error| panic!("failed to init repo: {error}"));
        let mut config = repo
            .config()
            .unwrap_or_else(|error| panic!("failed to open repo config: {error}"));
        if let Err(error) = config.set_str("user.name", "Penso Bot") {
            panic!("failed to set git user.name: {error}");
        }
        if let Err(error) = fs::write(
            dir.path().join("arbor.toml"),
            "[branch]\nprefix_mode = \"git-author\"\n",
        ) {
            panic!("failed to write arbor.toml: {error}");
        }

        assert_eq!(
            derive_branch_name_with_repo_config(dir.path(), "Issue 42", None),
            "penso-bot/issue-42"
        );
    }
}
