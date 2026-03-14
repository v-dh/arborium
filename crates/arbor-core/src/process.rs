use {
    schemars::JsonSchema,
    serde::{Deserialize, Serialize},
};

/// Status of a managed process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessStatus {
    Running,
    Restarting,
    Crashed,
    Stopped,
}

/// Source of a managed process definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessSource {
    ArborToml,
    Procfile,
}

pub const PROCFILE_MANAGED_PROCESS_TITLE_PREFIX: &str = "[Procfile] ";

pub fn procfile_managed_process_title(process_name: &str) -> String {
    format!("{PROCFILE_MANAGED_PROCESS_TITLE_PREFIX}{process_name}")
}

pub fn procfile_managed_process_name_from_title(title: &str) -> Option<&str> {
    title
        .strip_prefix(PROCFILE_MANAGED_PROCESS_TITLE_PREFIX)
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// Runtime information about a managed process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub repo_root: String,
    pub workspace_id: String,
    pub source: ProcessSource,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub restart_count: u32,
    /// Resident memory for the process tree rooted at this managed process.
    pub memory_bytes: Option<u64>,
    /// Links to a terminal daemon session, if any.
    pub session_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::process::{
        PROCFILE_MANAGED_PROCESS_TITLE_PREFIX, procfile_managed_process_name_from_title,
        procfile_managed_process_title,
    };

    #[test]
    fn procfile_title_round_trips_process_name() {
        let title = procfile_managed_process_title("web");

        assert_eq!(title, format!("{PROCFILE_MANAGED_PROCESS_TITLE_PREFIX}web"));
        assert_eq!(
            procfile_managed_process_name_from_title(&title),
            Some("web")
        );
    }

    #[test]
    fn procfile_title_rejects_empty_process_name() {
        assert_eq!(
            procfile_managed_process_name_from_title(PROCFILE_MANAGED_PROCESS_TITLE_PREFIX),
            None
        );
    }
}
