# OpenWork vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: https://github.com/different-ai/openwork (open source)

## What Is OpenWork

An open-source GUI client for OpenCode (AI coding agent CLI), positioned as an alternative to Anthropic Cowork / OpenAI Codex desktop app. Built with SolidJS + Tauri 2.x. The pitch: "OpenCode, but for everyone" — democratizing agent access for non-developers via polished GUI, messaging bridges (WhatsApp/Telegram/Slack), and cloud-hosted workers.

It is **not** a coding agent itself — it's a control surface / orchestration UI for OpenCode-powered sessions. Significantly different from Arbor's developer-focused worktree management approach.

## Architecture Overview

| | OpenWork | Arbor |
|---|---|---|
| **Framework** | Tauri 2.x (Rust shell + SolidJS frontend) | GPUI (pure Rust native) + Web UI (TS + xterm.js) |
| **Platform** | macOS (primary), Linux, Windows (partial) | macOS, Linux, Windows (all CI-tested) |
| **Agent** | OpenCode only (via SDK, HTTP proxy) | Claude, Codex, Pi, OpenCode, Copilot (terminal) |
| **State** | IndexedDB + localStorage + Solid stores | File-based stores (TOML, JSON) |
| **Backend** | Bun/Node.js server (Hono router) + OpenCode SDK | Direct Rust / REST + WebSocket daemon |
| **Git/Worktree** | None (workspace = directory) | Core feature (worktree management) |
| **Terminal** | None (agent-mediated shell commands) | Multi-backend PTY |
| **Cloud** | Den service (worker provisioning, billing via Polar) | None (fully local/self-hosted) |

## Feature Comparison

| Feature | OpenWork | Arbor | Gap / Opportunity |
|---------|----------|-------|-------------------|
| **Provider Support** | OpenCode only (delegates to OC's provider system) | Claude, Codex, Pi, OpenCode, Copilot | **Arbor wins** on breadth |
| **Chat UI** | Full: streaming markdown, tool call timelines, thinking blocks, diff rendering, image support, virtualized messages | None (terminal-based) | Major gap (recurring theme) |
| **Composer** | Rich contenteditable: `@` mentions (agents, files), `/` slash commands, image attachments, shell mode toggle, model picker, thinking effort selector | None | Major gap |
| **Session Management** | Create/list/rename/abort/summarize sessions per workspace, model overrides, agent assignment, draft history | Monitors agent activity (working/waiting) | OpenWork has full session CRUD |
| **Minimap** | Vertical message navigator with color-coded dots, click-to-jump | None | Nice UX for long conversations |
| **Skills System** | First-class: install from hub (opkg), import local, create in-chat, edit, publish/share as URLs | None | Unique extensibility model |
| **Soul Mode** | Persistent AI worker identity with heartbeat monitoring, steering checklists, self-improvement sweeps | None | Unique — autonomous agent continuity |
| **Scheduled Automations** | Cron-based task scheduling with OS integration (LaunchAgents, systemd), pre-built templates | None | Unique workflow automation |
| **Messaging Bridges** | WhatsApp, Telegram, Slack connectors via opencode-router | None | Broadens access beyond desktop |
| **Permissions UI** | Granular: allow once / allow for session / deny, audit logging | None — uses permissive flags or terminal-based | OpenWork is more granular |
| **Context Panel** | Working files, active plugins, MCP server status, skills list, authorized folders | None | Useful operational dashboard |
| **Artifacts Panel** | Generated files list, reveal in explorer, open in Obsidian | None | Nice for tracking agent outputs |
| **Command Palette** | Cmd+K: search sessions, change model/thinking | None | Commonly requested across competitors |
| **MCP Integration** | MCP server status, auth flow, per-workspace config | Arbor MCP server (separate crate) | Different: OpenWork consumes MCP; Arbor provides MCP |
| **Cloud Workers** | Den service: hosted agent infrastructure, billing, vanity domains | None (self-hosted) | Different target audiences |
| **Multi-Workspace** | Mixed runtimes: local + remote + cloud in single UI | Multiple repos with local/remote daemons | Both support mixed environments |
| **Diff Viewer** | Inline only (within tool call results, syntax highlighted) | Full side-by-side diff, changes panel, file tree | **Arbor wins** |
| **Git/Worktree** | None (workspace = directory, no git awareness) | Core feature: create/delete/track worktrees, branch management | **Arbor wins significantly** |
| **Terminal** | None (agent-mediated shell via tool calls) | Multi-backend PTY: embedded, Alacritty, Ghostty, SSH, Mosh | **Arbor wins significantly** |
| **GitHub Integration** | None | PR detection, GitHub OAuth, PR creation | **Arbor wins** |
| **File System** | Working files display, artifacts panel | File tree, file viewer with syntax highlighting | Different approaches |
| **Remote Access** | Connect to remote OpenCode by URL+token, QR pairing, cloud workers | Full HTTP daemon + REST API + WebSocket | Both strong |
| **Platform** | macOS primary, Linux supported, Windows partial | All three CI-tested | **Arbor wins** on reliability |
| **Open Source** | Yes | Yes | Both open |

## OpenWork's Unique Ideas Worth Considering

### 1. Skills as a First-Class Concept
Installable workflow modules (markdown-based in `.opencode/skills/`):
- Install from hub via `opkg install`
- Import local folders
- Create new skills via in-chat creator tool
- Edit content in modal editor
- Publish/share as URLs

**For Arbor**: The closest analogy would be shareable agent prompt templates. Could support a `.arbor/tasks/` directory with markdown-based task definitions (similar to Polyscope's tasks and Jean's magic commands). The "share as URL" concept is interesting for teams.

### 2. Soul Mode (Persistent Agent Identity)
A persistent AI worker identity with:
- Heartbeat monitoring (6h/12h/daily cadence)
- Steering checklist (focus areas, boundaries/guardrails)
- Self-improvement sweeps
- Continuity across sessions

**For Arbor**: Interesting concept but very different from Arbor's monitoring-focused approach. Could be relevant if Arbor adds a chat layer — persistent agent context per project.

### 3. Scheduled Automations
Cron-based task scheduling integrated with OS:
- macOS LaunchAgents, Linux systemd timers
- Pre-built templates: daily planning brief, inbox summary, meeting prep, weekly recap
- Execution tracking (status, timestamps, exit codes)

**For Arbor**: Could implement via the httpd daemon — scheduled agent sessions triggered by cron. Useful for automated code review, dependency updates, test runs.

### 4. Messaging Platform Bridges
WhatsApp, Telegram, Slack connectors:
- Run agents from messaging apps
- Pairing codes for private access control
- Broader access beyond desktop

**For Arbor**: Very different target audience. Not a priority for developer-focused tool, but the concept of non-desktop access is already served by Arbor's web UI.

### 5. Permission Approval UI
Granular permission responses: allow once / allow for session / deny. Audit logging of all decisions.

**For Arbor**: Relevant if adding a chat layer. The terminal approach currently delegates permission handling to the agent CLI itself.

### 6. Minimap Navigation
Vertical overlay on right side of message area with color-coded dots (user vs assistant). Click-to-jump navigation, active message highlighting.

**For Arbor**: Useful UX pattern for any scrollable content — could apply to terminal output or diff views.

### 7. Hot Reload / Living System
Agents can create new skills or update configuration while sessions are running. Changes take effect without tearing down active sessions.

**For Arbor**: Already has config hot-reload (600ms interval). The concept of agents self-modifying their own tooling is more relevant to the chat/skills paradigm.

## What Arbor Already Does Better

OpenWork and Arbor serve fundamentally different needs:

- **Developer tooling**: Arbor is built for developers managing parallel coding work. OpenWork targets broader "agent for everyone" use cases.
- **Git worktree management**: Arbor's core feature. OpenWork has zero git awareness.
- **Terminal access**: Arbor has full multi-backend PTY. OpenWork has no terminal.
- **Multi-provider**: Arbor supports 5 agent CLIs. OpenWork is OpenCode-only.
- **Diff viewer**: Arbor has full side-by-side diffs. OpenWork shows inline diffs only.
- **GitHub integration**: Arbor has PR detection/creation. OpenWork has none.
- **Platform reliability**: Arbor CI-tests all three platforms. OpenWork's Windows support is partial.
- **Native performance**: GPUI vs Tauri webview + SolidJS.
- **Self-contained**: Single Rust binary vs Node.js/Bun server + Tauri + SolidJS.

## Key Takeaway

OpenWork is the most architecturally different competitor — it's not really competing in the same space as Arbor. Where Arbor is a developer-centric worktree and agent monitoring tool, OpenWork is a general-purpose agent GUI with workflow automation, messaging bridges, and cloud hosting.

The transferable ideas are:
1. **Skills/task templates** as shareable, installable units (aligns with Jean's magic commands, Polyscope's tasks, Conductor's slash commands — this pattern is universal)
2. **Scheduled agent sessions** via OS-level cron integration
3. **Command palette** (Cmd+K) — yet another competitor with this
4. **Minimap** for scrollable content navigation

## Eight-Way Comparison Update

| Aspect | CodexMonitor | Superset | T3 Code | Jean | Conductor | Superterm | Polyscope | OpenWork | Arbor |
|--------|-------------|----------|---------|------|-----------|-----------|-----------|----------|-------|
| **Providers** | Codex | Any CLI+chat | Codex | Claude+Codex+OC | Claude+Codex | Any CLI | Claude+Codex | OpenCode | Any CLI (5) |
| **Chat UI** | Full | Mastra pane | Lexical | Full+magic | Full | None | Full | Full (Solid) | None (term) |
| **Terminal** | None | node-pty | node-pty | portable-pty | WebGL | tmux | Built-in | None | Multi-PTY |
| **Framework** | Tauri | Electron | Electron | Tauri | Native mac | tmux+browser | Native mac | Tauri+Solid | GPUI (Rust) |
| **Git/Worktree** | Optional | Worktree | Optional | Worktree | Worktree | None | CoW clones | None | Worktree |
| **Unique** | Thread list | Browser+MCP | Event-src | Magic cmds | Checkpoints | Attention | Autopilot | Soul+Skills | Remote daemon |
| **Platform** | mac/lin/win | macOS | mac/lin/win | macOS | macOS | lin/mac/wsl | macOS | mac/lin/(win) | mac/lin/win |
| **Open source** | Yes | No | Yes | Yes | No (free) | No ($250/yr) | No | Yes | Yes |
| **Target** | Codex users | Developers | Codex users | Developers | Developers | DevOps/agents | Developers | Everyone | Developers |
