# Superbot2 vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: https://github.com/gkkirsch/superbot2 (open source)

## What Is Superbot2

An autonomous multi-agent AI orchestrator built on Claude Code's experimental agent teams feature. Not a chat UI or monitoring tool — it's a **project management automation layer** that runs a persistent team of AI agents managing an entire portfolio of projects. You define spaces (domains) and projects, then Superbot2's team-lead agent autonomously plans, executes tasks, files escalations, and self-improves via heartbeat cron jobs.

Built as: Bash orchestration scripts + Node.js Express API + React/Vite dashboard + Electron macOS tray app. macOS only.

## Architecture Overview

| | Superbot2 | Arbor |
|---|---|---|
| **Architecture** | Bash scripts wrapping Claude CLI + Express API + React dashboard + Electron tray | GPUI (Rust native) + Web UI (TS + xterm.js) + HTTP daemon |
| **Platform** | macOS only | macOS, Linux, Windows |
| **Agent** | Claude Code only (agent teams: team-lead + space-workers) | Claude, Codex, Pi, OpenCode, Copilot (terminal) |
| **Data Storage** | Flat files (JSON + Markdown) in `~/.superbot2/` | File-based stores (TOML, JSON) |
| **Orchestration** | Autonomous: heartbeat cron + scheduler + escalation-driven | Manual: user launches agents in terminals |
| **Git/Worktree** | None (workers operate in `codeDir` per space) | Core feature |
| **Terminal** | None in UI (Claude CLI runs in background) | Multi-backend PTY |

## Feature Comparison

| Feature | Superbot2 | Arbor | Gap / Opportunity |
|---------|-----------|-------|-------------------|
| **Autonomy Model** | Fully autonomous: heartbeat cron wakes orchestrator, scheduler runs recurring tasks, workers self-assign | User-initiated: launch agent in terminal, monitor activity | Fundamentally different paradigms |
| **Multi-Agent Coordination** | Team-lead orchestrator spawns space-workers via Claude Code agent teams, file-based message passing via team inboxes | Multiple independent agents in separate terminals | Superbot2 has formal coordination |
| **Escalation System** | Typed escalations (decision/blocker/question/approval), three-phase lifecycle (untriaged→needs_human→resolved), auto-triage rules, suggested answers | None | Unique async human-in-the-loop pattern |
| **Scheduling** | Cron-based: heartbeat (change detection), recurring tasks (self-improvement, daily reminders), launchd integration | None | Gap for automated recurring work |
| **Project/Task Management** | Spaces → Projects → Tasks with priorities, acceptance criteria, blocking dependencies, backlog, todos | Worktrees with activity monitoring | Superbot2 is a project manager; Arbor is a dev tool |
| **Communication Channels** | Dashboard chat, iMessage bridge (macOS chat.db), Telegram bot with inline buttons | Web UI (browser) | Superbot2 has messaging bridges |
| **Skills System** | Claude Code skills in `~/.claude/skills/`, skill creator UI, plugin marketplace with credentials | None | Similar to OpenWork's skills concept |
| **Knowledge Management** | Global + per-space knowledge files, IDENTITY.md, USER.md, MEMORY.md, persistent context | None | Long-running agent memory |
| **Browser Automation** | Chrome DevTools Protocol with persistent auth profile | None | Enables web interaction |
| **Dashboard Layout** | Customizable: drag-and-drop sections (Pulse, Goals, Skills, Escalations, Files, Schedule, Todos), hide/show, collapsible | Fixed three-pane layout | Superbot2's dashboard is more flexible |
| **Session Management** | UUID session IDs, resume capability, session summaries (files changed, summary) | Monitors agent activity (working/waiting) | Different: Superbot2 manages autonomous sessions |
| **Heartbeat System** | Cron fingerprints filesystem state (MD5), generates actionable messages only on changes | Event-driven change detection in arbor-core | Different mechanisms, same goal |
| **`teammate-idle` Hook** | Enforces PM discipline: workers must update tasks, report results, verify work, dispatch reviews before going idle | None | Ensures agent completion quality |
| **Self-Improvement** | Scheduled analysis of system behavior with auto-generated suggestions | None | Autonomous system evolution |
| **Auto-Triage Rules** | User-defined rules for automatic escalation resolution | None | Reduces human intervention overhead |
| **Git/Worktree** | Minimal (workers commit with conventions, no UI) | Full worktree management, create/delete, branch tracking | **Arbor wins significantly** |
| **Diff Viewer** | None | Side-by-side diff, changes panel, file tree | **Arbor wins** |
| **Terminal** | None in UI | Multi-backend PTY: embedded, Alacritty, Ghostty, SSH, Mosh | **Arbor wins** |
| **GitHub Integration** | None | PR detection, GitHub OAuth, PR creation | **Arbor wins** |
| **Provider Support** | Claude Code only | 5 providers | **Arbor wins** |
| **Platform** | macOS only | macOS/Linux/Windows | **Arbor wins** |
| **Remote Access** | Express API (local), Telegram bot | Full REST + WebSocket daemon | **Arbor wins** |
| **Open Source** | Yes | Yes | Both open |

## Superbot2's Unique Ideas Worth Considering

### 1. Escalation-Driven Workflow
Rather than blocking on decisions, workers file typed escalations:
- **Types**: decision, blocker, question, approval, improvement, agent_plan
- **Lifecycle**: untriaged → needs_human → resolved
- **Auto-triage**: User-defined rules let the orchestrator resolve routine questions automatically
- **Suggested answers**: Each escalation comes with a suggested resolution for one-click approval

**For Arbor**: Could surface agent "stuck" states more prominently. When an agent transitions to waiting state, detect whether it's asking a question (parse terminal output) and surface it as a notification with suggested action. Lighter-weight than full escalations but similar UX value.

### 2. Heartbeat / Change Detection System
A cron job that:
1. Fingerprints entire filesystem state (escalations, knowledge, tasks) via MD5 hashing
2. Compares against previous fingerprint
3. Only wakes the orchestrator when changes are detected
4. Generates actionable messages describing what changed

**For Arbor**: Already has event-driven change detection in arbor-core. The "generate actionable messages" concept is interesting — could summarize what changed since last user interaction.

### 3. `teammate-idle` Hook (Completion Enforcement)
A Claude Code hook that prevents workers from going idle until they have:
- Updated all task statuses
- Reported results to the team inbox
- Verified their work passes tests
- Dispatched code reviews if applicable
- Distilled knowledge into notes
- Output a specific completion keyword

**For Arbor**: Could implement as a Claude Code hook configuration that Arbor installs. Ensures agents complete their work properly before signaling "done." Useful for parallel workflows where you want quality guarantees.

### 4. Persistent Agent Memory
Multiple layers of persistent context:
- `IDENTITY.md` — agent personality and behavior guidelines
- `USER.md` — user preferences and working style
- `MEMORY.md` — accumulated knowledge from past sessions
- Per-space knowledge directories
- Session summaries with files changed

**For Arbor**: Could add per-repo context files that agents read on session start. Already partially done with CLAUDE.md conventions, but formalizing it for all providers would be valuable.

### 5. Messaging Bridges (iMessage + Telegram)
- **iMessage**: Polls macOS `chat.db` SQLite, relays messages bidirectionally
- **Telegram**: Long-polling bot with inline buttons for escalation resolution, slash commands for status/todo/schedule

**For Arbor**: The Telegram/Slack notification pattern is interesting — when an agent finishes or needs attention, send a notification to a messaging app. Lighter than full bridges but high-value for mobile awareness.

### 6. Skill Creator UI
Interactive chat-based interface for building new Claude Code skills:
- Chat with Claude about what the skill should do
- Claude generates skill content with YAML frontmatter
- File preview and editing
- Draft → active promotion workflow
- File attachments for context

**For Arbor**: Relevant if adding task/skill template support. A guided creation flow is better than asking users to write markdown files manually.

## What Arbor Already Does Better

Superbot2 and Arbor serve completely different needs:

- **Developer visibility**: Arbor shows diffs, file trees, git status, terminal output. Superbot2 shows task lists and escalations.
- **Code-level tooling**: Arbor has full diff viewer, file browser, syntax highlighting. Superbot2 has none.
- **Terminal access**: Arbor has multi-backend PTY. Superbot2 runs agents in the background with no terminal UI.
- **Multi-provider**: Arbor supports 5 agent CLIs. Superbot2 is Claude-only.
- **Git worktree management**: Arbor's core feature. Superbot2 has zero worktree support.
- **Platform**: Arbor runs on all three platforms. Superbot2 is macOS-only.
- **Architecture**: Arbor is a single efficient Rust binary. Superbot2 is a patchwork of Bash scripts, Node.js, React, and Electron.
- **Remote**: Arbor has a proper REST/WebSocket daemon. Superbot2 has a local Express server.

## Key Takeaway

Superbot2 is the most philosophically different tool in the comparison set. It's not about monitoring or UI — it's about **autonomous agent project management**. The agents run continuously without human intervention, filing escalations when they need help.

The transferable ideas are about the **edges** of autonomous agent work:
1. **Escalation surfacing** — detect when agents are stuck and surface it prominently with suggested actions
2. **Completion enforcement** — hooks that ensure agents finish properly before signaling done
3. **Persistent agent memory** — per-repo context files that accumulate across sessions
4. **Messaging notifications** — send alerts to Telegram/Slack when agents need attention

These complement Arbor's core strengths (visibility, worktrees, terminals) by making the monitoring more actionable.

## Nine-Way Comparison Update

| Aspect | CodexMonitor | Superset | T3 Code | Jean | Conductor | Superterm | Polyscope | OpenWork | Superbot2 | Arbor |
|--------|-------------|----------|---------|------|-----------|-----------|-----------|----------|-----------|-------|
| **Paradigm** | Chat UI | Parallel agents | Chat UI | Chat+magic | Orchestrator | Monitor | Orchestrator | Agent GUI | Autonomous PM | Monitor+devtool |
| **Providers** | Codex | Any CLI+chat | Codex | Cl+Cx+OC | Cl+Cx | Any CLI | Cl+Cx | OpenCode | Claude only | Any CLI (5) |
| **Chat** | Full | Mastra | Lexical | Full+magic | Full | None | Full | Full | Dashboard chat | None (term) |
| **Terminal** | None | node-pty | node-pty | portable-pty | WebGL | tmux | Built-in | None | None | Multi-PTY |
| **Git** | Optional | Worktree | Optional | Worktree | Worktree | None | CoW clones | None | Minimal | Worktree |
| **Unique** | Threads | Browser+MCP | Event-src | Magic cmds | Checkpoints | Attention | Autopilot | Soul+Skills | Escalations | Remote daemon |
| **Autonomy** | Manual | Manual | Manual | Manual | Manual | Manual | Autopilot | Manual | Fully autonomous | Manual |
| **Platform** | mac/lin/win | macOS | mac/lin/win | macOS | macOS | lin/mac/wsl | macOS | mac/lin | macOS | mac/lin/win |
| **Open source** | Yes | No | Yes | Yes | No (free) | No ($250) | No | Yes | Yes | Yes |
