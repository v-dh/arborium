export type Repository = {
  root: string;
  label: string;
  github_repo_slug: string | null;
  avatar_url: string | null;
};

export type RightPanelTab = "changes" | "issues";

export type Worktree = {
  repo_root: string;
  path: string;
  branch: string;
  is_primary_checkout: boolean;
  last_activity_unix_ms: number | null;
  diff_additions: number | null;
  diff_deletions: number | null;
  pr_number: number | null;
  pr_url: string | null;
};

export type TerminalState = "running" | "completed" | "failed";

export type TerminalSession = {
  session_id: string;
  workspace_id: string;
  cwd: string;
  shell: string;
  cols: number;
  rows: number;
  title: string | null;
  last_command: string | null;
  output_tail: string | null;
  exit_code: number | null;
  state: TerminalState | null;
  updated_at_unix_ms: number | null;
};

export type ChangedFile = {
  path: string;
  kind: ChangeKind;
  additions: number;
  deletions: number;
};

export type IssueSource = {
  provider: string;
  label: string;
  repository: string;
  url: string | null;
};

export type IssueReviewKind = "pull_request" | "merge_request";

export type IssueReview = {
  kind: IssueReviewKind;
  label: string;
  url: string | null;
};

export type Issue = {
  id: string;
  display_id: string;
  title: string;
  state: string;
  url: string | null;
  suggested_worktree_name: string;
  updated_at: string | null;
  linked_branch: string | null;
  linked_review: IssueReview | null;
};

export type IssueListResponse = {
  source: IssueSource | null;
  issues: Issue[];
  notice: string | null;
};

export type ManagedWorktreePreview = {
  sanitized_worktree_name: string;
  branch: string;
  path: string;
};

export type WorktreeMutationResponse = {
  repo_root: string;
  path: string;
  branch: string | null;
  deleted_branch: string | null;
  message: string;
};

export type ChangeKind =
  | "added"
  | "modified"
  | "removed"
  | "renamed"
  | "copied"
  | "type-change"
  | "conflict"
  | "intent-to-add";

export type ProcessStatus = "running" | "restarting" | "crashed" | "stopped";

export type ProcessInfo = {
  name: string;
  command: string;
  status: ProcessStatus;
  exit_code: number | null;
  restart_count: number;
  session_id: string | null;
};

export type AgentSession = {
  session_id: string;
  cwd: string;
  state: "working" | "waiting";
  updated_at_unix_ms: number;
};

export type AgentActivityWsEvent =
  | { type: "snapshot"; sessions: AgentSession[] }
  | { type: "update"; session: AgentSession };

export type WsClientEvent =
  | { type: "resize"; cols: number; rows: number }
  | { type: "signal"; signal: "interrupt" | "terminate" | "kill" }
  | { type: "detach" };

export type WsServerEvent =
  | { type: "snapshot"; output_tail: string; state: TerminalState; exit_code: number | null; updated_at_unix_ms: number | null }
  | { type: "exit"; state: TerminalState; exit_code: number | null }
  | { type: "error"; message: string };
