# Superterm vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: https://superterm.dev/ (closed-source, website research only)

## What Is Superterm

"The terminal for the agentic era." A session-aware terminal dashboard built on top of tmux by OpenFaaS Ltd. Solves "Agentic Attention Deficit Disorder" — monitoring which of your many running agents needs attention, errored, finished, or is waiting for input. Not a terminal emulator; it's a monitoring/command layer above tmux sessions. $250/year after 14-day trial.

## Architecture Overview

| | Superterm | Arbor |
|---|---|---|
| **Architecture** | tmux wrapper with browser UI | GPUI native app + Web UI + HTTP daemon |
| **Platform** | Linux, macOS, Windows (WSL2) — daemon + any browser | macOS, Linux, Windows — native GUI |
| **Agent Protocol** | Agent-agnostic (reads terminal output directly) | Agent-agnostic (terminal PTY sessions) |
| **Terminal** | tmux sessions viewed through browser | Multi-backend PTY (embedded, Alacritty, Ghostty, SSH, Mosh) |
| **Git/Code** | None | Full worktree management, diff viewer, file tree |
| **Pricing** | $250/year | Open source |

## Feature Comparison

| Feature | Superterm | Arbor | Gap / Opportunity |
|---------|-----------|-------|-------------------|
| **Agent Attention System** | Sparkline activity graphs, colored status orbs (red/amber/green), bell detection, output burst monitoring, idle detection | Working/waiting dots per worktree | Superterm's attention indicators are richer |
| **Agent Hook System** | `superterm agent-setup` auto-configures native hooks for Claude/Codex/Amp/OpenCode; `superterm notify` for custom notifications | Passive session file detection | Both provider-agnostic; different mechanisms |
| **Agent Support** | Claude, Codex, Gemini, Amp, OpenCode, Aider, Qwen, any CLI | Claude, Codex, Pi, OpenCode, Copilot | Similar breadth |
| **Logbook** | Per-session Notes/Timeline/Prompts tabs with Now/This Week/Horizon goal tracking | None | Unique context management feature |
| **Mobile Access** | Browser-based UI optimized for phone: approve permissions, send follow-ups, check progress | Web UI (responsive) | Both have mobile-accessible UIs |
| **Privacy/Mask Mode** | One-click mask hides credentials across all sessions | None | Nice security feature |
| **PWA Support** | Installable as dock icon without browser chrome | N/A (native app) | Superterm's PWA is for their web approach |
| **Session Persistence** | tmux sessions survive restarts (independent daemon) | Daemon terminal sessions survive restarts | Comparable |
| **Git/Worktree** | None | Full worktree management, create/delete, branch tracking | **Arbor wins significantly** |
| **Diff Viewer** | None | Side-by-side diff, changes panel, file tree | **Arbor wins significantly** |
| **GitHub Integration** | None | PR detection, GitHub OAuth, PR creation | **Arbor wins** |
| **File System** | None | File tree, file viewer with syntax highlighting | **Arbor wins** |
| **Remote Access** | HTTPS tunnel (inlets, Cloudflare, ngrok, Tailscale) with 32-char access key | Full HTTP daemon + REST API + WebSocket with bearer token | Both strong; Arbor's is richer |
| **Headless Hardware** | Pitched for cheap Linux boxes ($300 Mini PC, 37€/mo Hetzner) | Works on any platform with httpd daemon | Similar capability |
| **Clipboard** | Universal copy/paste across machines via browser | N/A (native app handles clipboard) | Superterm's cross-machine clipboard is unique |
| **Native App** | No (browser-based) | Yes (GPUI native) | **Arbor wins** on native experience |
| **Open Source** | No ($250/year) | Yes | **Arbor wins** |

## Superterm's Unique Ideas Worth Considering

### 1. Attention System (Sparkline + Status Orbs)
Per-session sparkline activity graphs show recent terminal activity as mini bar charts. Colored orbs indicate state at a glance: red (errored), amber (waiting), green (active/idle). Detects: bell characters, output bursts, idle periods.

**For Arbor**: The current working/waiting dots are a simpler version. Could enhance with:
- Mini activity sparklines on worktree cards (last N minutes of terminal activity)
- More status states: errored, finished, waiting-for-input, active, idle
- Bell character detection in terminal output as a signal

### 2. Logbook (Notes/Timeline/Prompts per Session)
Each session has a context tracking panel with three tabs:
- **Notes**: Free-form notes about the task
- **Timeline**: Activity timeline
- **Prompts**: Prompts sent to the agent
Plus goal tracking across Now/This Week/Horizon timeframes.

**For Arbor**: Could add a lightweight notes/context panel per worktree. Even a simple markdown note per worktree would be useful for tracking what each parallel agent is supposed to do.

### 3. Privacy Mask Mode
One-click toggle that hides sensitive credentials and API keys across all sessions. Useful for screen sharing, demos, pair programming.

**For Arbor**: Could implement as a terminal filter that redacts patterns matching API keys, tokens, passwords when mask mode is enabled.

### 4. Agent Hook Integration
`superterm agent-setup` auto-configures hooks for supported agents. Enables agents to notify Superterm of state changes without Superterm parsing output.

**For Arbor**: Already does passive detection from session files. Could additionally support agent notification hooks (like Superset's Express server approach) for more immediate state updates.

### 5. Headless Remote Pattern
Run agents on cheap headless hardware, monitor from expensive laptop or phone. Explicitly pitched as a cost optimization.

**For Arbor**: Already possible with httpd daemon + web UI. Could make this pattern more explicit in docs/marketing.

## What Arbor Already Does Better

Superterm is intentionally minimal — a tmux monitor with agent awareness. Arbor is a fundamentally different (and much more complete) product:

- **Full worktree management**: Create, delete, track, diff — Superterm has zero git awareness
- **Diff viewer**: Side-by-side code diffs — Superterm doesn't show code at all
- **File system**: File tree, file viewer — Superterm doesn't access files
- **GitHub integration**: PR detection, creation, OAuth — Superterm has none
- **Native GUI**: GPUI-rendered desktop app — Superterm is browser-only
- **Open source**: Free forever — Superterm is $250/year
- **Multi-platform native**: Native binaries on all platforms — Superterm requires tmux + browser

## Key Takeaway

Superterm validates that **agent attention management** is a real problem worth solving well. Its sparkline activity graphs, multi-state status indicators, and logbook context tracking are focused solutions to the "which agent needs me?" problem. Arbor already has the building blocks (activity detection, terminal monitoring) but could polish the attention UX with richer visual indicators.

The rest of Superterm's feature set is a subset of what Arbor already provides. The main gap is Arbor's attention indicators could be more expressive.
