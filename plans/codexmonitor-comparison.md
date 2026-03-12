# CodexMonitor vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: <https://github.com/Dimillian/CodexMonitor>

## Architecture Overview

| | CodexMonitor | Arbor |
|---|---|---|
| **Framework** | Tauri v2 (Rust backend + React 19/TS frontend) | GPUI (pure Rust native) + Web UI (TS + xterm.js) |
| **Platform** | macOS, Linux, Windows, iOS (WIP) | macOS, Linux, Windows (GUI + httpd); Web UI anywhere |
| **UI Rendering** | Webview (Vite-bundled React) | Native GPU rendering (GPUI) + separate web dashboard |
| **Agent Protocol** | JSON-RPC over stdio to `codex app-server` | Terminal PTY sessions (provider-agnostic) |
| **Provider Lock-in** | Codex only | Multi-provider: Claude, Codex, Pi, OpenCode, Copilot |

## Feature Comparison

| Feature | CodexMonitor | Arbor | Gap / Opportunity |
|---------|-------------|-------|-------------------|
| **Session/Chat Listing** | Left sidebar shows threads per project via `thread/list` JSON-RPC; threads filtered by matching `cwd` to workspace | Monitors agent *activity* (working/waiting dots) but no conversation history listing | Major gap — add session listing per provider via traits |
| **Chat Composer** | Rich textarea: `@` file mentions, `$` skills, `/prompts:`, image attachments, dictation, prompt history, queue vs steer follow-up | No chat input — agents run in terminal tabs | Major gap — see hybrid approach below |
| **Workspace Creation** | "New Agent" (existing folder), "New Worktree Agent" (git worktree), "New Clone Agent" (full git clone) | Create worktree (branch-based); no clone-agent concept | Add clone-based agent creation; adopt naming |
| **Left Sidebar Design** | Lightweight: project name + thread titles with timestamps, collapsible, pinned threads | Heavier: repo groups with worktree cards, branch names, activity dots, diff stats | Add a compact/list view mode |
| **Diff View** | Right panel: staged/unstaged files, stage/unstage/revert, AI commit message generation, log, issues, PRs | Right pane: changes tab (file list + diff stats), file tree, side-by-side diff viewer | CodexMonitor has AI commit messages and inline issues/PRs |
| **Git Operations** | Commit, push, pull, sync, fetch — all in diff panel | Commit, push, create PR — via action menu | Arbor lacks pull/sync/fetch; has PR creation |
| **Model Selector** | Bottom bar: model picker from `model/list`, reasoning effort slider | None — agents launched with fixed preset commands | Add model/autonomy controls per agent preset |
| **Autonomy/Access Modes** | Three levels: read-only, approve-on-request, full-access | Fixed per preset (e.g. `--dangerously-skip-permissions`) | Expose per-session autonomy controls |
| **Collaboration Modes** | Plan mode toggle, custom collaboration modes with per-mode model/instructions | None | Could add plan-mode toggle per session |
| **Token/Usage Tracking** | Session context ring, weekly/daily usage from JSONL logs, rate limits, credits display | None | Add cost-awareness tracking |
| **Terminal Integration** | None — chat-only interaction with Codex | Full PTY: embedded, Alacritty, Ghostty, SSH, Mosh; multiple tabs per worktree | **Arbor wins significantly** |
| **Remote/Daemon** | Remote backend mode (TCP JSON-RPC), standalone daemon, Tailscale for iOS | Full HTTP daemon with REST API + WebSocket streaming | Both solid; Arbor's is more general-purpose |
| **Agent Activity Monitoring** | Thread status: running/pending/unread | Real-time working/waiting dots, task extraction from session files across providers | **Arbor wins** — provider-agnostic |
| **GitHub Integration** | Issues and PRs inline in diff panel (via `gh` CLI) | PR auto-detection per worktree, GitHub OAuth, PR URL tracking | Both solid; CodexMonitor shows issues inline |
| **Provider Support** | Codex only | Claude, Codex, Pi, OpenCode, Copilot with configurable presets | **Arbor wins** |
| **Web Dashboard** | N/A (Tauri webview is the main UI) | Separate responsive web UI with xterm.js terminals | **Arbor wins** |
| **Keyboard Shortcuts** | Configurable shortcuts for new agent/worktree/clone | Extensive keybindings in GPUI | Both good |
| **File Autocomplete** | `@` mention in composer triggers file path completion | File tree search in right pane | Different approaches |
| **Image Support** | Image attachments in composer, image diffs (base64) | Image file viewing in file tabs | CodexMonitor's composer attachments are richer |
| **Dictation** | Hold-to-talk with Whisper, waveform visualization | None | Nice-to-have |

## How CodexMonitor's Session Listing Works

CodexMonitor does **not** read from Codex's SQLite database directly. The flow is:

1. Spawns `codex app-server` as a child process (stdin/stdout piped)
2. Sends `initialize` request with `experimentalApi: true`
3. Calls `thread/list` to get all known threads
4. Filters threads by matching each thread's `cwd` against registered workspace paths
5. Threads from CLI sessions appear automatically if their `cwd` matches a workspace
6. Events stream via stdout JSON lines, routed to workspaces by thread ID mapping

One `codex app-server` process is shared across all workspaces (session multiplexing).

For **usage tracking**, it separately reads `$CODEX_HOME/sessions/YYYY/MM/DD/*.jsonl` files and parses `token_count`, `agent_message`, `agent_reasoning`, and `response_item` events.

## Ideas for Arbor

### 1. Multi-Provider Session Listing via Traits

Define a trait for session discovery across providers:

```rust
trait AgentSessionProvider {
    fn list_sessions(&self, repo_path: &Path) -> Vec<AgentSession>;
    fn session_messages(&self, session_id: &str) -> Vec<Message>;
    fn provider_name(&self) -> &str;
}
```

Per-provider implementations:
- **Claude Code**: Read `~/.claude/projects/{key}/` session files (already partially done for task extraction)
- **Codex**: Parse `$CODEX_HOME/sessions/YYYY/MM/DD/*.jsonl`, or spawn `codex app-server` for richer access
- **OpenCode**: Investigate session storage format
- **Copilot**: Investigate session format

This gives a lightweight left-pane thread listing grouped by provider, without being locked to any single one.

### 2. Hybrid Terminal + Composer Approach

Keep the terminal as the primary interface (always up-to-date with CLI changes), but add an optional composer overlay:

- **Terminal mode (default)**: Current PTY tabs — always works, always current
- **Enhanced mode**: A composer that constructs CLI commands, pipes input, and parses structured output
  - Claude Code has `--output-format stream-json` for structured output
  - Codex has `codex app-server` JSON-RPC
  - Can render agent responses with richer formatting while still being a thin wrapper over the CLI

This avoids the maintenance burden of reimplementing each provider's full chat protocol while still offering a nicer UX when desired.

### 3. Compact Sidebar Mode

Add a toggle between current card view and a compact list view:
- Show thread/task title + timestamp (like CodexMonitor)
- Collapse branch/diff stats into hover tooltip
- Group by provider icon
- Pin frequently-used sessions

### 4. Workspace Creation Naming

Adopt CodexMonitor's clear naming:
- **New Agent** = open terminal in existing worktree with an agent preset
- **New Worktree Agent** = create git worktree + launch agent (current flow)
- **New Clone Agent** = full `git clone` into sandbox + launch agent (new feature)

### 5. Lower-Priority Additions

- **Token usage tracking**: Parse provider session files for cost awareness (context ring, daily/weekly totals)
- **Model/autonomy selector**: Per-session controls instead of fixed preset commands
- **AI commit messages**: Generate commit message from staged diff via agent
- **Inline GitHub issues**: Show repo issues in the changes/diff panel
- **Plan mode toggle**: Quick switch between plan/execute modes per session

## What Arbor Already Does Better

- **Multi-provider**: Not locked to a single agent CLI
- **Real terminals**: Full PTY access vs chat-only
- **Native performance**: GPUI vs Tauri webview
- **Remote daemon**: General-purpose REST API + WebSocket, not just TCP JSON-RPC
- **Agent monitoring**: Provider-agnostic activity detection across Claude/Codex/Pi/OpenCode/Copilot
- **Web dashboard**: Accessible from any browser, not just the desktop app
