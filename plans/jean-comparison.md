# Jean vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: `~/code/jean` (github.com/coollabsio/jean, v0.1.29)

## What Is Jean

"A desktop AI assistant for managing multiple projects, worktrees, and chat sessions with Claude CLI, Codex CLI, and OpenCode." Built by Andras Bacsai (Coolify creator). Tauri v2 app (Rust backend + React 19/TS frontend) with a focus on opinionated AI-assisted development workflows.

Jean is the most feature-complete of the competitors surveyed — it combines chat UI, terminal, diff viewer, GitHub/Linear integration, "magic commands," and deep worktree management into a single cohesive app.

## Architecture Overview

| | Jean | Arbor |
|---|---|---|
| **Framework** | Tauri v2 (Rust + React 19 + Vite + shadcn/ui v4) | GPUI (pure Rust native) + Web UI (TS + xterm.js) |
| **Platform** | macOS (tested), Windows/Linux (untested) | macOS, Linux, Windows (all CI-tested) |
| **Agent Protocol** | Claude: spawn CLI + tail JSONL output. Codex: `codex app-server` JSON-RPC. OpenCode: HTTP server. | Terminal PTY sessions (provider-agnostic) |
| **Terminal** | portable-pty (Rust) | Embedded PTY + Alacritty/Ghostty/SSH/Mosh |
| **State** | JSON files per worktree/session (no database) | File-based stores (TOML config, JSON) |
| **IPC** | Tauri commands (Rust ↔ JS) + Axum HTTP/WS for remote | Direct Rust calls (GUI) / REST + WebSocket (httpd) |
| **Diff Viewer** | CodeMirror 6 (unified + side-by-side) | Custom GPUI renderer (side-by-side) |
| **Chat** | Native chat UI parsing CLI output streams | None (terminal-based) |

## Feature Comparison

| Feature | Jean | Arbor | Gap / Opportunity |
|---------|------|-------|-------------------|
| **Provider Support** | Claude CLI, Codex CLI (`app-server`), OpenCode (`opencode serve`) | Claude, Codex, Pi, OpenCode, Copilot (terminal presets) | Jean has deeper integration (parsing output); Arbor has more providers |
| **Chat UI** | Full chat: message timeline, streaming, tool call display, thinking blocks, content blocks, permission approval, image support | None — agents run in terminal | Major gap |
| **Execution Modes** | Plan / Build / Yolo — per-session toggle affecting CLI flags | Fixed per agent preset (e.g. `--dangerously-skip-permissions`) | Gap — Jean has granular per-session modes |
| **Model Selection** | Claude: Opus/Sonnet/Haiku. Codex: GPT models. OpenCode: dynamic model list. Per-session switching. | None — fixed preset commands | Gap |
| **Thinking/Effort Levels** | Off/Think/Megathink/Ultrathink (Claude), Low/Medium/High/Max effort (Opus 4.6) | None | Gap |
| **Multiple Sessions per Worktree** | Yes — tab-based sessions within each worktree, create/archive/restore | One terminal per tab per worktree | Jean's multi-session model is richer |
| **Session Canvas View** | Card-based overview of all sessions in a worktree with status indicators (idle/planning/vibing/yoloing/waiting/review/permission/completed) | Worktree cards in sidebar with activity dots | Jean's canvas is more detailed |
| **Session Naming** | AI-generated from first message (via Claude CLI call) | Task extraction from session files | Similar; Jean auto-renames branches too |
| **Session Digest/Recap** | AI-generated recap when returning to unfocused session (summary + last action) | None | Nice UX for managing many parallel sessions |
| **Session Labels** | User-assigned colored labels (e.g. "In Progress", "Needs Testing") | None | Useful for organizing parallel work |
| **Session Archiving** | Archive/unarchive sessions and worktrees; auto-archive on PR merge | None | Lifecycle management |
| **Magic Commands** | Investigate issues/PRs/workflows, code review with finding tracking, AI commit messages, PR content generation, merge conflict resolution, release notes, security alert investigation | None | Major differentiator — see details below |
| **Code Review** | AI-powered review with severity-rated findings, fix-one/fix-all buttons that send prompts back to agent | None | Unique workflow |
| **Custom System Prompts** | Global system prompt (opinionated default), per-project custom prompts, per-operation magic prompts, parallel execution prompt | None | Deep customization |
| **Custom CLI Profiles** | Support for OpenRouter and other providers via custom `settings.*.json` files and env vars | None | Enables non-standard providers |
| **MCP Integration** | MCP server support for agents, per-project MCP server enable/disable, health monitoring | Arbor MCP server (separate crate) | Both have MCP; Jean's is per-project configurable |
| **Claude Skills/Commands** | Reads `~/.claude/skills/` and `~/.claude/commands/` for context and slash commands | None | Nice integration with Claude ecosystem |
| **Permission Approval** | Inline approval UI for tool calls (accept/deny/accept-for-session), tool call display with input/output | None — uses permissive flags or terminal-based approval | Jean's is more granular |
| **Diff Viewer** | CodeMirror 6: unified + side-by-side, 15+ language modes, file save with conflict detection, binary/image preview | Custom GPUI: side-by-side, syntax highlighting | Jean is richer (unified mode, save, conflict detection) |
| **Git Operations** | Commit (AI messages), push, pull, create PR (AI-generated title/body), merge PR, resolve conflicts | Commit, push, create PR | Jean has merge, pull, AI messages, conflict resolution |
| **PR Status Tracking** | PR state, CI checks, behind/ahead counts, uncommitted changes, branch diff stats | PR auto-detection with number + URL | Jean is much richer |
| **GitHub Integration** | Issue/PR investigation, checkout PR as worktree, workflow run investigation, security alerts, Dependabot | PR auto-detection, GitHub OAuth | Jean is significantly deeper |
| **Linear Integration** | Per-project Linear API key, issue listing/filtering by team, investigate Linear issues | None | Unique to Jean |
| **Terminal** | portable-pty based terminal (xterm.js rendering expected) | Full PTY: embedded, Alacritty, Ghostty, SSH, Mosh | **Arbor wins** — more backends |
| **Detached Sessions** | Claude CLI spawned as detached process (survives app quit), output tailed from JSONL file | Daemon terminal sessions (survive app restarts) | Similar resilience approaches |
| **Remote Access** | Axum HTTP server + WebSocket, token auth, serves frontend as SPA | Full HTTP daemon + REST API + WebSocket | Both solid |
| **Setup Scripts** | Per-project setup script (runs on worktree creation) | None | Useful for environment provisioning |
| **Worktree Creation** | From branch, from GitHub issue, from PR (auto-checkout), with random names (e.g. "fuzzy-tiger") | From branch with custom name | Jean has more creation sources |
| **Worktree Cleanup** | Delete with teardown script, auto-archive on PR merge, orphaned worktree warnings | Delete with unpushed commit detection | Jean has more lifecycle management |
| **File Tree** | Right sidebar with file tree + changes view | Right pane: changes tab + files tab | Comparable |
| **Open in Editor** | Zed, VS Code, Cursor, Xcode and more | External launcher menu | Comparable |
| **Sidebar Design** | Project tree with folder groups, worktree list, drag-and-drop, inline rename | Repo groups with worktree cards, activity dots, diff stats | Jean has folders and DnD |
| **Notifications** | Sound options (none/ding/chime/pop/choochoo) for session completion | None | Gap |
| **Keybindings** | Full customization system | GPUI keybindings | Both customizable |
| **Parallel Execution Prompt** | Optional system prompt encouraging sub-agent usage for parallel work | None | Interesting approach |
| **Unread Indicators** | Per-session unread tracking | None | Useful for multi-session workflows |
| **Platform** | macOS tested, Win/Linux untested | macOS/Linux/Windows CI-tested | **Arbor wins** |
| **Web UI** | Remote serves SPA via Axum | Separate responsive web dashboard | Both have web access |
| **Open Source** | Yes (MIT) | Yes | Both open |

## Jean's Magic Commands — Detailed

Jean's "Magic Modal" (⌘M) provides AI-powered operations that go beyond simple chat:

| Magic Command | What It Does |
|---------------|--------------|
| **Investigate Issue** | Loads GitHub issue context, sends to agent with structured investigation prompt |
| **Investigate PR** | Loads PR context, sends to agent for analysis |
| **Investigate Workflow Run** | Loads failed GitHub Actions run, sends for debugging |
| **Investigate Security Alert** | Loads Dependabot/security advisory, sends for analysis |
| **Code Review** | Runs AI review on branch diff, produces severity-rated findings panel with fix-one/fix-all |
| **AI Commit** | Generates commit message from staged changes via agent |
| **Commit and Push** | Commit + push in one action |
| **Create PR** | AI-generates PR title/body from branch diff |
| **Update PR** | Updates existing PR description |
| **Merge** | Merge PR from within the app |
| **Resolve Conflicts** | Sends merge conflicts to agent with resolution prompt |
| **Release Notes** | Generates release notes from commits/PRs |
| **Save Context** | Saves session context for later reuse |
| **Load Context** | Loads saved contexts (issues, PRs, files) into session |
| **Create Recap** | AI-generates session digest (summary + last action) |

Each magic prompt is **customizable** in settings with defaults that can be overridden per-user.

## Jean's Session Model

Jean has a sophisticated multi-session-per-worktree model:

- **Sessions**: Multiple chat sessions per worktree (tab-based), each with its own message history, model, execution mode, thinking level
- **Canvas View**: Card-based overview showing all sessions with real-time status
- **Session States**: idle, planning, vibing, yoloing, waiting, review, permission, completed
- **Archiving**: Sessions and worktrees can be archived/unarchived
- **Digests**: AI-generated recap when returning to an unfocused session
- **Labels**: User-assigned colored labels for organization
- **Recovery**: Crashed sessions can be recovered with partial message history
- **Detached execution**: Claude CLI runs as a detached process that writes to JSONL; Jean tails the file for real-time updates. Sessions survive app quit.

## What Arbor Already Does Better

- **Multi-platform**: All three platforms CI-tested vs macOS-only tested
- **Native performance**: GPUI vs Tauri webview
- **Terminal backends**: Alacritty, Ghostty, SSH, Mosh — Jean only has portable-pty
- **Provider breadth**: 5 providers vs 3 (Jean lacks Pi and Copilot)
- **Provider-agnostic monitoring**: Passive activity detection without parsing CLI output
- **Remote daemon**: More general-purpose REST API vs Tauri-centric Axum server
- **Lightweight**: Single Rust binary vs Tauri + React + 100s of npm deps
- **Self-hosted web dashboard**: Full terminal access in browser

## Priority Ideas from This Comparison

### High Value
1. **Session management per worktree** — Multiple sessions per worktree with tabs. Jean's model is the most mature of all competitors.
2. **Execution mode toggle** — Plan/Build/Yolo per session. Maps naturally to CLI flags (`--plan`, default, `--dangerously-skip-permissions`).
3. **AI commit messages** — Generate from staged diff via quick agent call. All three competitors have this.
4. **Session digest/recap** — When returning to an unfocused worktree, show AI-generated summary of what happened. Good for managing parallel work.
5. **Magic commands modal** — Structured AI operations (investigate issue, code review, PR content) as a command palette.

### Medium Value
6. **GitHub issue/PR investigation** — Load issue/PR context into agent session. Extends existing PR detection.
7. **Code review with findings panel** — AI review producing severity-rated findings with fix buttons.
8. **Setup/teardown scripts** — Per-project scripts on worktree create/delete (also in Superset).
9. **Session labels** — Color-coded labels for organizing parallel sessions.
10. **Notifications on agent completion** — Desktop notifications when sessions complete (also in Superset).

### Lower Priority
11. **Custom system prompts** — Per-project and global agent instructions.
12. **Linear integration** — Issue tracking integration.
13. **Merge conflict resolution** — Send conflicts to agent with structured prompt.
14. **Release notes generation** — Generate from commits/PRs.
15. **Parallel execution prompt** — System prompt encouraging sub-agent usage.

## Five-Way Comparison Update

| Aspect | CodexMonitor | Superset | T3 Code | Jean | Arbor |
|--------|-------------|----------|---------|------|-------|
| **Providers** | Codex only | Any CLI + built-in chat | Codex only | Claude + Codex + OpenCode | Any CLI (5 presets) |
| **Chat UI** | Full composer | Mastra chat pane | Lexical composer | Full chat + magic commands | None (terminal) |
| **Terminal** | None | node-pty + xterm.js | node-pty + xterm.js | portable-pty | Multi-backend PTY |
| **Framework** | Tauri (Rust+React) | Electron (React) | Electron (React+Effect) | Tauri (Rust+React) | GPUI (Rust) + Web |
| **Worktree model** | Optional (3 types) | Core (every task) | Optional (per-thread) | Core (multi-session) | Core (worktree-first) |
| **Diff** | Live git status | CodeMirror (4 categories) | Checkpoint per-turn | CodeMirror (unified+split) | Custom GPUI side-by-side |
| **GitHub depth** | Issues/PRs in panel | PR status/CI | PR resolution | Deep: investigate issues/PRs/workflows/security | PR auto-detection |
| **Magic/AI ops** | None | None | AI commit messages | Full suite (15 operations) | None |
| **Sessions** | Thread list per project | Workspace = worktree | Thread per project | Multi-session per worktree | Activity monitoring |
| **Remote** | TCP JSON-RPC | Cloud sync | WebSocket bind | Axum HTTP/WS | REST + WS daemon |
| **Platform** | macOS/Linux/Win/iOS | macOS only | macOS/Linux/Win | macOS (tested) | macOS/Linux/Win |
| **Cloud dep** | No | Yes (Neon, Stripe) | No | No | No |
| **Open source** | Yes | No (commercial) | Yes | Yes (MIT) | Yes |
