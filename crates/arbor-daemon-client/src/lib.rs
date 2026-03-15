mod client;
mod error;
mod types;

pub use {
    client::{
        DaemonClient, default_mcp_resource_templates, default_mcp_resources,
        parse_terminal_snapshot_resource, parse_worktree_changes_resource, read_json_text_resource,
    },
    error::DaemonClientError,
    types::{
        AgentSessionDto, ChangedFileDto, CommitWorktreeRequest, CreateManagedWorktreeRequest,
        CreateTerminalRequest, CreateTerminalResponse, CreateWorktreeRequest,
        DeleteWorktreeRequest, GitActionResponse, HealthResponse, IssueDto, IssueLabelDto,
        IssueListResponse, IssueReviewDto, IssueReviewKind, IssueSourceDto, IssueTypeDto,
        ManagedWorktreePreviewRequest, ManagedWorktreePreviewResponse, PushWorktreeRequest,
        RepositoryDto, TerminalResizeRequest, TerminalSignalRequest, WorktreeDto,
        WorktreeMutationResponse,
    },
};
