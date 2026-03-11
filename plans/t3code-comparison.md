# T3 Code vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: `~/code/t3code` (github.com/pingdotgg/t3code)

## What Is T3 Code

"A minimal web GUI for coding agents" by Ping. Currently Codex-first (OpenAI Codex CLI via `codex app-server` JSON-RPC), with Claude Code and Cursor reserved but not yet implemented. Runs as `npx t3` (web mode) or as an Electron desktop app. Very early — version 0.0.9.

Built as a Turborepo monorepo: React 19 + Vite 8 frontend, Effect-TS Node.js backend, WebSocket IPC, SQLite persistence, xterm.js terminal, Lexical rich text editor.

## Architecture Overview

| | T3 Code | Arbor |
|---|---|---|
| **Framework** | Electron 40 + React 19 + Vite 8 + Effect-TS backend | GPUI (pure Rust native) + Web UI (TS + xterm.js) |
| **Platform** | macOS (ARM64 + x64), Linux (AppImage x64), Windows (NSIS x64) | macOS, Linux (x86_64 + aarch64), Windows |
| **Agent Protocol** | JSON-RPC over stdio to `codex app-server` | Terminal PTY sessions (provider-agnostic) |
| **Terminal** | node-pty + xterm.js | Embedded PTY + Alacritty/Ghostty/SSH/Mosh |
| **State** | Event-sourced orchestration + SQLite (Effect-TS) | GPUI reactive model + file-based stores |
| **IPC** | WebSocket (JSON-RPC + push channels) | Direct Rust calls (GUI) / REST + WebSocket (httpd) |
| **Diff** | `@pierre/diffs` (unified + split, virtualized) | Custom GPUI renderer (side-by-side) |
| **Composer** | Lexical rich text editor | None (terminal-based) |
| **Provider Lock-in** | Codex only (Claude/Cursor reserved but unimplemented) | Multi-provider: Claude, Codex, Pi, OpenCode, Copilot |

## Feature Comparison

| Feature | T3 Code | Arbor | Gap / Opportunity |
|---------|---------|-------|-------------------|
| **Provider Support** | Codex only (Claude/Cursor "coming soon") | Claude, Codex, Pi, OpenCode, Copilot | **Arbor wins** — multi-provider today |
| **Chat/Message Timeline** | Full chat UI: user/assistant messages, work logs, streaming, Markdown rendering, virtualized scrolling, image attachments | None — agents run in terminal only | Major gap |
| **Composer** | Lexical rich text: `@` file mentions with autocomplete, `/model`, `/plan`, `/default` slash commands, image paste/drag (up to 8, 10MB each), model picker, reasoning effort, runtime mode toggle | None | Major gap |
| **Plan Mode** | Dedicated interaction mode: agent proposes plan before executing, plan sidebar with step tracking (pending/inProgress/completed), export as text, implement via follow-up | None | Interesting feature |
| **Diff Viewer** | Checkpoint-based: captures git state on turn start/complete, `@pierre/diffs` for unified + split rendering, per-turn diffs, cumulative thread diffs, virtualized | Custom GPUI: changes tab + side-by-side diff | T3's checkpoint approach is clever |
| **Git Operations** | Stacked actions: commit, commit+push, commit+push+PR as atomic pipelines with per-step status reporting | Commit, push, create PR (separate actions) | T3's stacked actions are cleaner UX |
| **AI Commit Messages** | Auto-generated via Codex text generation from diffs | None | Nice feature |
| **Git Worktree** | Per-thread worktree creation, local vs worktree mode toggle, orphaned worktree cleanup warnings | Per-repo worktree management, create/delete | Comparable |
| **PR Review Flow** | Paste PR number/URL -> resolve -> create thread (local or worktree) for review | PR auto-detection per worktree | T3 has dedicated PR review workflow |
| **Terminal** | xterm.js drawer: resizable, up to 4 terminals per thread, split, activity detection, file path links, theme sync | Full PTY: multiple backends, tabs, daemon sessions, SSH/Mosh | **Arbor wins** — more backends, persistent sessions |
| **Session/Thread Management** | Per-project thread list, thread history, drag-and-drop reorder, rename, delete, draft persistence | Monitors agent activity (working/waiting) but no conversation listing | Gap — T3 has full thread management |
| **Sidebar Design** | Project groups + thread list per project, status pills, DnD reorder | Repo groups + worktree cards with activity dots, diff stats | Both have sidebar; T3 is thread-centric, Arbor is worktree-centric |
| **Approval System** | Inline approval prompts: accept/acceptForSession/decline for commands, file changes, file reads | None — uses `--dangerously-skip-permissions` style flags | T3 has granular approval UX |
| **Runtime Modes** | Full Access vs Supervised (approval required) toggle per thread | Fixed per agent preset | Gap — per-session autonomy control |
| **History Bootstrap** | Condensed prior conversation into transcript preamble when resuming threads | None | Maintains agent context across sessions |
| **Project Scripts** | Define project-specific scripts (play/test/lint/configure/build/debug) with custom keybindings | `just` recipes (external) | Different approaches |
| **Keybindings** | Full customization via `~/.t3/keybindings.json` with `when` conditions | GPUI keybindings | Both customizable |
| **Remote Access** | Bind to any interface, auth token, Tailscale guide, `--no-browser` headless | Full HTTP daemon + WebSocket, outpost system | Both good; Arbor's remote is richer |
| **Web UI** | Primary interface is web (Vite SPA via WebSocket) | Separate web dashboard alongside native GUI | T3 is web-first; Arbor has native GUI |
| **Agent Activity** | Thread status pills (connecting/running/error) | Real-time working/waiting dots, task extraction across providers | **Arbor wins** — provider-agnostic, passive detection |
| **Image Support** | Composer: paste/drag images (up to 8, 10MB). Messages: inline image rendering | File viewer: image display | T3's composer attachments are richer |
| **File Search** | Chunked workspace file listing for `@`-mention picker | File tree search in right pane | Different approaches |
| **GitHub Integration** | PR creation/resolution via `gh` CLI, PR thread creation | PR auto-detection, GitHub OAuth, PR URL tracking | Comparable |
| **Desktop App** | Electron with auto-updater, tray, native menus | GPUI native (no Electron overhead) | **Arbor wins** — native performance |
| **Multi-platform** | macOS (ARM64 + x64), Linux (x64 AppImage), Windows (x64 NSIS) | macOS, Linux (x86_64 + aarch64), Windows | Arbor has Linux aarch64 too |
| **Cloud Dependency** | None (fully local, anonymous telemetry only) | None | Both self-contained |
| **Open Source** | Yes | Yes | Both open |

## Unique T3 Code Features Worth Considering

### 1. Event-Sourced Orchestration
T3 uses an event-sourcing pattern: commands produce events, events update a read model (projection), reactors respond asynchronously. All orchestration events persisted to SQLite. This gives:
- Full replay capability
- Checkpoint-based diffs (capture git state on turn start/complete)
- Deterministic test synchronization

**For Arbor**: Overkill for current needs, but the checkpoint-based diff idea (snapshot git state per agent turn) is valuable for showing "what did the agent change in this turn?"

### 2. Plan Mode
An interaction mode where the agent proposes a plan (Markdown with steps) before executing. The plan sidebar shows real-time step status (pending/inProgress/completed). Plans can be exported or used to create new threads.

**For Arbor**: Could be implemented as a UI overlay when an agent outputs structured plan content. Provider-agnostic if parsing agent output for plan markers.

### 3. Stacked Git Actions
Atomic pipelines: `commit` -> `commit+push` -> `commit+push+PR`, each step reports status (created/skipped/pushed). One button press for the full flow.

**For Arbor**: Currently has separate commit, push, create-PR buttons. Could combine into a stacked flow with progress feedback.

### 4. Checkpoint-Based Diffs
Captures git state at turn boundaries (start/complete). Stores diffs as blobs in SQLite. Users can navigate per-turn diffs or see cumulative thread diffs.

**For Arbor**: Could snapshot `git diff --stat` at agent state transitions (working -> waiting) to show per-interaction change summaries on worktree cards.

### 5. History Bootstrap
When resuming a thread in a new session, prior conversation is condensed into a transcript preamble so the agent has context from previous interactions.

**For Arbor**: Relevant if/when adding a chat layer. The terminal approach sidesteps this since agents manage their own session continuity.

### 6. Approval UI
Inline prompts in the chat timeline for approving/declining agent actions: command execution, file changes, file reads. Three response options: accept, accept for session, decline.

**For Arbor**: Not needed while using terminal-based agents (they handle their own approval flows). Would matter if adding a built-in chat.

### 7. Composer `@`-Mention File Picker
Typing `@` in the composer triggers a chunked file/directory listing with search. Files are inserted as path references in the prompt.

**For Arbor**: Useful if adding a composer. Could reuse file tree data from the right pane.

## What Arbor Already Does Better

- **Multi-provider**: Five agents supported today vs Codex-only
- **Native performance**: GPUI vs Electron + React + Effect-TS
- **Terminal backends**: Embedded PTY, Alacritty, Ghostty, SSH, Mosh vs node-pty only
- **Agent monitoring**: Passive, provider-agnostic activity detection vs explicit session tracking
- **Remote daemon**: General-purpose REST/WebSocket with outpost system vs basic WebSocket bind
- **Platform coverage**: Linux aarch64 support, multiple terminal backends for remote
- **Lightweight**: Single Rust binary vs Node.js + Bun + Electron + 1000+ npm deps
- **Worktree-first**: Arbor's core model is worktree management; T3 treats worktrees as optional per-thread isolation

## Priority Ideas from This Comparison

1. **Stacked git actions** — Combine commit + push + PR into a single flow with progress steps. Low effort, good UX.
2. **Per-turn diff snapshots** — Capture git state when agent transitions working -> waiting. Show "what changed this turn" on worktree cards.
3. **PR review thread creation** — Accept PR URL/number, create worktree for reviewing it (also noted in Superset comparison).
4. **AI commit messages** — Generate commit message from staged diff via a quick agent call.
5. **Runtime mode toggle** — Per-session autonomy control (full access vs supervised) instead of fixed preset flags.

## Comparison with CodexMonitor and Superset

| Aspect | CodexMonitor | Superset | T3 Code | Arbor |
|--------|-------------|----------|---------|-------|
| **Provider model** | Codex only (deep) | Any CLI + built-in chat (Anthropic/OpenAI/Google) | Codex only (extensible) | Any CLI (5 presets) |
| **Chat UI** | Full composer | Built-in Mastra chat | Full Lexical composer | None (terminal) |
| **Terminal** | None | node-pty + xterm.js | node-pty + xterm.js | Multi-backend PTY |
| **Framework** | Tauri (Rust + React) | Electron (React) | Electron (React + Effect-TS) | GPUI (Rust) + Web |
| **Worktree model** | Optional (3 workspace types) | Core (every task = worktree) | Optional (per-thread) | Core (worktree-first) |
| **Diff approach** | Live git status | CodeMirror (4 categories) | Checkpoint-based per-turn | Custom GPUI side-by-side |
| **Remote** | TCP JSON-RPC + daemon | Cloud sync + host service | WebSocket bind + auth | REST + WebSocket daemon |
| **Platform** | macOS/Linux/Win/iOS | macOS only | macOS/Linux/Win | macOS/Linux/Win |
| **Cloud** | No | Yes (Neon, Stripe) | No | No |
| **Open source** | Yes | No (commercial) | Yes | Yes |
