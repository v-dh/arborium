impl ArborWindow {
    fn open_theme_picker_modal(&mut self, cx: &mut Context<Self>) {
        self.show_theme_picker = true;
        self.theme_picker_selected_index = theme_picker_index_for_kind(self.theme_kind);
        cx.notify();
    }

    fn move_theme_picker_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = ThemeKind::ALL.len();
        if len == 0 {
            return;
        }
        let current = self.theme_picker_selected_index.min(len - 1) as isize;
        self.theme_picker_selected_index = (current + delta).rem_euclid(len as isize) as usize;
        cx.notify();
    }

    fn apply_selected_theme_picker_theme(&mut self, cx: &mut Context<Self>) {
        let Some(&kind) = ThemeKind::ALL.get(self.theme_picker_selected_index) else {
            return;
        };
        self.switch_theme(kind, cx);
    }

    fn render_theme_picker_modal(&mut self, cx: &mut Context<Self>) -> Div {
        if !self.show_theme_picker {
            return div();
        }

        let theme = self.theme();
        let current_theme = self.theme_kind;
        let selected_index = self
            .theme_picker_selected_index
            .min(ThemeKind::ALL.len().saturating_sub(1));

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.show_theme_picker = false;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.show_theme_picker = false;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(modal_backdrop())
            .child(
                div()
                    .w(px(820.))
                    .max_h(px(600.))
                    .flex_none()
                    .overflow_hidden()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.sidebar_bg))
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme.text_primary))
                            .child("Choose Theme"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .children(ThemeKind::ALL.iter().enumerate().map(|(idx, &kind)| {
                                let palette = kind.palette();
                                let is_active = kind == current_theme;
                                let is_selected = idx == selected_index;
                                let border_color = if is_selected || is_active {
                                    theme.accent
                                } else {
                                    theme.border
                                };
                                div()
                                    .id(("theme-card", idx))
                                    .w(px(148.))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(border_color))
                                    .when(is_active || is_selected, |d| d.border_2())
                                    .bg(rgb(if is_selected {
                                        theme.panel_active_bg
                                    } else {
                                        theme.panel_bg
                                    }))
                                    .overflow_hidden()
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.85))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.theme_picker_selected_index = idx;
                                        this.switch_theme(kind, cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .h(px(36.))
                                            .child(div().flex_1().bg(rgb(palette.app_bg)))
                                            .child(div().flex_1().bg(rgb(palette.sidebar_bg)))
                                            .child(div().flex_1().bg(rgb(palette.accent)))
                                            .child(div().flex_1().bg(rgb(palette.text_primary)))
                                            .child(div().flex_1().bg(rgb(palette.border))),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(6.))
                                            .text_xs()
                                            .text_color(rgb(theme.text_primary))
                                            .when(is_active || is_selected, |d| {
                                                d.font_weight(FontWeight::SEMIBOLD)
                                            })
                                            .child(kind.label()),
                                    )
                            })),
                    ),
            )
    }
}

fn theme_picker_columns() -> usize {
    5
}

fn theme_picker_index_for_kind(theme_kind: ThemeKind) -> usize {
    ThemeKind::ALL
        .iter()
        .position(|candidate| *candidate == theme_kind)
        .unwrap_or(0)
}
