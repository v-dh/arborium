use super::*;

pub(crate) fn diff_tab_title(session: &DiffSession) -> String {
    truncate_with_ellipsis(&session.title, TERMINAL_TAB_COMMAND_MAX_CHARS)
}

pub(crate) fn build_worktree_diff_document(
    worktree_path: &Path,
    changed_files: &[ChangedFile],
) -> Result<(Vec<DiffLine>, HashMap<PathBuf, usize>), GitError> {
    let mut lines = Vec::new();
    let mut file_row_indices = HashMap::new();

    for changed_file in changed_files {
        file_row_indices.insert(changed_file.path.clone(), lines.len());
        lines.push(DiffLine {
            left_line_number: None,
            right_line_number: None,
            left_text: format!(
                "{} {}",
                change_code(changed_file.kind),
                changed_file.path.display()
            ),
            right_text: String::new(),
            kind: DiffLineKind::FileHeader,
        });

        let file_lines = build_file_diff_lines(
            worktree_path,
            changed_file.path.as_path(),
            changed_file.kind,
        )?;
        if file_lines.is_empty() {
            lines.push(DiffLine {
                left_line_number: None,
                right_line_number: None,
                left_text: "  no textual changes".to_owned(),
                right_text: String::new(),
                kind: DiffLineKind::Context,
            });
        } else {
            lines.extend(file_lines);
        }
    }

    Ok((lines, file_row_indices))
}

fn build_file_diff_lines(
    worktree_path: &Path,
    file_path: &Path,
    change_kind: ChangeKind,
) -> Result<Vec<DiffLine>, GitError> {
    let head_bytes = match change_kind {
        ChangeKind::Added | ChangeKind::IntentToAdd => Vec::new(),
        _ => read_head_file_bytes(worktree_path, file_path)?,
    };
    let worktree_bytes = match change_kind {
        ChangeKind::Removed => Vec::new(),
        _ => read_worktree_file_bytes(worktree_path, file_path)?,
    };
    let head_text = String::from_utf8_lossy(&head_bytes).into_owned();
    let worktree_text = String::from_utf8_lossy(&worktree_bytes).into_owned();
    Ok(build_side_by_side_diff_lines(&head_text, &worktree_text))
}

fn read_head_file_bytes(worktree_path: &Path, file_path: &Path) -> Result<Vec<u8>, GitError> {
    let relative = git_relative_path(file_path)?;
    let object_spec = format!("HEAD:{relative}");

    let repo = gix::open(worktree_path).map_err(|error| {
        GitError::Operation(format!(
            "failed to open repository at `{}`: {error}",
            worktree_path.display()
        ))
    })?;

    let object_id = match repo.rev_parse_single(object_spec.as_str()) {
        Ok(id) => id,
        Err(_) => return Ok(Vec::new()), // file does not exist at HEAD
    };

    let object = object_id.object().map_err(|error| {
        GitError::Operation(format!(
            "failed to read `{relative}` at HEAD in `{}`: {error}",
            worktree_path.display()
        ))
    })?;

    Ok(object.data.to_vec())
}

fn read_worktree_file_bytes(worktree_path: &Path, file_path: &Path) -> Result<Vec<u8>, GitError> {
    let absolute = worktree_path.join(file_path);
    match fs::read(&absolute) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(GitError::Operation(format!(
            "failed to read worktree file `{}`: {error}",
            absolute.display()
        ))),
    }
}

fn git_relative_path(file_path: &Path) -> Result<String, GitError> {
    let path_text = file_path.to_string_lossy();
    if path_text.trim().is_empty() {
        return Err(GitError::Operation("cannot diff an empty path".to_owned()));
    }

    Ok(path_text.replace('\\', "/"))
}

pub(crate) fn build_side_by_side_diff_lines(before_text: &str, after_text: &str) -> Vec<DiffLine> {
    let before_rope = Rope::from_str(before_text);
    let after_rope = Rope::from_str(after_text);
    let input = BlobInternedInput::new(before_text.as_bytes(), after_text.as_bytes());
    let mut diff = BlobDiff::compute(DiffAlgorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let hunks = diff.hunks().collect::<Vec<_>>();

    if hunks.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut before_cursor = 0_usize;
    let mut after_cursor = 0_usize;
    let hunk_count = hunks.len();

    for (hunk_index, hunk) in hunks.iter().enumerate() {
        let before_start = hunk.before.start as usize;
        let before_end = hunk.before.end as usize;
        let after_start = hunk.after.start as usize;
        let after_end = hunk.after.end as usize;

        let (leading_context, trailing_context) = if hunk_index == 0 {
            (0, DIFF_HUNK_CONTEXT_LINES)
        } else {
            (DIFF_HUNK_CONTEXT_LINES, DIFF_HUNK_CONTEXT_LINES)
        };
        push_hunk_context_lines(
            &mut lines,
            &before_rope,
            &after_rope,
            before_cursor,
            before_start,
            after_cursor,
            after_start,
            leading_context,
            trailing_context,
        );

        let removed_count = before_end.saturating_sub(before_start);
        let added_count = after_end.saturating_sub(after_start);
        let changed_count = removed_count.max(added_count);

        for offset in 0..changed_count {
            let left_index = (offset < removed_count).then_some(before_start + offset);
            let right_index = (offset < added_count).then_some(after_start + offset);
            let kind = match (left_index.is_some(), right_index.is_some()) {
                (true, true) => DiffLineKind::Modified,
                (true, false) => DiffLineKind::Removed,
                (false, true) => DiffLineKind::Added,
                (false, false) => DiffLineKind::Context,
            };
            push_diff_line(
                &mut lines,
                &before_rope,
                &after_rope,
                left_index,
                right_index,
                kind,
            );
        }

        before_cursor = before_end;
        after_cursor = after_end;

        if hunk_index + 1 == hunk_count {
            push_hunk_context_lines(
                &mut lines,
                &before_rope,
                &after_rope,
                before_cursor,
                input.before.len(),
                after_cursor,
                input.after.len(),
                DIFF_HUNK_CONTEXT_LINES,
                0,
            );
        }
    }

    lines
}

fn push_hunk_context_lines(
    output: &mut Vec<DiffLine>,
    before_rope: &Rope,
    after_rope: &Rope,
    before_start: usize,
    before_end: usize,
    after_start: usize,
    after_end: usize,
    leading_context: usize,
    trailing_context: usize,
) {
    let before_count = before_end.saturating_sub(before_start);
    let after_count = after_end.saturating_sub(after_start);
    if before_count == 0 && after_count == 0 {
        return;
    }

    let leading_before_count = leading_context.min(before_count);
    let leading_after_count = leading_context.min(after_count);
    let leading_before_end = before_start.saturating_add(leading_before_count);
    let leading_after_end = after_start.saturating_add(leading_after_count);

    let trailing_before_available = before_end.saturating_sub(leading_before_end);
    let trailing_after_available = after_end.saturating_sub(leading_after_end);
    let trailing_before_count = trailing_context.min(trailing_before_available);
    let trailing_after_count = trailing_context.min(trailing_after_available);
    let trailing_before_start = before_end.saturating_sub(trailing_before_count);
    let trailing_after_start = after_end.saturating_sub(trailing_after_count);

    if leading_before_end > before_start || leading_after_end > after_start {
        push_context_diff_lines(
            output,
            before_rope,
            after_rope,
            before_start,
            leading_before_end,
            after_start,
            leading_after_end,
        );
    }

    let hidden_before_count = trailing_before_start.saturating_sub(leading_before_end);
    let hidden_after_count = trailing_after_start.saturating_sub(leading_after_end);
    if hidden_before_count > 0 || hidden_after_count > 0 {
        push_collapsed_gap_line(output, hidden_before_count, hidden_after_count);
    }

    if trailing_before_start < before_end || trailing_after_start < after_end {
        push_context_diff_lines(
            output,
            before_rope,
            after_rope,
            trailing_before_start,
            before_end,
            trailing_after_start,
            after_end,
        );
    }
}

fn push_collapsed_gap_line(
    output: &mut Vec<DiffLine>,
    hidden_before_count: usize,
    hidden_after_count: usize,
) {
    output.push(DiffLine {
        left_line_number: None,
        right_line_number: None,
        left_text: format!("… {hidden_before_count} unchanged lines hidden"),
        right_text: format!("… {hidden_after_count} unchanged lines hidden"),
        kind: DiffLineKind::Context,
    });
}

fn push_context_diff_lines(
    output: &mut Vec<DiffLine>,
    before_rope: &Rope,
    after_rope: &Rope,
    before_start: usize,
    before_end: usize,
    after_start: usize,
    after_end: usize,
) {
    let before_count = before_end.saturating_sub(before_start);
    let after_count = after_end.saturating_sub(after_start);
    let paired_count = before_count.min(after_count);

    for offset in 0..paired_count {
        push_diff_line(
            output,
            before_rope,
            after_rope,
            Some(before_start + offset),
            Some(after_start + offset),
            DiffLineKind::Context,
        );
    }

    for offset in paired_count..before_count {
        push_diff_line(
            output,
            before_rope,
            after_rope,
            Some(before_start + offset),
            None,
            DiffLineKind::Removed,
        );
    }

    for offset in paired_count..after_count {
        push_diff_line(
            output,
            before_rope,
            after_rope,
            None,
            Some(after_start + offset),
            DiffLineKind::Added,
        );
    }
}

fn push_diff_line(
    output: &mut Vec<DiffLine>,
    before_rope: &Rope,
    after_rope: &Rope,
    left_index: Option<usize>,
    right_index: Option<usize>,
    kind: DiffLineKind,
) {
    output.push(DiffLine {
        left_line_number: left_index.map(|index| index + 1),
        right_line_number: right_index.map(|index| index + 1),
        left_text: left_index
            .map(|index| rope_display_line(before_rope, index))
            .unwrap_or_default(),
        right_text: right_index
            .map(|index| rope_display_line(after_rope, index))
            .unwrap_or_default(),
        kind,
    });
}

fn rope_display_line(rope: &Rope, line_index: usize) -> String {
    if line_index >= rope.len_lines() {
        return String::new();
    }

    let mut text = rope.line(line_index).to_string();
    while text.ends_with('\n') || text.ends_with('\r') {
        let _ = text.pop();
    }
    text.replace('\t', "    ")
}

impl ArborWindow {
    pub(crate) fn scroll_diff_to_file(&self, file_path: &Path) -> bool {
        let Some(worktree_path) = self.selected_worktree_path() else {
            return false;
        };
        let Some(active_diff_id) = self.active_diff_session_id else {
            return false;
        };
        let Some(session) = self.diff_sessions.iter().find(|session| {
            session.id == active_diff_id && session.worktree_path.as_path() == worktree_path
        }) else {
            return false;
        };
        let Some(row_index) = session.file_row_indices.get(file_path) else {
            return false;
        };

        self.diff_scroll_handle
            .scroll_to_item_strict(*row_index, ScrollStrategy::Top);
        true
    }

    pub(crate) fn open_diff_tab_for_selected_file(&mut self, cx: &mut Context<Self>) {
        let Some(worktree_path) = self.selected_worktree_path().map(Path::to_path_buf) else {
            self.notice = Some("select a worktree before opening a diff".to_owned());
            return;
        };
        tracing::info!(worktree = %worktree_path.display(), "opening diff tab");
        let Some(selected_file_path) = self
            .selected_changed_file()
            .map(|change| change.path.clone())
            .or_else(|| self.changed_files.first().map(|change| change.path.clone()))
        else {
            self.notice = Some("select a changed file before opening a diff".to_owned());
            return;
        };

        let changed_files = self.changed_files.clone();
        let (session_id, should_rebuild) = match self
            .diff_sessions
            .iter_mut()
            .find(|session| session.worktree_path == worktree_path)
        {
            Some(existing) => {
                self.active_diff_session_id = Some(existing.id);
                (
                    existing.id,
                    !existing.is_loading
                        && (existing.lines.is_empty()
                            || !existing.file_row_indices.contains_key(&selected_file_path)),
                )
            },
            None => {
                let session_id = self.next_diff_session_id;
                self.next_diff_session_id = self.next_diff_session_id.saturating_add(1);
                self.diff_sessions.push(DiffSession {
                    id: session_id,
                    worktree_path: worktree_path.clone(),
                    title: "Diff".to_owned(),
                    raw_lines: Arc::<[DiffLine]>::from(Vec::<DiffLine>::new()),
                    raw_file_row_indices: HashMap::new(),
                    lines: Arc::<[DiffLine]>::from(Vec::<DiffLine>::new()),
                    file_row_indices: HashMap::new(),
                    wrapped_columns: 0,
                    is_loading: true,
                });
                self.active_diff_session_id = Some(session_id);
                (session_id, true)
            },
        };

        self.pending_diff_scroll_to_file = Some(selected_file_path.clone());
        if !should_rebuild {
            let _ = self.scroll_diff_to_file(selected_file_path.as_path());
            self.sync_navigation_ui_state_store(cx);
            cx.notify();
            return;
        }

        if let Some(session) = self
            .diff_sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.is_loading = true;
            session.raw_lines = Arc::<[DiffLine]>::from(Vec::<DiffLine>::new());
            session.raw_file_row_indices.clear();
            session.lines = Arc::<[DiffLine]>::from(Vec::<DiffLine>::new());
            session.file_row_indices.clear();
            session.wrapped_columns = 0;
        }
        self.sync_navigation_ui_state_store(cx);
        cx.notify();

        cx.spawn(async move |this, cx| {
            let diff_document = cx
                .background_spawn(async move {
                    build_worktree_diff_document(&worktree_path, &changed_files)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                let cell_width = diff_cell_width_px(cx);
                let wrap_columns = this
                    .live_diff_list_width_px()
                    .map(|list_width| {
                        this.estimated_diff_wrap_columns_for_list_width(list_width, cell_width)
                    })
                    .unwrap_or_else(|| this.estimated_diff_wrap_columns(cell_width));
                let Some(session) = this
                    .diff_sessions
                    .iter_mut()
                    .find(|session| session.id == session_id)
                else {
                    return;
                };

                match diff_document {
                    Ok((lines, file_row_indices)) => {
                        let raw_lines = Arc::<[DiffLine]>::from(lines);
                        let raw_file_row_indices = file_row_indices;
                        let (wrapped_lines, wrapped_indices) = wrap_diff_document_lines(
                            raw_lines.as_ref(),
                            &raw_file_row_indices,
                            wrap_columns,
                        );
                        session.raw_lines = raw_lines;
                        session.raw_file_row_indices = raw_file_row_indices;
                        session.lines = Arc::<[DiffLine]>::from(wrapped_lines);
                        session.file_row_indices = wrapped_indices;
                        session.wrapped_columns = wrap_columns;
                        session.is_loading = false;

                        if let Some(target_path) = this.pending_diff_scroll_to_file.clone()
                            && this.scroll_diff_to_file(target_path.as_path())
                        {
                            this.pending_diff_scroll_to_file = None;
                        }
                    },
                    Err(error) => {
                        let fallback_lines = Arc::<[DiffLine]>::from(vec![DiffLine {
                            left_line_number: None,
                            right_line_number: None,
                            left_text: format!("failed to build diff: {error}"),
                            right_text: String::new(),
                            kind: DiffLineKind::FileHeader,
                        }]);
                        let fallback_indices = HashMap::new();
                        let (wrapped_lines, wrapped_indices) = wrap_diff_document_lines(
                            fallback_lines.as_ref(),
                            &fallback_indices,
                            wrap_columns,
                        );
                        session.raw_lines = fallback_lines;
                        session.raw_file_row_indices = fallback_indices;
                        session.lines = Arc::<[DiffLine]>::from(wrapped_lines);
                        session.file_row_indices = wrapped_indices;
                        session.wrapped_columns = wrap_columns;
                        session.is_loading = false;
                        this.notice = Some(format!("failed to build diff: {error}"));
                    },
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn select_diff_tab(&mut self, session_id: u64, cx: &mut Context<Self>) {
        if self.active_diff_session_id == Some(session_id) && !self.logs_tab_active {
            return;
        }
        self.active_diff_session_id = Some(session_id);
        self.active_file_view_session_id = None;
        self.logs_tab_active = false;
        if let Some(selected_path) = self.selected_changed_file.clone()
            && !self.scroll_diff_to_file(selected_path.as_path())
        {
            self.pending_diff_scroll_to_file = Some(selected_path);
        }
        self.sync_navigation_ui_state_store(cx);
        cx.notify();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn side_by_side_diff_marks_modified_lines() {
        let lines = build_side_by_side_diff_lines("alpha\nbeta\n", "alpha\ngamma\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].kind, DiffLineKind::Context);
        assert_eq!(lines[0].left_text, "alpha");
        assert_eq!(lines[0].right_text, "alpha");
        assert_eq!(lines[1].kind, DiffLineKind::Modified);
        assert_eq!(lines[1].left_text, "beta");
        assert_eq!(lines[1].right_text, "gamma");
    }

    #[test]
    fn side_by_side_diff_marks_inserted_and_removed_lines() {
        let insertion = build_side_by_side_diff_lines("one\n", "one\ntwo\n");
        assert_eq!(insertion.len(), 2);
        assert_eq!(insertion[1].kind, DiffLineKind::Added);
        assert_eq!(insertion[1].left_text, "");
        assert_eq!(insertion[1].right_text, "two");

        let removal = build_side_by_side_diff_lines("one\ntwo\n", "one\n");
        assert_eq!(removal.len(), 2);
        assert_eq!(removal[1].kind, DiffLineKind::Removed);
        assert_eq!(removal[1].left_text, "two");
        assert_eq!(removal[1].right_text, "");
    }

    #[test]
    fn side_by_side_diff_hides_large_unchanged_gaps() {
        let before = "a1\na2\na3\na4\na5\na6\na7\na8\na9\na10\n";
        let after = "a1\na2\na3\na4\na5\nchanged\na7\na8\na9\na10\n";
        let lines = build_side_by_side_diff_lines(before, after);

        assert!(!lines.is_empty());
        assert!(
            lines
                .iter()
                .any(|line| line.left_text.contains("unchanged lines hidden"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.left_text == "a3" && line.right_text == "a3")
        );
    }
}
