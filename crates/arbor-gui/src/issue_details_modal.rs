impl ArborWindow {
    fn open_issue_details_modal_for_target(
        &mut self,
        target: IssueTarget,
        source_label: String,
        issue: terminal_daemon_http::IssueDto,
        cx: &mut Context<Self>,
    ) {
        self.create_modal = None;
        self.issue_details_modal = Some(IssueDetailsModal {
            target,
            source_label,
            issue,
        });
        cx.notify();
    }

    fn close_issue_details_modal(&mut self, cx: &mut Context<Self>) {
        self.issue_details_modal = None;
        cx.notify();
    }

    fn open_create_modal_from_issue_details(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.issue_details_modal.clone() else {
            return;
        };
        self.issue_details_modal = None;
        self.open_issue_create_modal_for_target(
            modal.target,
            modal.source_label,
            modal.issue,
            cx,
        );
    }

    fn render_issue_details_modal(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(modal) = self.issue_details_modal.clone() else {
            return div();
        };

        let theme = self.theme();
        let issue = modal.issue;
        let issue_url = issue.url.clone();
        let issue_number = issue.display_id.clone();
        let issue_body = issue.body.clone();
        let issue_state = issue.state.clone();
        let updated_at = issue.updated_at.clone();
        let linked_review = issue.linked_review.clone();
        let linked_branch = issue.linked_branch.clone();
        let title = issue.title.clone();
        let source_label = modal.source_label;
        let description_body = div()
            .id("issue-details-description-body")
            .flex()
            .flex_col()
            .gap(px(6.))
            .max_h(px(320.))
            .overflow_y_scroll()
            .pr_1()
            .children(issue_body_lines(
                issue_body
                    .as_deref()
                    .unwrap_or("No issue description is available for this issue."),
                theme.text_primary,
                theme.text_muted,
            ));

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.close_issue_details_modal(cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.close_issue_details_modal(cx);
                    cx.stop_propagation();
                }),
            )
            .child(modal_backdrop())
            .child(
                div()
                    .w(px(680.))
                    .max_w(px(680.))
                    .max_h(px(640.))
                    .flex_none()
                    .overflow_hidden()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.sidebar_bg))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_mouse_down(MouseButton::Right, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(theme.text_primary))
                                            .child("Issue"),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(theme.text_primary))
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .flex_wrap()
                                            .child(issue_meta_chip(
                                                issue_number.clone(),
                                                theme.accent,
                                                theme.panel_active_bg,
                                                issue_url.is_some(),
                                                issue_url.clone(),
                                                cx,
                                            ))
                                            .child(issue_meta_chip(
                                                issue_state,
                                                theme.text_primary,
                                                theme.panel_bg,
                                                false,
                                                None,
                                                cx,
                                            ))
                                            .child(issue_meta_chip(
                                                source_label,
                                                theme.text_muted,
                                                theme.panel_bg,
                                                false,
                                                None,
                                                cx,
                                            ))
                                            .when_some(updated_at.clone(), |this, updated_at| {
                                                this.child(issue_meta_chip(
                                                    issue_updated_label(&updated_at),
                                                    theme.text_muted,
                                                    theme.panel_bg,
                                                    false,
                                                    None,
                                                    cx,
                                                ))
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("issue-details-close")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme.text_muted))
                                    .hover(|this| this.text_color(rgb(theme.text_primary)))
                                    .child("Close")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_issue_details_modal(cx);
                                        cx.stop_propagation();
                                    })),
                            ),
                    )
                    .when(linked_review.is_some() || linked_branch.is_some(), |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .when_some(linked_review.clone(), |this, review| {
                                    let review_color = match review.kind {
                                        terminal_daemon_http::IssueReviewKind::PullRequest => {
                                            theme.accent
                                        },
                                        terminal_daemon_http::IssueReviewKind::MergeRequest => {
                                            0x72d69c
                                        },
                                    };
                                    this.child(issue_meta_chip(
                                        review.label,
                                        review_color,
                                        theme.panel_active_bg,
                                        review.url.is_some(),
                                        review.url,
                                        cx,
                                    ))
                                })
                                .when_some(linked_branch.clone(), |this, branch| {
                                    this.child(issue_meta_chip(
                                        branch,
                                        theme.text_primary,
                                        theme.panel_bg,
                                        false,
                                        None,
                                        cx,
                                    ))
                                }),
                        )
                    })
                    .child(
                        div()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(theme.border))
                            .bg(rgb(theme.panel_bg))
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme.text_muted))
                                    .child("Description"),
                            )
                            .child(
                                div()
                                    .min_h(px(120.))
                                    .child(description_body),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("issue-details-cancel")
                                    .cursor_pointer()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(rgb(theme.border))
                                    .px_2()
                                    .py_1()
                                    .text_xs()
                                    .text_color(rgb(theme.text_primary))
                                    .hover(|this| this.bg(rgb(theme.panel_active_bg)))
                                    .child("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_issue_details_modal(cx);
                                        cx.stop_propagation();
                                    })),
                            )
                            .when_some(issue_url, |this, issue_url| {
                                this.child(
                                    div()
                                        .id("issue-details-open-browser")
                                        .cursor_pointer()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(rgb(theme.border))
                                        .px_2()
                                        .py_1()
                                        .text_xs()
                                        .text_color(rgb(theme.text_primary))
                                        .hover(|this| this.bg(rgb(theme.panel_active_bg)))
                                        .child("Open in Browser")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open_external_url(&issue_url, cx);
                                            cx.stop_propagation();
                                        })),
                                )
                            })
                            .child(
                                div()
                                    .id("issue-details-create-worktree")
                                    .cursor_pointer()
                                    .rounded_sm()
                                    .bg(rgb(theme.accent))
                                    .px_2()
                                    .py_1()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme.sidebar_bg))
                                    .hover(|this| this.opacity(0.92))
                                    .child("Create Worktree")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_create_modal_from_issue_details(cx);
                                        cx.stop_propagation();
                                    })),
                            ),
                    ),
            )
    }
}

fn issue_meta_chip(
    label: String,
    text_color: u32,
    background: u32,
    is_interactive: bool,
    url: Option<String>,
    cx: &mut Context<ArborWindow>,
) -> Div {
    div()
        .rounded_full()
        .border_1()
        .border_color(rgb(text_color))
        .bg(rgb(background))
        .px(px(8.))
        .py(px(3.))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .font_family(FONT_MONO)
        .text_color(rgb(text_color))
        .when(is_interactive, |this| {
            this.cursor_pointer().hover(|this| this.opacity(0.9))
        })
        .when_some(url, |this, url| {
            this.on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                this.open_external_url(&url, cx);
                cx.stop_propagation();
            }))
        })
        .child(label)
}

fn issue_body_lines(body: &str, text_color: u32, muted_color: u32) -> Vec<Div> {
    body.split('\n')
        .map(|line| {
            if line.is_empty() {
                div().h(px(8.))
            } else {
                div()
                    .text_sm()
                    .text_color(rgb(if line.starts_with('#') {
                        muted_color
                    } else {
                        text_color
                    }))
                    .child(line.to_owned())
            }
        })
        .collect()
}

fn issue_updated_label(updated_at: &str) -> String {
    format!("updated {updated_at}")
}

fn issue_source_summary(source: &terminal_daemon_http::IssueSourceDto) -> String {
    source
        .url
        .as_deref()
        .map(|url| format!("{} · {} · {url}", source.provider, source.label))
        .unwrap_or_else(|| format!("{} · {} · {}", source.provider, source.label, source.repository))
}

fn issue_modal_source_label(source: &terminal_daemon_http::IssueSourceDto) -> String {
    source.provider.clone()
}
