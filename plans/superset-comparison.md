# Superset vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: `~/code/superset` (github.com/superset-sh/superset)

## What Is Superset

"The Terminal for Coding Agents" — an Electron desktop app (macOS) for running, monitoring, and managing multiple AI coding agents in parallel. Each agent gets its own git worktree. Built as a Bun/Turborepo monorepo with React 19, TailwindCSS v4, shadcn/ui, xterm.js, CodeMirror 6, and tRPC over Electron IPC.

## Architecture Overview

| | Superset | Arbor |
|---|---|---|
| **Framework** | Electron 40 + React 19 + Vite 7 | GPUI (pure Rust native) + Web UI (TS + xterm.js) |
| **Platform** | macOS only | macOS, Linux, Windows |
| **Terminal** | node-pty + xterm.js (WebGL) | Embedded PTY + Alacritty/Ghostty/SSH/Mosh |
| **Git** | simple-git (Node.js) | libgit2 / shell git |
| **Database** | SQLite (Drizzle ORM) + Neon PostgreSQL (cloud sync) | File-based stores (TOML config, JSON persistence) |
| **State** | Zustand + TanStack Query + Electric SQL sync | GPUI reactive model |
| **IPC** | tRPC over Electron IPC | Direct Rust function calls (GUI) / REST + WebSocket (httpd) |
| **Diff Viewer** | CodeMirror 6 (side-by-side + unified) | Custom GPUI renderer (side-by-side) |
| **Cloud** | Neon DB, Electric SQL sync, Stripe billing, PostHog analytics | None (local-only, self-hosted daemon) |
| **License** | Commercial (subscription-gated features) | Open source |

## Feature Comparison

| Feature | Superset | Arbor | Gap / Opportunity |
|---------|----------|-------|-------------------|
| **Parallel Agent Execution** | Core feature — run 10+ agents simultaneously | Supports multiple terminals per worktree | Roughly equivalent |
| **Git Worktree Isolation** | Every task gets its own worktree; auto-create/destroy | Worktree creation with branch naming | Comparable; Superset has setup/teardown scripts |
| **Setup/Teardown Scripts** | Per-project scripts (`.superset/config.json`) run on worktree create/delete with env vars | None | Useful for `cp .env`, `bun install`, etc. |
| **Agent Presets** | Named presets: Claude, Codex, Gemini, OpenCode, Copilot, Cursor Agent; pinnable bar, drag-and-drop reorder, Ctrl+1-9 shortcuts | Agent presets: Claude, Codex, Pi, OpenCode, Copilot; configurable via TOML | Superset has richer UX (pinnable bar, reorder, shortcuts) |
| **Built-in Chat** | Mastra-powered chat pane with MCP tools, slash commands, multi-model (Anthropic/OpenAI/Google), file search, image upload | None — agents run in terminal only | Major gap — but Superset's approach adds maintenance burden |
| **Diff Viewer** | CodeMirror: 4 categories (vs-base, committed, staged, unstaged), side-by-side + unified, syntax highlighting for 15+ languages | Custom GPUI: changes tab + side-by-side diff, file tree | Superset is richer (4 categories, unified mode, inline editor) |
| **File Editor** | Full CodeMirror editor with save + conflict detection | File viewer (read-only with syntax highlighting, image support) | Gap — Arbor lacks in-app editing |
| **Git Operations** | Stage/unstage, commit, push (auto upstream), pull (rebase), sync, fetch, stash/pop, discard, delete untracked, create PR, merge PR (squash/merge/rebase) | Commit, push, create PR | Gap — Arbor lacks pull, stash, merge PR strategies |
| **PR Status Tracking** | Per-workspace: PR state, review decision, CI checks, additions/deletions, requested reviewers, preview URLs | PR auto-detection with number + URL | Superset is much richer |
| **PR-based Workspace Creation** | Accept PR URL -> fetch info via `gh` -> create worktree from PR branch (handles forks) | None | Nice workflow for reviewing PRs |
| **Terminal System** | Daemon-backed PTY (node-pty), cold restore, scrollback persistence, session survives app restart | Daemon PTY sessions, attach/detach, multiple backends (embedded, Alacritty, Ghostty, SSH, Mosh) | Both strong; Arbor has more backend options, Superset has cold restore |
| **Port Detection** | Auto-detects listening ports per workspace, shows in sidebar with optional labels (`.superset/ports.json`) | None | Nice feature for dev servers |
| **Built-in Browser** | Embedded Chromium with navigation, screenshot, JS eval, console capture, DevTools pane | None | Unique to Superset |
| **Desktop MCP Server** | Agents can interact with browser: click, screenshot, inspect DOM, navigate, type, evaluate JS | None | Enables agents to test web UIs |
| **Agent Notifications** | Desktop notifications on agent finish/error, custom ringtones, Express hook server | None (activity dots only) | Gap — useful for parallel workflows |
| **Workspace Sections** | User-created groups for organizing workspaces (like folders) | Repository groups (auto-detected from git remotes) | Superset is more flexible; Arbor's is auto-detected |
| **Resource Monitor** | CPU/memory metrics per process | None | Nice-to-have |
| **Left Sidebar** | Project sections -> workspace list, drag-and-drop reorder, multi-select, inline rename, port list, setup script card | Repo groups -> worktree cards with activity dots, diff stats, branch, last activity | Superset is more interactive (DnD, multi-select) |
| **Tab/Pane Layout** | Mosaic split-pane layout (split H/V), tab groups, pane types: terminal, browser, chat, file viewer, devtools | Multi-tab: terminal, diff, file view, logs | Superset has richer layout with arbitrary splits |
| **Command Palette** | Cmd+K style command palette + search dialog | None | Gap |
| **Agent Activity Detection** | Via notification hooks (agents POST events to local server) | Real-time detection from session files (working/waiting) across providers | Different approaches; Arbor's is passive/automatic |
| **Task Management** | Cloud-synced tasks (Linear integration), task-to-workspace dispatch with agent prompts | None | Superset's is cloud-dependent |
| **Branch Naming** | Configurable prefix: github username, author, custom | Manual branch name on worktree creation | Superset automates naming |
| **External Editor Integration** | 20+ editors: VS Code, Cursor, Zed, Sublime, Xcode, iTerm, Warp, Ghostty, JetBrains family | External launcher menu for IDE/terminal | Superset has more integrations |
| **Remote Capabilities** | Host service (Hono tRPC), cloud sync, org-level management | Full HTTP daemon + WebSocket, MCP server, outpost system | Arbor's remote is more general-purpose |
| **Web UI** | Separate web app (app.superset.sh) — cloud-dependent | Self-hosted web dashboard with full terminal access | Arbor's web UI is more self-contained |
| **Provider Support** | Terminal: any CLI agent. Chat: Anthropic, OpenAI, Google | Terminal: Claude, Codex, Pi, OpenCode, Copilot. No chat. | Superset adds Gemini + Cursor Agent + built-in chat |
| **Platform** | macOS only | macOS, Linux, Windows | **Arbor wins** |
| **Cloud Dependency** | Neon PostgreSQL, Electric SQL sync, Stripe billing | None — fully local/self-hosted | **Arbor wins** for privacy/self-hosting |
| **Open Source** | No (commercial) | Yes | **Arbor wins** |

## Unique Superset Features Worth Considering

### 1. Setup/Teardown Scripts
Per-project scripts that run on worktree creation/deletion. Environment variables provided: `SUPERSET_WORKSPACE_PATH`, `SUPERSET_ROOT_PATH`, etc. Common uses: copy `.env`, install dependencies, run migrations. Config in `.superset/config.json`:
```json
{
  "setup": ["./.superset/setup.sh"],
  "teardown": ["./.superset/teardown.sh"]
}
```

**For Arbor**: Already has `worktreeSetupScript` concept from CodexMonitor comparison. Could extend config.toml with per-repo setup/teardown commands.

### 2. Desktop MCP Server for Browser Automation
Agents can programmatically interact with an embedded browser via MCP tools: `take-screenshot`, `click`, `navigate`, `evaluate-js`, `inspect-dom`, `send-keys`, `type-text`, `get-console-logs`. This enables agents to test web UIs they're building.

**For Arbor**: Would require embedding a browser (heavy). Alternative: expose a lighter MCP tool that takes screenshots of external browser windows via OS APIs.

### 3. Port Detection per Workspace
`PortScanner` detects listening TCP ports per workspace by mapping PIDs to process trees. Sidebar shows detected ports with optional labels from `.superset/ports.json`. Clicking opens the URL in the embedded browser.

**For Arbor**: Could detect ports from terminal process trees and show them on worktree cards. Useful feedback for "is my dev server running?"

### 4. Agent Notification Hooks
Express server on a local port receives lifecycle events from agents (start, finish, error). Agents send POSTs via wrapper scripts in `~/.superset/bin`. Triggers desktop notifications with custom sounds.

**For Arbor**: Could implement via the existing agent activity detection system. Add OS-level notifications when an agent transitions from working to waiting.

### 5. Cold Restore (Terminal Session Persistence)
Terminal sessions run in a daemon process. On app restart, sessions are recovered with scrollback replay. Session history persisted to disk.

**For Arbor**: The httpd daemon already persists terminal sessions. Could add scrollback persistence for GUI terminal tabs.

### 6. Mosaic Pane Layout
react-mosaic allows arbitrary horizontal/vertical splits. Users can arrange terminals, browsers, diffs, and chat panes freely within a workspace tab.

**For Arbor**: GPUI supports flexible layouts. Could add split-pane support beyond the current fixed three-pane layout.

### 7. PR-based Workspace Creation
Paste a PR URL -> fetches PR info via `gh` -> creates worktree on PR branch -> sets merge-base for comparison. Handles cross-repository forks.

**For Arbor**: Natural extension of worktree creation. Accept PR URL/number, fetch branch, create worktree, auto-set base for diffing.

### 8. Branch Prefix Modes
Configurable branch naming: `none`, `github` (GitHub username), `author` (git author), `custom` prefix. Auto-generates branch name from workspace name.

**For Arbor**: Could add configurable branch naming patterns to worktree creation.

## What Arbor Already Does Better

- **Multi-platform**: macOS + Linux + Windows vs macOS-only
- **Native performance**: GPUI renders natively vs Electron webview overhead
- **No cloud dependency**: Fully local, self-hosted, no accounts needed
- **Open source**: Community-driven vs commercial
- **Multiple terminal backends**: Embedded PTY, Alacritty, Ghostty, SSH, Mosh — vs node-pty only
- **Provider-agnostic monitoring**: Passive agent detection from session files vs explicit notification hooks
- **Self-hosted remote**: General-purpose REST/WebSocket daemon vs cloud-dependent host service
- **Lightweight**: Single Rust binary vs Electron + Node.js + Bun + 900+ npm dependencies

## Priority Ideas from This Comparison

1. **Setup/teardown scripts** — Low effort, high value. Run user scripts on worktree create/delete.
2. **Agent notifications** — OS-level alerts when an agent finishes (working -> waiting transition). You already detect the state change.
3. **Richer PR tracking** — Show CI status, review decision, additions/deletions per worktree.
4. **Port detection** — Show listening ports per worktree on cards.
5. **Command palette** — Cmd+K for quick actions.
6. **PR-based worktree creation** — "Review PR" flow that creates a worktree from a PR.
7. **Branch prefix modes** — Auto-generate branch names with configurable patterns.
8. **Cold restore for GUI terminals** — Persist scrollback across app restarts.
