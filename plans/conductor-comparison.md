# Conductor vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: https://www.conductor.build/ (closed-source, website research only)

## What Is Conductor

"Run a team of coding agents on your Mac." A native macOS app by Melty Labs ($2.8M seed) for orchestrating multiple Claude Code and Codex agents in parallel across isolated git worktrees. Positions itself as the first "AI orchestrator." Used at Linear, Vercel, Ramp, Notion, Stripe. Current version 0.38.4. Currently free (may charge for collaboration features later).

## Architecture Overview

| | Conductor | Arbor |
|---|---|---|
| **Framework** | Native macOS app (likely Swift/AppKit) | GPUI (Rust native) + Web UI (TS + xterm.js) |
| **Platform** | macOS only (Win/Linux in development) | macOS, Linux, Windows |
| **Agent Protocol** | Bundles Claude Code + Codex, deep integration | Terminal PTY sessions (provider-agnostic) |
| **Diff** | "Pierre" diff engine with inline commenting | Custom GPUI renderer (side-by-side) |
| **Terminal** | WebGL-rendered embedded terminal | Multi-backend PTY (embedded, Alacritty, Ghostty, SSH, Mosh) |
| **Remote** | None (local-only) | Full HTTP daemon + WebSocket |
| **Pricing** | Free (seed-funded, future collaboration tier) | Open source |

## Feature Comparison

| Feature | Conductor | Arbor | Gap / Opportunity |
|---------|-----------|-------|-------------------|
| **Provider Support** | Claude Code + Codex (bundled), alternative providers via env vars (OpenRouter, Bedrock, Vertex, Azure) | Claude, Codex, Pi, OpenCode, Copilot (terminal presets) | Different approaches — Conductor bundles agents; Arbor wraps CLIs |
| **Chat UI** | Full chat with tool call display, Mermaid diagrams, LaTeX, inline code blocks, token cost display | None (terminal-based) | Major gap |
| **Workspace Status Board** | Workspaces organized by: backlog, in progress, review, done | Worktree cards with activity dots | Conductor has a kanban-like workflow |
| **Checkpoints** | Git-based snapshots per agent turn, revert to any turn | None | Valuable for safe experimentation |
| **Turn-by-Turn Diffs** | See exactly what each agent turn changed, historical navigation | Single diff view | Gap — similar to T3's checkpoint diffs |
| **Inline Diff Commenting** | Multiline comments with markdown on diff lines | None | Useful for review workflows |
| **Diff Viewer** | Pierre engine: unified view, mark-as-viewed, inline commenting, GitHub comment sync | Custom GPUI side-by-side | Conductor is richer |
| **CI/CD Integration** | View GitHub Actions logs, re-run actions, forward failures to agent for auto-fix | None | Significant gap |
| **Checks Tab** | Unified view: git status + deployments + CI results + todos | None | Unique workflow dashboard |
| **Linear Integration** | Create workspace from Linear issue, deep links | None | Enterprise feature |
| **GitHub Issues** | Create workspace from issue, attach issues in chat | PR auto-detection | Conductor is deeper |
| **Agent Teams** | Experimental: multiple agents coordinating on related tasks | Multiple agents run independently | Conductor's agent coordination is unique |
| **Plan Mode** | Interactive planning with questions before execution | None | Gap |
| **Hand Off Plans** | Pass plans between different agent instances | None | Unique orchestration feature |
| **Workspace Forking** | Fork a workspace with chat summary carried over | None | Nice for "try different approach" |
| **Todos** | Per-workspace todos that block merge until completed | None | Merge safety feature |
| **Notes Tab** | WYSIWYG scratchpad per workspace | None | Nice-to-have |
| **Zen Mode** | Distraction-free mode (Ctrl+Z) | None | Minor |
| **Setup/Archive Scripts** | Run on workspace create/delete via `conductor.json` | None | Also in Superset and Jean |
| **Run Scripts** | Launch dev servers with `$CONDUCTOR_PORT` variable | None | Useful for dev server management |
| **Git Sparse Checkout** | Monorepo directory isolation | None | Enterprise monorepo feature |
| **Graphite Integration** | Stack visualization for stacked PRs | None | Niche but valuable |
| **Context Meter** | Token usage with breakdown on hover | None | Cost awareness |
| **Thinking Toggle** | Alt+T with customizable defaults | None | Quick thinking level switching |
| **Command Palette** | Cmd+K with fuzzy search across commands/workspaces/branches | None | Commonly requested |
| **File Picker** | Cmd+P to find files in workspace | File tree in right pane | Different approaches |
| **Slash Commands** | Custom `.claude/commands/` integration | None | Claude ecosystem integration |
| **`@todos` in Composer** | Inform agent of pending tasks | None | Nice context feature |
| **Branch Prefix Config** | Customizable branch prefixes | Manual branch naming | Also in Superset |
| **Terminal** | WebGL-rendered, Claude reads output, Cmd+F search | Multi-backend PTY, SSH, Mosh | **Arbor wins** on backends |
| **Remote/Daemon** | None (local only) | Full REST + WebSocket daemon | **Arbor wins** |
| **Platform** | macOS only | macOS/Linux/Windows | **Arbor wins** |
| **Open Source** | No (free, seed-funded) | Yes | **Arbor wins** |
| **Web UI** | None | Separate responsive web dashboard | **Arbor wins** |

## Conductor's Unique Features Worth Considering

### 1. Checkpoint / Turn-by-Turn Diffs
Git-based snapshots at each agent turn, stored as private git refs. Users can:
- Navigate forward/backward through agent turns
- See exactly what changed at each turn
- Revert to any previous checkpoint

**For Arbor**: Could implement by snapshotting `git stash create` or `git diff --stat` at agent state transitions (working→waiting). Lower-cost alternative to full git refs.

### 2. Workspace Status Board (Kanban)
Workspaces organized by status: backlog → in progress → review → done. Visual workflow management for parallel agent work.

**For Arbor**: Could add optional status labels to worktrees (similar to Jean's labels but with kanban-like progression).

### 3. CI/CD Checks Integration
Unified Checks tab showing git status, deployments, CI results, and todos. Can view GitHub Actions logs, re-run actions, and forward failures to agents for auto-fix.

**For Arbor**: Already has GitHub integration. Could extend to show CI status per worktree and enable "send failure to agent" workflow.

### 4. Agent Teams (Experimental)
Multiple agents coordinating on related tasks within a workspace. Still experimental but points toward multi-agent orchestration as a differentiator.

**For Arbor**: Future consideration. Current multi-terminal approach already allows parallel agents; formal coordination is a bigger undertaking.

### 5. `conductor.json` Team Config
Shared configuration committed to repo: setup scripts, run scripts, branch prefixes, etc. Team members get consistent workspace setup.

**For Arbor**: Could support a `.arbor.toml` or similar repo-level config for shared team settings.

### 6. Fork Workspace
Create a copy of a workspace (with chat summary) to try a different approach. Original preserved as fallback.

**For Arbor**: Natural extension of worktree creation — "branch from worktree" with context preservation.

## What Arbor Already Does Better

- **Multi-platform**: macOS/Linux/Windows vs macOS-only
- **Open source**: Community-driven, no funding dependency
- **Remote daemon**: Full REST/WebSocket API for remote access
- **Terminal backends**: SSH, Mosh for true remote terminal
- **Provider breadth**: 5 agents + any CLI vs Claude Code + Codex only
- **Web dashboard**: Accessible from any browser
- **Self-hosted**: No vendor dependency, no telemetry concerns
- **Native Rust**: GPUI performance vs whatever Conductor uses

## Priority Ideas from This Comparison

1. **Turn-by-turn diff snapshots** — Capture git state at agent state transitions. Most requested feature across competitors.
2. **CI status per worktree** — Show GitHub Actions pass/fail on worktree cards. Extend existing GitHub integration.
3. **Command palette (Cmd+K)** — Fuzzy search across actions/worktrees/branches. Also requested in Superset comparison.
4. **Workspace status labels** — Simple status progression (backlog/active/review/done) for organizing parallel work.
5. **Repo-level config** (`.arbor.toml`) — Setup scripts, run commands, branch prefix config shared via repo.
