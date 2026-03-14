import { openCreateWorktreeModal } from "./create-worktree-modal";
import {
  refresh,
  refreshIssues,
  selectedIssueRepoRoot,
  setRightPanelTab,
  state,
  subscribe,
} from "../state";
import type { Issue } from "../types";
import { el } from "../utils";

type PaletteMode = "actions" | "issues";

type PaletteItem = {
  id: string;
  title: string;
  subtitle: string;
  keywords: string;
  run: () => void;
};

let overlay: HTMLDivElement | null = null;
let inputEl: HTMLInputElement | null = null;
let resultsEl: HTMLDivElement | null = null;
let open = false;
let mode: PaletteMode = "actions";
let query = "";
let selectedIndex = 0;

const ACTIONS: PaletteItem[] = [
  {
    id: "issues",
    title: "View Issues",
    subtitle: "Browse repository issues in the command palette",
    keywords: "issues github gitlab linear command palette tickets bugs",
    run: () => {
      openIssuesMode();
    },
  },
  {
    id: "changes",
    title: "Changes",
    subtitle: "View changed files",
    keywords: "changes files diff right panel",
    run: () => {
      closeCommandPalette();
      setRightPanelTab("changes");
    },
  },
  {
    id: "refresh",
    title: "Refresh",
    subtitle: "Reload repositories, worktrees, and terminals",
    keywords: "refresh reload sync fetch",
    run: () => {
      closeCommandPalette();
      void refresh();
    },
  },
];

export function createCommandPalette(): HTMLElement {
  overlay = el("div", "overlay-shell overlay-hidden");
  overlay.setAttribute("data-testid", "command-palette");
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) {
      closeCommandPalette();
    }
  });

  const dialog = el("div", "overlay-dialog palette-dialog");
  dialog.addEventListener("click", (event) => event.stopPropagation());

  inputEl = document.createElement("input");
  inputEl.className = "palette-input";
  inputEl.type = "text";
  inputEl.autocomplete = "off";
  inputEl.spellcheck = false;
  inputEl.addEventListener("input", () => {
    query = inputEl?.value ?? "";
    selectedIndex = 0;
    render();
  });
  inputEl.addEventListener("keydown", handlePaletteKeydown);

  resultsEl = el("div", "palette-results");
  dialog.append(inputEl, resultsEl);
  overlay.append(dialog);

  document.addEventListener("keydown", (event) => {
    const isPaletteShortcut =
      (event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === "k";

    if (isPaletteShortcut) {
      event.preventDefault();
      if (open) {
        closeCommandPalette();
      } else {
        openCommandPalette();
      }
      return;
    }

    if (!open || event.target === inputEl) {
      return;
    }

    handlePaletteKeydown(event);
  });

  subscribe(render);
  render();
  return overlay;
}

function openCommandPalette(): void {
  open = true;
  mode = "actions";
  query = "";
  selectedIndex = 0;
  if (inputEl !== null) {
    inputEl.value = "";
  }
  render();
  requestAnimationFrame(() => inputEl?.focus());
}

function closeCommandPalette(): void {
  open = false;
  mode = "actions";
  query = "";
  selectedIndex = 0;
  if (inputEl !== null) {
    inputEl.value = "";
  }
  render();
}

function openIssuesMode(): void {
  mode = "issues";
  query = "";
  selectedIndex = 0;
  if (inputEl !== null) {
    inputEl.value = "";
  }

  const repoRoot = selectedIssueRepoRoot();
  if (repoRoot !== null) {
    refreshIssues(
      repoRoot,
      state.issuesRepoRoot !== repoRoot || state.issuesLoadedRepoRoot !== repoRoot,
    );
  }

  render();
  requestAnimationFrame(() => inputEl?.focus());
}

function buildIssuePaletteItem(issue: Issue): PaletteItem {
  const subtitleParts = [issue.state];
  if (issue.linked_review !== null) {
    subtitleParts.push(issue.linked_review.label);
  }
  if (issue.linked_branch !== null) {
    subtitleParts.push(issue.linked_branch);
  }
  if (subtitleParts.length === 1) {
    subtitleParts.push("Create worktree");
  }

  return {
    id: `issue-${issue.id}`,
    title: `${issue.display_id} ${issue.title}`,
    subtitle: subtitleParts.join(" · "),
    keywords: [
      "issue",
      "issues",
      issue.id,
      issue.display_id,
      issue.title,
      issue.state,
      issue.suggested_worktree_name,
      issue.linked_branch ?? "",
      issue.linked_review?.label ?? "",
    ]
      .join(" ")
      .toLowerCase(),
    run: () => {
      closeCommandPalette();
      openCreateWorktreeModal(issue);
    },
  };
}

function filteredItems(): PaletteItem[] {
  const trimmed = query.trim().toLowerCase();
  const items = mode === "actions" ? ACTIONS : state.issues.map(buildIssuePaletteItem);
  if (trimmed.length === 0) {
    return items;
  }

  return items.filter((item) => {
    const haystack = `${item.title} ${item.subtitle} ${item.keywords}`.toLowerCase();
    return haystack.includes(trimmed);
  });
}

function emptyMessage(): string {
  if (mode === "actions") {
    return "No matching actions";
  }

  const repoRoot = selectedIssueRepoRoot();
  if (repoRoot === null) {
    return "Select a repository to browse issues";
  }
  if (state.issuesLoading) {
    return "Loading issues…";
  }
  if (state.issuesError !== null) {
    return state.issuesError;
  }
  if (state.issuesNotice !== null) {
    return state.issuesNotice;
  }
  if (query.trim().length > 0) {
    return "No matching issues";
  }
  return "No open issues";
}

function render(): void {
  if (overlay === null || resultsEl === null || inputEl === null) {
    return;
  }

  overlay.classList.toggle("overlay-hidden", !open);
  overlay.classList.toggle("overlay-visible", open);
  resultsEl.replaceChildren();

  if (!open) {
    return;
  }

  inputEl.placeholder = mode === "actions" ? "Search actions…" : "Search issues…";

  const items = filteredItems();
  if (items.length === 0) {
    resultsEl.append(el("div", "palette-empty", emptyMessage()));
    return;
  }

  items.forEach((itemData, index) => {
    const item = el("button", "palette-item");
    item.type = "button";
    item.setAttribute("data-palette-item-id", itemData.id);
    if (index === selectedIndex) {
      item.classList.add("active");
    }
    item.append(
      el("div", "palette-item-title", itemData.title),
      el("div", "palette-item-subtitle", itemData.subtitle),
    );
    item.addEventListener("mousemove", () => {
      if (selectedIndex !== index) {
        selectedIndex = index;
        render();
      }
    });
    item.addEventListener("click", () => {
      itemData.run();
    });
    resultsEl.append(item);
  });
}

function handlePaletteKeydown(event: KeyboardEvent): void {
  if (!open) {
    return;
  }

  if (event.key === "Escape") {
    event.preventDefault();
    if (mode === "issues") {
      mode = "actions";
      query = "";
      selectedIndex = 0;
      if (inputEl !== null) {
        inputEl.value = "";
      }
      render();
    } else {
      closeCommandPalette();
    }
    return;
  }

  const items = filteredItems();
  if (event.key === "ArrowDown") {
    event.preventDefault();
    if (items.length > 0) {
      selectedIndex = (selectedIndex + 1) % items.length;
      render();
    }
    return;
  }

  if (event.key === "ArrowUp") {
    event.preventDefault();
    if (items.length > 0) {
      selectedIndex = (selectedIndex + items.length - 1) % items.length;
      render();
    }
    return;
  }

  if (event.key === "Enter") {
    event.preventDefault();
    const selected = items[selectedIndex];
    if (selected !== undefined) {
      selected.run();
    }
  }
}
