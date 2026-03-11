# Codex AutoRunner (CAR) vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: https://github.com/Git-on-my-level/codex-autorunner (open source, MIT)

## What Is CAR

"Low-opinion agent coordination tools for you to run long complex implementations using the agents you already love." A meta-harness for coding agents — not an agent itself, but an orchestration layer that automates running sequences of tasks ("tickets") against AI coding agents without human babysitting. You write a plan as markdown tickets, point CAR at them, and walk away.

Python-based, supports Codex and OpenCode as agent backends, extensible via plugin system.

## Architecture Overview

| | CAR | Arbor |
|---|---|---|
| **Language** | Python | Rust |
| **Architecture** | Engine → Control Plane → Adapters → Surfaces | arbor-core → arbor-gui / arbor-httpd / arbor-web-ui |
| **Agent Protocol** | AgentHarness protocol (ensure_ready/start_turn/stream_events/interrupt) | Terminal PTY sessions (provider-agnostic) |
| **Data Plane** | Filesystem (tickets, context docs) + SQLite (flow state) | File-based stores (TOML, JSON) |
| **Task Model** | Sequential ticket queue (`TICKET-###.md`) | Manual agent launch per worktree |
| **Worktree** | Hub mode manages git worktrees per repo | Core feature (worktree management) |
| **Terminal** | Web PTY websockets to agent TUIs | Multi-backend PTY |
| **Providers** | Codex, OpenCode (plugin-extensible) | Claude, Codex, Pi, OpenCode, Copilot |

## This Is Not a Direct Competitor

CAR and Arbor serve different roles:
- **CAR** = task automation harness ("run this queue of tickets unattended")
- **Arbor** = monitoring and management UI ("see what your agents are doing, interact with them")

They are complementary. CAR could run inside an Arbor-managed worktree. The interesting ideas are about making Arbor better at **automating** and **detecting problems** in agent runs.

## Transferable Ideas for Arbor

### 1. Loop Guard / No-Progress Detection
**What:** Track git fingerprint (HEAD SHA + `git status` hash) before and after each agent turn. If no change for N consecutive turns (default: 2), the agent is stuck.
**Why it matters:** Arbor already detects working/waiting states. Adding no-progress detection would surface stuck agents early — "this agent has been working for 10 minutes but hasn't changed any files."
**Implementation:** On each working→waiting transition, compare `git diff --stat` against previous snapshot. If identical N times, escalate the worktree status to "stuck" with a prominent indicator.

### 2. Dispatch / Pause / Reply (Human-in-the-Loop)
**What:** Two-mode notification system:
- `notify`: FYI message, agent continues working
- `pause`: Agent stops and waits for human reply before resuming
**Why it matters:** When running agents in parallel, you want to be notified when one needs a decision, and have a quick way to respond. Currently Arbor shows working/waiting dots but can't relay context about *why* the agent paused.
**Implementation:** Parse terminal output for common "waiting for input" patterns (permission prompts, questions). Surface as a notification with the question text. "Reply" action types into the terminal.

### 3. Ticket-as-Task Queue (`.arbor/tasks/`)
**What:** Markdown files as a task queue. Each ticket has YAML frontmatter (title, agent, model, done, context files). Agent picks up next incomplete ticket. Tickets can create sub-tickets.
**Why it matters:** Already in the roadmap as "Task Templates / Magic Commands" (Tier 1, #6). CAR's implementation is the most mature version of this pattern — tickets are inspectable, editable, and version-controlled.
**Implementation:** `.arbor/tasks/TASK-001.md` files. Frontmatter specifies agent preset, model override, done status. "Run next task" action in UI picks first non-done task and launches agent with its content as prompt.

### 4. Contextspace (Persistent Per-Worktree Knowledge)
**What:** Shared documents that persist across agent runs in a worktree:
- `active_context.md` — what the agent is currently working on
- `decisions.md` — architectural decisions made during this work
- `spec.md` — specification for the task
**Why it matters:** When agents restart or new sessions start in the same worktree, they lose context. Persistent context docs solve this.
**Implementation:** `.arbor/context/` directory in each worktree. Agents read these on startup (include in system prompt or as file references). Agents can update them during runs.

### 5. Auto-Commit After Agent Turns
**What:** Optionally auto-commit changes after each agent working→waiting transition, with retry logic if pre-commit hooks fail.
**Why it matters:** Creates natural checkpoints. Enables turn-by-turn diff viewing (Tier 2, #10 in roadmap). Prevents loss of work if agent crashes.
**Implementation:** Config option per worktree/repo. On agent state transition to waiting, run `git add -A && git commit -m "arbor: auto-checkpoint"`. Store as lightweight commits that can be squashed before PR.

### 6. Notification Routing (Multi-Channel)
**What:** Configurable notification channels per event type:
- `run_finished` → Telegram + desktop notification
- `run_error` → Discord + desktop notification
- `agent_stuck` → Telegram (pause mode, wait for reply)
**Why it matters:** Desktop notifications (Tier 1, #4) are the baseline. Adding Telegram/Discord/Slack routing lets users monitor from their phone without the web UI.
**Implementation:** Webhook-based notification system in arbor-httpd. Config in `.arbor.toml`: `[notifications]` section with channel URLs and event filters.

### 7. Circuit Breaker for Agent Backends
**What:** Classic circuit breaker pattern (closed→open→half-open) for agent backend resilience. If an agent CLI crashes N times, stop retrying and surface degradation.
**Why it matters:** When running multiple agents in parallel, one flaky backend shouldn't cause cascading failures or zombie processes.
**Implementation:** Track agent process exit codes per worktree. After N consecutive failures, mark worktree as "backend error" and stop auto-restart. Surface in UI with "retry" button.

### 8. Per-Ticket Model/Agent Overrides
**What:** YAML frontmatter in task files can override which agent and model to use:
```yaml
---
agent: codex
model: gpt-5.1-codex-mini
reasoning: high
---
```
**Why it matters:** Different tasks benefit from different agents/models. A quick fix might use a fast model; a complex refactor might use Opus.
**Implementation:** Task template frontmatter parsed by Arbor. When launching a task, override the default agent preset and model flag accordingly.

### 9. Agent Harness Protocol
**What:** Clean abstraction: `ensure_ready()`, `new_conversation()`, `start_turn()`, `stream_events()`, `interrupt()`. Agent-agnostic.
**Why it matters:** Arbor currently treats agents as terminal processes. A formal harness protocol would enable richer integration (structured events, turn tracking, interruption) without losing the terminal fallback.
**Implementation:** Trait in arbor-core:
```rust
trait AgentHarness {
    fn ensure_ready(&self) -> Result<()>;
    fn start_turn(&self, prompt: &str) -> Result<TurnHandle>;
    fn stream_events(&self) -> impl Stream<Item = AgentEvent>;
    fn interrupt(&self) -> Result<()>;
}
```
Terminal-based agents implement this generically (start_turn = write to PTY, stream_events = read PTY output). Richer agents (Codex app-server, OpenCode HTTP) can implement structured versions.

### 10. Prompt Budget Management
**What:** Intelligent prompt truncation when context exceeds model limits. Prioritized section shrinking that preserves structure (frontmatter kept intact, lower-priority context trimmed first).
**Why it matters:** Relevant if Arbor assembles prompts for task templates or magic commands.
**Implementation:** Not urgent for terminal-based approach. Relevant when adding composer/task-runner features.

## What This Doesn't Change in the Roadmap

CAR validates and reinforces several items already in `arbor-feature-roadmap.md`:
- **Task templates** (Tier 1 #6) — CAR's ticket system is the most mature version
- **Turn-by-turn diff snapshots** (Tier 2 #10) — CAR's auto-commit creates these naturally
- **Desktop notifications** (Tier 1 #4) — CAR adds multi-channel routing
- **Persistent agent memory** (Tier 4 #28) — CAR's contextspace is a concrete implementation

## New Items to Add to Roadmap

These are genuinely new ideas not previously captured:

1. **No-progress detection** → Add to Tier 2. Track git fingerprint across agent turns, surface "stuck" agents.
2. **Auto-checkpoint commits** → Add to Tier 3. Optional auto-commit on agent state transitions for safety and history.
3. **Circuit breaker for agent backends** → Add to Tier 3. Track crash rates, stop retrying flaky backends.
4. **Multi-channel notification routing** → Upgrade Tier 1 #4 from "desktop notifications" to "notification routing" with webhook support.
5. **Agent harness trait** → Add to Tier 4. Formal abstraction for richer agent integration beyond terminal.
