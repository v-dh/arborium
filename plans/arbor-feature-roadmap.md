# Arbor Feature Roadmap — Synthesized from 10 Competitor Analyses

Date: 2026-03-11
Sources: CodexMonitor, Superset, T3 Code, Jean, Conductor, Superterm, Polyscope, OpenWork, Superbot2, Codex AutoRunner

## Arbor's Current Strengths (Keep / Protect)

These are where Arbor wins against all or most competitors:
- **Multi-platform** (macOS/Linux/Windows) — only T3 matches this; most are macOS-only
- **Multi-provider** (5 agent CLIs) — no competitor matches breadth
- **Native GPUI performance** — everyone else is Electron/Tauri/browser
- **Multi-backend terminal** (embedded, Alacritty, Ghostty, SSH, Mosh) — unique
- **Self-hosted remote daemon** (REST + WebSocket) — most flexible remote story
- **Open source, no cloud dependency** — Superset requires cloud, Conductor/Polyscope/Superterm are closed
- **Provider-agnostic passive monitoring** — detects agent state without hooks or API coupling

---

## Tier 1 — High Impact, Appears Across 4+ Competitors

These features are table stakes in the competitive landscape.

### 1. Setup/Teardown Scripts per Worktree
**Seen in:** Conductor, Superset, Jean, Polyscope
**What:** Run user-defined scripts on worktree creation (e.g., `cp .env .`, `bun install`, `npm install`) and deletion (cleanup). Configure in repo-level config file.
**Implementation:** Add `[scripts]` section to `.arbor.toml` or per-repo config. Execute on worktree create/delete. Pass env vars (`ARBOR_WORKTREE_PATH`, `ARBOR_REPO_PATH`, `ARBOR_BRANCH`).

### 2. Command Palette (Cmd+K)
**Seen in:** Conductor, Polyscope, OpenWork, Superset, Jean (magic modal)
**What:** Fuzzy search across actions, worktrees, branches, repos. Quick access to all operations.
**Implementation:** GPUI modal with text input, fuzzy matching against registered actions and worktree/repo names.

### 3. AI Commit Messages
**Seen in:** Jean, Conductor, T3 Code, Polyscope
**What:** Generate commit message from staged diff via a quick agent call.
**Implementation:** "Generate message" button in commit flow. Run `claude --print` or equivalent with the diff as context. Populate commit message field.

### 4. Notification Routing (Desktop + Webhooks)
**Seen in:** Superset, Jean, Superterm, Superbot2, CAR
**What:** OS-level notification when an agent transitions from working → waiting. Optionally route to Telegram/Discord/Slack via webhooks for mobile awareness.
**Implementation:** Already detect the state change. Add `notify-rust` or OS notification API call on transition. Add optional webhook config in `.arbor.toml`: `[notifications]` section with channel URLs and event filters (agent_finished, agent_stuck, agent_error). Desktop notifications are the baseline; webhooks are opt-in.

### 5. CI Status per Worktree
**Seen in:** Conductor, Polyscope, Jean, Superset
**What:** Show GitHub Actions pass/fail/pending on worktree cards. Already have PR detection — extend to check status.
**Implementation:** Poll GitHub API for check runs on the PR's head SHA. Show status icon on worktree card (green check, red X, yellow dot).

### 6. Task Templates / Magic Commands
**Seen in:** Jean (magic commands), Polyscope (tasks), Conductor (slash commands), OpenWork (skills)
**What:** Predefined, reusable prompt templates for common operations: investigate issue, code review, generate tests, security audit. One-click launch into an agent session.
**Implementation:** `.arbor/tasks/` directory with markdown files. Each file = a task template with name, description, prompt. Show in command palette or dedicated UI. Launch by starting agent with prompt pre-filled.

---

## Tier 2 — High Impact, Appears Across 2-3 Competitors

### 7. Stacked Git Actions (Commit + Push + PR)
**Seen in:** T3 Code, Jean
**What:** Combine commit → push → create PR into a single flow with per-step progress feedback. One button for the full flow.
**Implementation:** Sequential action pipeline in the quick actions menu. Show progress: "Committing... Pushing... Creating PR... Done."

### 8. PR-Based Worktree Creation
**Seen in:** Superset, T3 Code, Jean, Polyscope, Conductor
**What:** Paste a PR URL/number → fetch branch info → create worktree for reviewing it.
**Implementation:** "Review PR" action in command palette or worktree creation dialog. Use `gh pr view` to get branch, then create worktree.

### 9. Richer PR Tracking
**Seen in:** Superset, Jean, Conductor, Polyscope
**What:** Show CI status, review decision (approved/changes requested), additions/deletions, ahead/behind counts on worktree cards.
**Implementation:** Extend existing PR detection with more GitHub API data. Show as badges/icons on worktree cards.

### 10. Turn-by-Turn Diff Snapshots + No-Progress Detection
**Seen in:** Conductor, T3 Code, Polyscope, CAR
**What:** Capture git diff state when agent transitions working → waiting. Show "what changed this turn" vs cumulative. Detect stuck agents when no files change across N consecutive turns.
**Implementation:** On agent state transition, run `git diff --stat` and store the snapshot. Compare against previous snapshot — if identical for 2+ consecutive turns, mark worktree as "stuck" with a prominent indicator. Display change history on worktree card or in a history view.

### 11. Compact Sidebar Mode
**Seen in:** CodexMonitor (lightweight thread list)
**What:** Toggle between current card view and a compact list showing just: worktree name + branch + status dot + timestamp. Collapse diff stats and details into hover.
**Implementation:** Add a view mode toggle. Compact mode renders a simple list row per worktree.

### 12. Session Listing via Provider Traits
**Seen in:** CodexMonitor (thread/list), Jean (multi-session)
**What:** List past chat sessions per worktree, grouped by provider. Show session title, timestamp, message count.
**Implementation:** Define `AgentSessionProvider` trait. Implement for Claude Code (read `~/.claude/projects/`), Codex (parse session JSONL), OpenCode. Display in sidebar or worktree detail.

### 13. Execution Mode Toggle
**Seen in:** Jean (Plan/Build/Yolo), T3 Code (Full/Supervised), Conductor (access modes)
**What:** Per-session toggle for agent autonomy level. Maps to CLI flags.
**Implementation:** Dropdown or toggle on worktree card or terminal header. Modifies the agent launch command (e.g., add/remove `--dangerously-skip-permissions`, `--plan`).

---

## Tier 3 — Medium Impact, Differentiators

### 14. Richer Attention Indicators
**Seen in:** Superterm (sparklines + colored orbs)
**What:** Enhance working/waiting dots with: mini activity sparklines (terminal output rate), more states (errored, finished, waiting-for-input, idle), color-coded severity.
**Implementation:** Track terminal output rate over time windows. Render sparkline on worktree card. Map agent states to specific colors.

### 15. Repo-Level Config (`.arbor.toml`)
**Seen in:** Conductor (`conductor.json`), Polyscope (`polyscope.json`), Superset (`.superset/config.json`)
**What:** Shared team configuration committed to repo: setup scripts, task templates, branch prefix patterns, default agent preset.
**Implementation:** Read `.arbor.toml` from repo root. Merge with user config. Support: `[scripts]`, `[tasks]`, `[branch]`, `[agent]` sections.

### 16. Per-Worktree Notes
**Seen in:** Conductor (Notes tab), Superterm (Logbook), Superbot2 (knowledge files)
**What:** Simple markdown note per worktree for tracking what each agent is supposed to do, decisions made, blockers.
**Implementation:** Store as `.arbor/notes.md` in worktree or as metadata in Arbor's data dir. Show in right panel or worktree detail.

### 17. Port Detection per Worktree
**Seen in:** Superset
**What:** Detect listening TCP ports from terminal process trees. Show on worktree card. Click to open in browser.
**Implementation:** Scan `/proc/net/tcp` or `lsof -i` filtered by process group. Display port badges on worktree cards.

### 18. Branch Prefix Modes
**Seen in:** Superset, Conductor
**What:** Auto-generate branch names with configurable prefix: GitHub username, git author, custom string.
**Implementation:** Config option in `.arbor.toml` or user settings. Apply on worktree creation.

### 19. Workspace Status Labels
**Seen in:** Conductor (kanban: backlog/in-progress/review/done), Jean (colored labels)
**What:** Optional status labels on worktrees for visual organization of parallel work.
**Implementation:** Simple enum (active/review/done) or free-form colored labels. Stored in worktree metadata. Filterable in sidebar.

### 20. Clone-Based Agent Creation
**Seen in:** CodexMonitor ("New Clone Agent")
**What:** Full `git clone` into a sandbox directory for cases where full isolation is needed (can't share git objects).
**Implementation:** "New Clone Agent" action that clones the repo, creates a fresh branch, and launches an agent.

### 21. Auto-Checkpoint Commits
**Seen in:** CAR (auto-commit after agent turns)
**What:** Optionally auto-commit changes after each agent working→waiting transition. Creates natural checkpoints, enables turn-by-turn diff viewing, prevents work loss on crashes.
**Implementation:** Config option per worktree/repo. On agent state transition to waiting, run `git add -A && git commit -m "arbor: auto-checkpoint"`. Store as lightweight commits that can be squashed before PR.

### 22. Circuit Breaker for Agent Backends
**Seen in:** CAR
**What:** Track agent process exit codes per worktree. After N consecutive crashes, stop auto-retrying and surface degradation. Prevents zombie processes and cascading failures when running many agents.
**Implementation:** Count consecutive non-zero exit codes per worktree. After threshold (e.g., 3), mark as "backend error" in UI. Offer manual "retry" button. Reset counter on success.

---

## Tier 4 — Nice-to-Have / Longer-Term

### 21. Inline Diff Commenting
**Seen in:** Conductor, Polyscope
**What:** Add comments to diff lines that become agent prompts ("fix this", "why was this changed").

### 22. Hybrid Terminal + Composer
**Seen in:** CodexMonitor, Jean, T3 Code, Polyscope, Conductor (all have chat UIs)
**What:** Optional composer overlay that wraps agent CLIs with structured output parsing. Terminal remains the default; composer adds richer UX.

### 23. Token Usage / Cost Tracking
**Seen in:** CodexMonitor, Conductor, Polyscope
**What:** Parse provider session files for token usage, display context ring / daily/weekly totals.

### 24. Privacy Mask Mode
**Seen in:** Superterm
**What:** One-click toggle that redacts API keys/tokens/passwords in terminal output. Useful for screen sharing.

### 25. Escalation Surfacing (Dispatch/Pause/Reply)
**Seen in:** Superbot2, CAR
**What:** When agent transitions to waiting, parse terminal output to detect if it's asking a question or permission prompt. Surface prominently with context. "Reply" action types directly into the terminal. Two modes: notify (FYI) and pause (needs response).

### 26. Scheduled Agent Sessions
**Seen in:** OpenWork, Superbot2
**What:** Cron-triggered agent work via httpd daemon (automated code review, dependency updates, test runs).

### 27. Workspace Linking (Cross-Worktree Context)
**Seen in:** Polyscope
**What:** Let agents in one worktree read files from another for multi-repo coordination.

### 28. Persistent Agent Memory per Repo (Contextspace)
**Seen in:** Superbot2 (MEMORY.md, IDENTITY.md), CAR (active_context.md, decisions.md, spec.md)
**What:** `.arbor/context/` directory with files that accumulate knowledge across agent sessions. `active_context.md` for current work, `decisions.md` for architectural choices, `spec.md` for task specifications. Agents read on startup and update during runs.

### 29. Cold Restore for GUI Terminals
**Seen in:** Superset
**What:** Persist terminal scrollback across app restarts.

### 30. Mosaic Pane Layout
**Seen in:** Superset
**What:** Arbitrary horizontal/vertical splits for terminals, diffs, file views.

### 31. Agent Harness Trait
**Seen in:** CAR (AgentHarness protocol)
**What:** Formal abstraction for agent lifecycle beyond terminal PTY: `ensure_ready()`, `start_turn()`, `stream_events()`, `interrupt()`. Terminal agents implement generically (write/read PTY). Richer agents (Codex app-server, OpenCode HTTP) can provide structured events and turn tracking.

### 32. Task Queue Runner
**Seen in:** CAR (ticket queue)
**What:** Sequential execution of `.arbor/tasks/TASK-###.md` files. Agent picks up next non-done task, runs it, marks done. Enables "write a plan, walk away" workflows.

---

## What NOT to Build (Low Value or Wrong Direction)

- **Built-in browser / visual editor** (Superset, Polyscope) — too heavy, not core to worktree management
- **Cloud sync / billing** (Superset, OpenWork) — contradicts self-hosted positioning
- **Full chat protocol reimplementation** (CodexMonitor, Jean) — high maintenance; hybrid terminal + optional composer is better
- **Messaging bridges** (OpenWork, Superbot2) — web UI already covers mobile access
- **Agent teams / multi-agent coordination** (Conductor, Superbot2) — let agent CLIs handle this
- **Linear/Jira integration** (Jean, Conductor) — niche; GitHub issues via `gh` is sufficient
- **Autopilot / story decomposition** (Polyscope) — complex, let agents self-organize
- **Soul mode / persistent identity** (OpenWork, Superbot2) — too opinionated for a monitoring tool
