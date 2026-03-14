import { restartProcess, startProcess, stopProcess } from "../api";
import type { ProcessInfo, Worktree } from "../types";
import { changeKindInfo, el, formatAge } from "../utils";
import { openCreateWorktreeModal } from "./create-worktree-modal";
import {
  refresh,
  refreshIssues,
  selectedIssueRepoRoot,
  setActiveSession,
  setRightPaneTab,
  setRightPanelTab,
  state,
  subscribe,
} from "../state";

export function createChangesPanel(): HTMLElement {
  const panel = el("div", "changes-panel");
  panel.setAttribute("data-testid", "changes-panel");

  function render(): void {
    panel.replaceChildren();
    const worktree = selectedWorktree();

    panel.append(renderPrimaryTabs());

    if (state.rightPanelTab === "issues") {
      panel.append(renderIssuesContent());
      return;
    }

    if (worktree === undefined) {
      panel.append(el("div", "changes-empty", "Select a worktree"));
      return;
    }

    panel.append(renderChangesTabs(worktree));

    if (state.rightPaneTab === "procfile") {
      panel.append(renderProcfileContent(worktree));
      return;
    }

    panel.append(renderChangesContent());
  }

  subscribe(render);
  render();
  return panel;
}

function selectedWorktree(): Worktree | undefined {
  if (state.selectedWorktreePath === null) {
    return undefined;
  }
  return state.worktrees.find((entry) => entry.path === state.selectedWorktreePath);
}

function renderPrimaryTabs(): HTMLElement {
  const tabs = el("div", "changes-tabs");
  tabs.append(
    renderPanelTabButton("Changes", "changes", state.changedFiles.length),
    renderPanelTabButton("Issues", "issues", state.issues.length),
  );
  return tabs;
}

function renderPanelTabButton(
  label: string,
  tab: "changes" | "issues",
  count: number,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = "changes-tab-button";
  button.type = "button";
  if (state.rightPanelTab === tab) {
    button.classList.add("active");
  }

  const content = el("span", "changes-tab-content");
  content.append(
    el("span", "changes-tab-label", label),
    el("span", "changes-tab-count", String(count)),
  );

  button.append(content);
  button.addEventListener("click", () => setRightPanelTab(tab));
  return button;
}

function renderChangesTabs(worktree: Worktree): HTMLElement {
  const tabs = el("div", "changes-tabs changes-subtabs");
  tabs.append(
    renderPaneTabButton("Changes", "changes"),
    renderPaneTabButton("Processes", "procfile", worktree.processes.length),
  );
  return tabs;
}

function renderPaneTabButton(
  label: string,
  tab: "changes" | "procfile",
  count?: number,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = "changes-tab-button";
  button.type = "button";
  if (state.rightPaneTab === tab) {
    button.classList.add("active");
  }

  const content = el("span", "changes-tab-content");
  content.append(el("span", "changes-tab-label", label));
  if (count !== undefined) {
    content.append(el("span", "changes-tab-count", String(count)));
  }

  button.append(content);
  button.addEventListener("click", () => setRightPaneTab(tab));
  return button;
}

function renderChangesContent(): HTMLElement {
  const wrapper = el("div", "changes-pane-body");
  const header = el("div", "changes-header");
  header.append(
    el("h3", "changes-title", "Changes"),
    el("span", "changes-count", String(state.changedFiles.length)),
  );
  wrapper.append(header);

  if (state.changedFiles.length === 0) {
    wrapper.append(el("div", "changes-empty", "No changes"));
    return wrapper;
  }

  const list = el("ul", "changes-list");
  for (const file of state.changedFiles) {
    const info = changeKindInfo(file.kind);
    const item = el("li", "changes-item");

    const statusBadge = el("span", "changes-status");
    statusBadge.textContent = info.code;
    statusBadge.style.color = info.color;

    const pathEl = el("span", "changes-path", file.path);
    pathEl.title = file.path;

    const stats = el("span", "changes-stats");
    if (file.additions > 0) {
      stats.append(el("span", "changes-additions", `+${file.additions}`));
    }
    if (file.deletions > 0) {
      stats.append(el("span", "changes-deletions", `-${file.deletions}`));
    }

    item.append(statusBadge, pathEl, stats);
    list.append(item);
  }

  wrapper.append(list);
  return wrapper;
}

function renderIssuesContent(): HTMLElement {
  const repoRoot = selectedIssueRepoRoot();
  if (repoRoot === null) {
    return el("div", "changes-empty", "Select a repository");
  }

  if (state.issuesLoading && state.issues.length === 0) {
    return el("div", "changes-empty", "Loading issues…");
  }

  if (state.issuesError !== null) {
    return el("div", "changes-empty changes-empty-error", state.issuesError);
  }

  if (state.issuesNotice !== null) {
    return el("div", "changes-empty", state.issuesNotice);
  }

  const wrapper = el("div", "issues-panel");
  const source = el("div", "issues-source");
  const sourceLabel = state.issueSource !== null
    ? `${state.issueSource.label} · ${state.issueSource.repository}`
    : repoRoot;
  source.append(el("span", "issues-source-label", sourceLabel));

  const actions = el("div", "issues-source-actions");
  if (state.issueSource?.url !== null) {
    const link = document.createElement("a");
    link.className = "issues-source-link";
    link.href = state.issueSource.url;
    link.target = "_blank";
    link.rel = "noopener";
    link.textContent = "Open";
    actions.append(link);
  }

  const refreshButton = el("button", "changes-action-btn", "Refresh");
  refreshButton.type = "button";
  refreshButton.disabled = state.issuesLoading;
  refreshButton.addEventListener("click", () => {
    refreshIssues(repoRoot, true);
  });
  actions.append(refreshButton);

  source.append(actions);
  wrapper.append(source);

  if (state.issues.length === 0) {
    wrapper.append(el("div", "changes-empty", "No open issues"));
    return wrapper;
  }

  const list = el("div", "issues-list");
  for (const issue of state.issues) {
    const linkedReview = issue.linked_review;
    const linkedBranch = issue.linked_branch;
    const issueActionLabel = linkedReview !== null
      ? linkedReview.kind === "merge_request"
        ? "MR exists"
        : "PR exists"
      : linkedBranch !== null
        ? "Branch exists"
        : "Create worktree";
    const item = el("article", "issue-item");
    item.setAttribute("role", "button");
    item.tabIndex = 0;
    item.addEventListener("click", () => openCreateWorktreeModal(issue));
    item.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        openCreateWorktreeModal(issue);
      }
    });

    const topRow = el("div", "issue-item-top");
    topRow.append(
      el("span", "issue-display-id", issue.display_id),
      el("span", "issue-title", issue.title),
    );

    if (issue.url !== null) {
      const link = document.createElement("a");
      link.className = "issue-link";
      link.href = issue.url;
      link.target = "_blank";
      link.rel = "noopener";
      link.textContent = "Open";
      link.addEventListener("click", (event) => {
        event.stopPropagation();
      });
      topRow.append(link);
    }

    item.append(topRow);

    if (linkedReview !== null || linkedBranch !== null) {
      const linkedRow = el("div", "issue-linked");

      if (linkedReview !== null) {
        if (linkedReview.url !== null) {
          const reviewLink = document.createElement("a");
          reviewLink.className = "issue-linked-chip issue-linked-review";
          reviewLink.href = linkedReview.url;
          reviewLink.target = "_blank";
          reviewLink.rel = "noopener";
          reviewLink.textContent = linkedReview.label;
          reviewLink.addEventListener("click", (event) => {
            event.stopPropagation();
          });
          linkedRow.append(reviewLink);
        } else {
          linkedRow.append(
            el("span", "issue-linked-chip issue-linked-review", linkedReview.label),
          );
        }
      }

      if (linkedBranch !== null) {
        linkedRow.append(el("span", "issue-linked-chip issue-linked-branch", linkedBranch));
      }

      item.append(linkedRow);
    }

    const bottomRow = el("div", "issue-item-bottom");
    bottomRow.append(
      el("span", "issue-state", issue.state),
      el(
        "span",
        "issue-age",
        issue.updated_at === null ? "recently" : formatIssueAge(issue.updated_at),
      ),
      el("span", "issue-cta", issueActionLabel),
    );

    item.append(bottomRow);
    list.append(item);
  }

  wrapper.append(list);
  return wrapper;
}

function renderProcfileContent(worktree: Worktree): HTMLElement {
  const wrapper = el("div", "changes-pane-body");
  const header = el("div", "changes-header");
  header.append(
    el("h3", "changes-title", "Processes"),
    el("span", "changes-count", String(worktree.processes.length)),
  );
  wrapper.append(header);

  if (worktree.processes.length === 0) {
    wrapper.append(
      el("div", "changes-empty", "No processes yet. Procfile processes are listed here."),
    );
    return wrapper;
  }

  const list = el("div", "procfile-list");
  for (const [index, process] of worktree.processes.entries()) {
    list.append(renderProcfileProcessCard(process, index));
  }
  wrapper.append(list);
  return wrapper;
}

function renderProcfileProcessCard(proc: ProcessInfo, processIndex: number): HTMLElement {
  const card = el("div", "procfile-card");
  const sessionId = proc.session_id;
  if (sessionId !== null) {
    card.classList.add("is-openable");
    card.addEventListener("click", () => setActiveSession(sessionId));
  }

  const header = el("div", "procfile-card-header");
  header.append(el("span", `process-dot ${statusDotClass(proc.status)}`));

  const info = el("div", "procfile-card-info");
  const line1 = el("div", "procfile-line1");
  line1.append(
    el("span", "procfile-name", proc.name),
    el(
      "span",
      `procfile-status procfile-status-${proc.status}`,
      formatProcessStatus(proc.status),
    ),
    el("span", "procfile-source", proc.source === "procfile" ? "Procfile" : "arbor.toml"),
  );
  if (proc.restart_count > 0) {
    line1.append(el("span", "process-restarts", `x${proc.restart_count}`));
  }
  if (proc.memory_bytes !== null) {
    line1.append(el("span", "process-memory", formatProcessMemory(proc.memory_bytes)));
  }
  if (sessionId !== null) {
    line1.append(el("span", "procfile-session", "Openable"));
  }

  const line2 = el("div", "procfile-line2");
  line2.append(el("span", "procfile-command", proc.command));

  info.append(line1, line2);
  header.append(info);
  card.append(header);

  const actions = el("div", "procfile-actions");
  if (sessionId !== null) {
    actions.append(
      renderProcessActionButton(
        "Open",
        () => setActiveSession(sessionId),
        processIndex,
        "open",
      ),
    );
  }

  if (proc.status === "running" || proc.status === "restarting") {
    actions.append(
      renderProcessActionButton(
        "Restart",
        () => restartProcessAndRefresh(proc.id),
        processIndex,
        "restart",
      ),
    );
    actions.append(
      renderProcessActionButton(
        "Stop",
        () => stopProcessAndRefresh(proc.id),
        processIndex,
        "stop",
      ),
    );
  } else if (proc.status === "crashed") {
    actions.append(
      renderProcessActionButton(
        "Restart",
        () => restartProcessAndRefresh(proc.id),
        processIndex,
        "restart",
      ),
    );
  } else {
    actions.append(
      renderProcessActionButton(
        "Start",
        () => startProcessAndRefresh(proc.id),
        processIndex,
        "start",
      ),
    );
  }

  card.append(actions);
  return card;
}

function renderProcessActionButton(
  label: string,
  action: () => Promise<void> | void,
  processIndex: number,
  actionKind: string,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = "process-action-btn";
  button.type = "button";
  button.textContent = label;
  button.dataset["processAction"] = `${actionKind}-${processIndex}`;
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    Promise.resolve(action()).catch(() => {});
  });
  return button;
}

async function startProcessAndRefresh(id: string): Promise<void> {
  await startProcess(id);
  await refresh();
}

async function stopProcessAndRefresh(id: string): Promise<void> {
  await stopProcess(id);
  await refresh();
}

async function restartProcessAndRefresh(id: string): Promise<void> {
  await restartProcess(id);
  await refresh();
}

function formatIssueAge(updatedAt: string): string {
  const timestamp = Date.parse(updatedAt);
  if (Number.isNaN(timestamp)) {
    return updatedAt;
  }
  return formatAge(timestamp);
}

function formatProcessStatus(status: ProcessInfo["status"]): string {
  switch (status) {
    case "running":
      return "Running";
    case "restarting":
      return "Restarting";
    case "crashed":
      return "Crashed";
    default:
      return "Stopped";
  }
}

function formatProcessMemory(memoryBytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = memoryBytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  const digits = value >= 10 || unitIndex === 0 ? 0 : 1;
  return `RSS ${value.toFixed(digits)} ${units[unitIndex]}`;
}

function statusDotClass(status: ProcessInfo["status"]): string {
  switch (status) {
    case "running":
      return "dot-green";
    case "restarting":
      return "dot-yellow";
    case "crashed":
      return "dot-red";
    default:
      return "dot-gray";
  }
}
