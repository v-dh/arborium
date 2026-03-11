# Polyscope vs Arbor — Deep Dive Comparison

Date: 2026-03-11
Source: https://getpolyscope.com/ (closed-source, website research only)

## What Is Polyscope

"The agent-first dev environment" / "The new cockpit." A native macOS app by Beyond Code that runs multiple AI coding agents in parallel, each in isolated copy-on-write repository clones. Not an IDE — a management/orchestration layer wrapping Claude Code and Codex CLI. Requires macOS 13.3+. Pricing appears freemium with a "Pro" tier (not publicly listed).

## Architecture Overview

| | Polyscope | Arbor |
|---|---|---|
| **Framework** | Native macOS app | GPUI (Rust native) + Web UI (TS + xterm.js) |
| **Platform** | macOS only (13.3+) | macOS, Linux, Windows |
| **Isolation** | Copy-on-write filesystem clones (~/.polyscope/clones/) | Git worktrees |
| **Agent Protocol** | Wraps Claude Code + Codex CLI | Terminal PTY sessions (provider-agnostic) |
| **Remote** | Encrypted relay server (E2EE, AES-256-GCM) | REST + WebSocket daemon |
| **Pricing** | Freemium (Pro tier) | Open source |

## Feature Comparison

| Feature | Polyscope | Arbor | Gap / Opportunity |
|---------|-----------|-------|-------------------|
| **Workspace Isolation** | Copy-on-write filesystem clones (only duplicates modified files) | Git worktrees (shared object store) | Different tradeoffs: CoW is simpler but uses more disk; worktrees share git objects |
| **Provider Support** | Claude Code + Codex CLI | Claude, Codex, Pi, OpenCode, Copilot | **Arbor wins** on breadth |
| **Chat UI** | Full chat with tool calls, follow-up messages while agent works | None (terminal-based) | Major gap |
| **Autopilot Mode** | Decomposes goals into numbered user stories (US-001...), sequential execution with progress tracking, play/pause/stop, crash recovery | None | Major unique feature |
| **Opinions (Multi-Model Consensus)** | Query multiple AI models on same question, get synthesized agreement/divergence/recommendation | None | Unique feature |
| **Visual Editor** | Web preview with element picker, click any element + describe changes in natural language | None | Unique for web development |
| **Workspace Linking** | Link workspaces for cross-repo context sharing, supervisor/worker patterns | None | Unique orchestration feature |
| **Tasks System** | One-click reusable prompts in `polyscope.json` (security review, test gen, code audit) | None | Similar to Jean's magic commands |
| **Diff Viewer** | Right panel: CHANGES/FILES tabs, Local vs Base toggle, per-file stats, inline commenting | Custom GPUI: changes tab + side-by-side diff | Comparable; Polyscope has inline commenting |
| **PR Creation** | "PR" button, agent handles commit/push/PR; draft PR support | Create PR via action menu | Polyscope handles full flow |
| **Merge** | Direct base-branch merge from UI | None | Gap |
| **CI Auto-Fix** | Monitor GitHub Actions, auto-send failures to agent for resolution | None | Significant workflow feature |
| **GitHub Issues** | Sidebar issue browser, workspace auto-linked to issues, auto-close on merge | PR auto-detection | Polyscope is deeper |
| **PR Checkout** | Check out existing PR branches into workspaces | None | Useful review workflow |
| **Plan Mode** | Agent proposes plan before implementing | None | Gap (also in Conductor, Jean, T3) |
| **Setup/Archive Scripts** | Via `polyscope.json`, auto-run on create/delete | None | Also in Conductor, Superset, Jean |
| **Model Selector** | Per-prompt model switching (Claude/Codex) | Fixed per agent preset | Gap |
| **`@` File References** | In chat composer | None | Chat feature |
| **Image Attachments** | Drag-and-drop/paste images in chat | None | Chat feature |
| **Command Palette** | Cmd+K with fuzzy search | None | Commonly requested |
| **Context Meter** | Token usage percentage indicator | None | Cost awareness |
| **Remote Access** | E2EE relay (AES-256-GCM, ECDH P-256), mobile optimized, zero-knowledge | REST + WebSocket daemon with bearer token | Polyscope's E2EE is more secure; Arbor's is more flexible |
| **Terminal** | Built-in (Cmd+backtick) | Multi-backend PTY, SSH, Mosh | **Arbor wins** on backends |
| **Platform** | macOS 13.3+ only | macOS/Linux/Windows | **Arbor wins** |
| **Open Source** | No (freemium) | Yes | **Arbor wins** |
| **Web Dashboard** | None (mobile via relay only) | Full web UI with terminal access | **Arbor wins** |

## Polyscope's Unique Features Worth Considering

### 1. Autopilot Mode (Story Decomposition)
Breaks a high-level goal into numbered user stories (US-001, US-002...). Executes sequentially with context carryover via `.context/progress.md`. Features:
- Play/pause/stop controls
- Edit story titles/descriptions/acceptance criteria inline
- Drag-and-drop reorder stories
- Crash recovery (resets in-progress → pending, auto-pauses)
- Max iteration limit (default 25)
- Can reference linked workspaces for context

**For Arbor**: Could implement as a "task breakdown" feature that creates a plan file, then executes steps via agent prompts. The progress tracking via a markdown file is a lightweight approach that doesn't require deep agent integration.

### 2. Opinions (Multi-Model Consensus)
Query multiple AI models simultaneously on a technical question:
1. Each model independently analyzes the codebase (read-only)
2. A synthesis pass produces: points of agreement, points of divergence, final recommendation

Use cases: architecture decisions, security reviews, implementation strategy.

**For Arbor**: Interesting but requires multiple API keys/providers. Could implement by launching parallel agent sessions with the same prompt and displaying results side-by-side.

### 3. Visual Editor (Web Preview + Element Picker)
Built-in web preview panel with:
- Element picker (crosshair icon) — click any element
- Describe changes in natural language
- Agent receives element context (tag, CSS, content, position)
- Supports text edits, styling, layout, component transforms

**For Arbor**: Would require embedding a browser (heavy, like Superset). Lower priority unless targeting web developers specifically.

### 4. Workspace Linking (Cross-Repo Context)
Link workspaces so agents can read each other's state:
- `+` icon in chat to link another workspace
- Works cross-repository
- Per-prompt basis (selective)
- Use cases: frontend+backend coordination, supervisor pattern

**For Arbor**: Could implement by mounting/reading other worktree paths. The per-prompt linking is harder without a chat UI, but could expose as terminal environment variables or agent context.

### 5. CI Auto-Fix
Monitors GitHub Actions. When checks fail:
1. Shows CI status in activity feed (passed/failed/skipped)
2. Offers "Auto fix" toggle
3. Agent reads failure details and attempts resolution automatically

**For Arbor**: Could implement by polling GitHub Actions status for PR branches and triggering a terminal prompt with failure context. Natural extension of existing PR detection.

### 6. Copy-on-Write Clones
Uses filesystem-level CoW instead of git worktrees. Only duplicates files that are actually modified.

**For Arbor**: Git worktrees are the better approach for most cases (shared object store, proper git integration). CoW clones are simpler conceptually but lose git worktree benefits (shared reflog, easier branch management).

### 7. Tasks System (Reusable Prompts)
Defined in `polyscope.json` as named, one-click-triggerable prompts:
- Security review
- Test generation
- Code audit
- Documentation
- Refactoring
Each runs in its own isolated workspace.

**For Arbor**: Similar to Jean's magic commands and Conductor's slash commands. Could support `.arbor.toml` with named tasks that launch agent sessions with predefined prompts.

## What Arbor Already Does Better

- **Multi-platform**: macOS/Linux/Windows vs macOS-only
- **Open source**: No vendor lock-in
- **Terminal backends**: SSH, Mosh for true remote terminals
- **Provider breadth**: 5 agents vs 2
- **Remote daemon**: Full REST/WebSocket API vs relay-only
- **Web dashboard**: Self-hosted, full terminal access
- **Git worktrees**: Proper git integration vs filesystem clones
- **Lightweight**: Single Rust binary, no runtime dependencies

## Priority Ideas from This Comparison

1. **CI status per worktree** — Show GitHub Actions pass/fail on worktree cards. "Send failure to agent" action. (Also in Conductor.)
2. **Workspace linking** — Let agents in one worktree read files from another. Useful for multi-repo projects.
3. **Task templates** — Predefined prompts in repo config (`.arbor.toml`) for common operations. One-click launch.
4. **Inline diff commenting** — Add comments to diff lines that can be sent as agent prompts. (Also in Conductor.)
5. **Autopilot/story decomposition** — Longer-term: break goals into steps with progress tracking.

## Seven-Way Comparison Update

| Aspect | CodexMonitor | Superset | T3 Code | Jean | Conductor | Superterm | Polyscope | Arbor |
|--------|-------------|----------|---------|------|-----------|-----------|-----------|-------|
| **Providers** | Codex | Any CLI + chat | Codex | Claude+Codex+OC | Claude+Codex | Any CLI | Claude+Codex | Any CLI (5) |
| **Chat UI** | Full | Mastra pane | Lexical | Full + magic | Full | None | Full | None (terminal) |
| **Terminal** | None | node-pty | node-pty | portable-pty | WebGL | tmux | Built-in | Multi-backend PTY |
| **Framework** | Tauri | Electron | Electron | Tauri | Native macOS | tmux+browser | Native macOS | GPUI (Rust) |
| **Isolation** | Optional | Worktree | Optional | Worktree | Worktree | None | CoW clones | Worktree |
| **Diff** | Live status | CodeMirror | Checkpoint | CodeMirror | Pierre engine | None | Built-in | GPUI custom |
| **GitHub** | Issues/PRs | PR status | PR resolve | Deep (15 ops) | CI+Actions+Issues | None | CI auto-fix | PR detection |
| **Unique** | Thread list | Browser+MCP | Event-sourced | Magic commands | Checkpoints+Teams | Attention system | Autopilot+Opinions | Remote daemon |
| **Remote** | TCP JSON-RPC | Cloud sync | WS bind | Axum HTTP/WS | None | HTTPS tunnel | E2EE relay | REST+WS daemon |
| **Platform** | macOS/Lin/Win | macOS | macOS/Lin/Win | macOS (tested) | macOS | Lin/macOS/WSL | macOS | macOS/Lin/Win |
| **Open source** | Yes | No | Yes | Yes | No (free) | No ($250/yr) | No (freemium) | Yes |
