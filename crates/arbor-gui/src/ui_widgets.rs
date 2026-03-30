use {
    super::*,
    arbor_core::agent::AgentState,
    gpui::{
        AnyElement, Bounds, Div, ElementId, FontWeight, Image, ImageFormat, Pixels, Stateful, div,
        img, point, px, rgb, size,
    },
    std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex, OnceLock},
    },
};

pub(crate) fn loading_status_text(theme: ThemePalette, text: impl Into<String>) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme.accent))
        .child(text.into())
}

pub(crate) fn loading_spinner_frame(frame: usize) -> &'static str {
    LOADING_SPINNER_FRAMES[frame % LOADING_SPINNER_FRAMES.len()]
}

pub(crate) fn action_button(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    style: ActionButtonStyle,
    enabled: bool,
) -> Stateful<Div> {
    let background = if enabled && style == ActionButtonStyle::Primary {
        theme.panel_active_bg
    } else {
        theme.panel_bg
    };
    let text_color = if enabled {
        theme.text_primary
    } else {
        theme.text_disabled
    };

    div()
        .id(id)
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgb(theme.panel_active_bg)))
        })
        .rounded_sm()
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(background))
        .px_2()
        .py_1()
        .text_xs()
        .text_color(rgb(text_color))
        .child(label.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionButtonStyle {
    Primary,
    Secondary,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum StatusMetricIconKind {
    Cpu,
    Memory,
}

/// Return the icon image for a preset kind, if one exists.
/// Returns `None` for agents without custom icon assets.
pub(crate) fn preset_icon_image(kind: AgentPresetKind) -> Arc<Image> {
    static ICONS: OnceLock<Mutex<HashMap<AgentPresetKind, Arc<Image>>>> = OnceLock::new();
    let map = ICONS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(kind)
        .or_insert_with(|| {
            let bytes = preset_icon_bytes(kind);
            let format = preset_icon_format(kind);
            tracing::info!(
                preset = kind.key(),
                bytes = bytes.len(),
                "loading preset icon asset"
            );
            Arc::new(Image::from_bytes(format, bytes.to_vec()))
        })
        .clone()
}

pub(crate) fn preset_icon_bytes(kind: AgentPresetKind) -> &'static [u8] {
    match kind {
        AgentPresetKind::Codex => PRESET_ICON_CODEX_SVG,
        AgentPresetKind::Claude => PRESET_ICON_CLAUDE_PNG,
        AgentPresetKind::Pi => PRESET_ICON_PI_SVG,
        AgentPresetKind::OpenCode => PRESET_ICON_OPENCODE_SVG,
        AgentPresetKind::Copilot => PRESET_ICON_COPILOT_SVG,
        // Agents without custom icons use a generic terminal icon
        _ => PRESET_ICON_CODEX_SVG,
    }
}

pub(crate) fn preset_icon_format(kind: AgentPresetKind) -> ImageFormat {
    match kind {
        AgentPresetKind::Claude => ImageFormat::Png,
        _ => ImageFormat::Svg,
    }
}

pub(crate) fn preset_icon_asset_path(kind: AgentPresetKind) -> &'static str {
    match kind {
        AgentPresetKind::Codex => "assets/preset-icons/codex-white.svg",
        AgentPresetKind::Claude => "assets/preset-icons/claude.png",
        AgentPresetKind::Pi => "assets/preset-icons/pi-white.svg",
        AgentPresetKind::OpenCode => "assets/preset-icons/opencode-white.svg",
        AgentPresetKind::Copilot => "assets/preset-icons/copilot-white.svg",
        _ => "assets/preset-icons/codex-white.svg",
    }
}

pub(crate) fn log_preset_icon_fallback_once(kind: AgentPresetKind, fallback_glyph: &'static str) {
    static LOGGED: OnceLock<Mutex<HashSet<AgentPresetKind>>> = OnceLock::new();
    let set = LOGGED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut set = set.lock().unwrap_or_else(|e| e.into_inner());
    if set.insert(kind) {
        tracing::warn!(
            preset = kind.key(),
            asset = preset_icon_asset_path(kind),
            bytes = preset_icon_bytes(kind).len(),
            fallback = fallback_glyph,
            "preset icon asset could not be rendered, using fallback glyph"
        );
        eprintln!(
            "WARN preset icon fallback preset={} asset={} bytes={} fallback={}",
            kind.key(),
            preset_icon_asset_path(kind),
            preset_icon_bytes(kind).len(),
            fallback_glyph
        );
    }
}

pub(crate) fn log_preset_icon_render_once(kind: AgentPresetKind) {
    static LOGGED: OnceLock<Mutex<HashSet<AgentPresetKind>>> = OnceLock::new();
    let set = LOGGED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut set = set.lock().unwrap_or_else(|e| e.into_inner());
    if set.insert(kind) {
        tracing::info!(
            preset = kind.key(),
            asset = preset_icon_asset_path(kind),
            "preset icon render path active"
        );
    }
}

pub(crate) fn preset_icon_render_size_px(kind: AgentPresetKind) -> f32 {
    match kind {
        AgentPresetKind::Codex => 20.,
        _ => 14.,
    }
}

pub(crate) fn agent_preset_button_content(kind: AgentPresetKind, text_color: u32) -> Div {
    log_preset_icon_render_once(kind);
    let icon = preset_icon_image(kind);
    let icon_size = preset_icon_render_size_px(kind);
    // Use consistent slot size for all icons to ensure vertical alignment
    let icon_slot_size = 20_f32;
    let fallback_color = match kind {
        AgentPresetKind::Claude => 0xD97757,
        _ => text_color,
    };
    let fallback_glyph = kind.fallback_icon();
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .w(px(icon_slot_size))
                .h(px(icon_slot_size))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(img(icon).size(px(icon_size)).with_fallback(move || {
                    log_preset_icon_fallback_once(kind, fallback_glyph);
                    div()
                        .font_family(FONT_MONO)
                        .text_size(px(12.))
                        .line_height(px(12.))
                        .text_color(rgb(fallback_color))
                        .child(fallback_glyph)
                        .into_any_element()
                })),
        )
        .child(
            div()
                .text_size(px(12.))
                .line_height(px(14.))
                .text_color(rgb(text_color))
                .child(kind.label()),
        )
}

/// Render an agent chat icon element for use in tab bars and menus.
///
/// Uses the agent's SVG/PNG icon from presets, with a Nerd Font chat bubble fallback.
pub(crate) fn agent_chat_tab_icon_element(
    kind: AgentPresetKind,
    text_color: u32,
    icon_size_px: f32,
) -> Div {
    let icon = preset_icon_image(kind);
    let fallback_color = match kind {
        AgentPresetKind::Claude => 0xD97757,
        _ => text_color,
    };
    let fallback_glyph = match kind {
        AgentPresetKind::Claude => "C",
        _ => kind.fallback_icon(),
    };
    div()
        .w(px(icon_size_px))
        .h(px(icon_size_px))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(img(icon).size(px(icon_size_px)).with_fallback(move || {
            log_preset_icon_fallback_once(kind, fallback_glyph);
            div()
                .font_family(FONT_MONO)
                .text_size(px(icon_size_px * 0.7))
                .line_height(px(icon_size_px))
                .text_color(rgb(fallback_color))
                .child(fallback_glyph)
                .into_any_element()
        }))
}

pub(crate) fn git_action_button(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    icon: &'static str,
    label: &'static str,
    enabled: bool,
    active: bool,
) -> Stateful<Div> {
    let background = if active {
        theme.panel_active_bg
    } else {
        theme.panel_bg
    };
    let icon_color = if active {
        theme.accent
    } else if enabled {
        theme.text_muted
    } else {
        theme.text_disabled
    };
    let text_color = if enabled || active {
        theme.text_primary
    } else {
        theme.text_disabled
    };

    div()
        .id(id)
        .h(px(24.))
        .rounded_sm()
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(background))
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(rgb(theme.panel_active_bg)))
        })
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(px(13.))
                .text_color(rgb(icon_color))
                .child(icon),
        )
        .child(div().text_xs().text_color(rgb(text_color)).child(label))
}

pub(crate) fn modal_backdrop() -> Div {
    div().absolute().inset_0().bg(rgb(0x000000)).opacity(0.28)
}

pub(crate) fn modal_input_field(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    value: &str,
    cursor: usize,
    placeholder: impl Into<String>,
    active: bool,
) -> Stateful<Div> {
    let label = label.into();
    let placeholder = placeholder.into();

    div()
        .id(id)
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme.text_muted))
                .child(label),
        )
        .child(
            div()
                .overflow_hidden()
                .cursor_pointer()
                .rounded_sm()
                .border_1()
                .border_color(rgb(if active {
                    theme.accent
                } else {
                    theme.border
                }))
                .bg(rgb(theme.panel_bg))
                .px_2()
                .py_1()
                .text_sm()
                .font_family(FONT_MONO)
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(if active {
                    if value.is_empty() {
                        active_input_display(theme, "", &placeholder, theme.text_disabled, 0, 48)
                    } else {
                        active_input_display(
                            theme,
                            value,
                            &placeholder,
                            theme.text_primary,
                            cursor,
                            56,
                        )
                    }
                } else if value.is_empty() {
                    div()
                        .text_color(rgb(theme.text_disabled))
                        .child(placeholder)
                        .into_any_element()
                } else {
                    div()
                        .text_color(rgb(theme.text_primary))
                        .child(value.to_owned())
                        .into_any_element()
                }),
        )
}

pub(crate) fn single_line_input_field(
    theme: ThemePalette,
    id: impl Into<ElementId>,
    value: &str,
    cursor: usize,
    placeholder: impl Into<String>,
    active: bool,
) -> Stateful<Div> {
    let placeholder = placeholder.into();

    div()
        .id(id)
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .h(px(30.))
        .cursor_text()
        .rounded_sm()
        .border_1()
        .border_color(rgb(if active {
            theme.accent
        } else {
            theme.border
        }))
        .bg(rgb(theme.panel_bg))
        .px_2()
        .text_sm()
        .font_family(FONT_MONO)
        .flex()
        .items_center()
        .child(if active {
            if value.is_empty() {
                active_input_display(theme, "", &placeholder, theme.text_disabled, 0, 48)
            } else {
                active_input_display(theme, value, &placeholder, theme.text_primary, cursor, 48)
            }
        } else {
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(rgb(if value.is_empty() {
                    theme.text_disabled
                } else {
                    theme.text_primary
                }))
                .child(if value.is_empty() {
                    placeholder
                } else {
                    value.to_owned()
                })
                .into_any_element()
        })
}

pub(crate) fn active_input_display(
    theme: ThemePalette,
    value: &str,
    placeholder: &str,
    text_color: u32,
    cursor: usize,
    max_chars: usize,
) -> AnyElement {
    if value.is_empty() {
        return div()
            .relative()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(
                div()
                    .text_color(rgb(text_color))
                    .child(placeholder.to_owned()),
            )
            .child(
                input_caret(theme)
                    .flex_none()
                    .absolute()
                    .left(px(0.))
                    .top(px(2.)),
            )
            .into_any_element();
    }

    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .flex()
        .items_center()
        .justify_start()
        .gap(px(0.))
        .child({
            let (before_cursor, after_cursor) = visible_input_segments(value, cursor, max_chars);
            div()
                .flex()
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(text_color))
                        .child(before_cursor),
                )
                .child(input_caret(theme).flex_none())
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(text_color))
                        .child(after_cursor),
                )
        })
        .into_any_element()
}

pub(crate) fn visible_input_segments(
    value: &str,
    cursor: usize,
    max_chars: usize,
) -> (String, String) {
    let chars: Vec<char> = value.chars().collect();
    let len = chars.len();
    let cursor = cursor.min(len);
    if len <= max_chars {
        let before: String = chars[..cursor].iter().collect();
        let after: String = chars[cursor..].iter().collect();
        return (before, after);
    }

    let window = max_chars.max(1);
    let preferred_left = window.saturating_sub(8);
    let mut start = cursor.saturating_sub(preferred_left);
    start = start.min(len.saturating_sub(window));
    let end = (start + window).min(len);

    let mut before: String = chars[start..cursor].iter().collect();
    let mut after: String = chars[cursor..end].iter().collect();
    if start > 0 {
        before.insert(0, '\u{2026}');
    }
    if end < len {
        after.push('\u{2026}');
    }
    (before, after)
}

pub(crate) fn multiline_input_display(
    theme: ThemePalette,
    value: &str,
    placeholder: &str,
    text_color: u32,
    cursor: usize,
) -> AnyElement {
    if value.is_empty() {
        return div()
            .relative()
            .child(
                div()
                    .text_color(rgb(text_color))
                    .opacity(0.5)
                    .child(placeholder.to_owned()),
            )
            .child(input_caret(theme).absolute().left(px(0.)).top(px(2.)))
            .into_any_element();
    }

    let chars: Vec<char> = value.chars().collect();
    let cursor = cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let after: String = chars[cursor..].iter().collect();

    // Split into lines, rendering the caret at the cursor position.
    // We build a column of lines; the line containing the cursor gets the
    // caret inserted inline between `before` and `after`.
    let before_lines: Vec<&str> = before.split('\n').collect();
    let after_lines: Vec<&str> = after.split('\n').collect();

    let mut container = div().flex().flex_col();

    // Lines entirely before the cursor line
    for line in &before_lines[..before_lines.len() - 1] {
        container = container.child(div().text_color(rgb(text_color)).child(if line.is_empty() {
            "\u{00A0}".to_owned() // non-breaking space to preserve empty lines
        } else {
            line.to_string()
        }));
    }

    // The cursor line: last segment of `before` + caret + first segment of `after`
    let cursor_line_before = before_lines.last().copied().unwrap_or("");
    let cursor_line_after = after_lines.first().copied().unwrap_or("");
    container = container.child(
        div()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(text_color))
                    .child(cursor_line_before.to_owned()),
            )
            .child(input_caret(theme).flex_none())
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(text_color))
                    .child(cursor_line_after.to_owned()),
            ),
    );

    // Lines entirely after the cursor line
    for line in &after_lines[1..] {
        container = container.child(div().text_color(rgb(text_color)).child(if line.is_empty() {
            "\u{00A0}".to_owned()
        } else {
            line.to_string()
        }));
    }

    container.into_any_element()
}

pub(crate) fn input_caret(theme: ThemePalette) -> Div {
    div().w(px(1.)).h(px(14.)).bg(rgb(theme.accent)).mt(px(1.))
}

pub(crate) fn status_text(theme: ThemePalette, text: impl Into<String>) -> Div {
    div()
        .text_xs()
        .text_color(rgb(theme.text_muted))
        .child(text.into())
}

pub(crate) fn status_metric_icon(theme: ThemePalette, kind: StatusMetricIconKind) -> Div {
    let color = theme.text_muted;
    let label = kind.fallback_label();
    let icon = status_metric_icon_image(kind, color);

    div()
        .size(px(12.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            img(icon)
                .size(px(12.))
                .with_fallback(move || status_text(theme, label).into_any_element()),
        )
}

fn status_metric_icon_image(kind: StatusMetricIconKind, color: u32) -> Arc<Image> {
    static ICONS: OnceLock<Mutex<HashMap<(StatusMetricIconKind, u32), Arc<Image>>>> =
        OnceLock::new();
    let map = ICONS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map.lock().unwrap_or_else(|error| error.into_inner());

    map.entry((kind, color))
        .or_insert_with(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                status_metric_icon_svg(kind, color).into_bytes(),
            ))
        })
        .clone()
}

fn status_metric_icon_svg(kind: StatusMetricIconKind, color: u32) -> String {
    let stroke = format!("#{:06X}", color);

    match kind {
        StatusMetricIconKind::Cpu => format!(
            r##"<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg" fill="none" stroke="{stroke}" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round">
  <rect x="4.25" y="4.25" width="7.5" height="7.5" rx="1.4"/>
  <path d="M6.75 6.75h2.5v2.5h-2.5z"/>
  <path d="M6.5 1.75v1.5"/>
  <path d="M9.5 1.75v1.5"/>
  <path d="M6.5 12.75v1.5"/>
  <path d="M9.5 12.75v1.5"/>
  <path d="M1.75 6.5h1.5"/>
  <path d="M1.75 9.5h1.5"/>
  <path d="M12.75 6.5h1.5"/>
  <path d="M12.75 9.5h1.5"/>
</svg>"##
        ),
        StatusMetricIconKind::Memory => format!(
            r##"<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg" fill="none" stroke="{stroke}" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round">
  <rect x="2.25" y="4.25" width="11.5" height="7.5" rx="1.5"/>
  <path d="M4.75 6.5h6.5"/>
  <path d="M4.75 8h6.5"/>
  <path d="M4.75 9.5h3.5"/>
  <path d="M4.5 2.5v1.5"/>
  <path d="M7 2.5v1.5"/>
  <path d="M9.5 2.5v1.5"/>
  <path d="M12 2.5v1.5"/>
  <path d="M4.5 12v1.5"/>
  <path d="M7 12v1.5"/>
  <path d="M9.5 12v1.5"/>
  <path d="M12 12v1.5"/>
</svg>"##
        ),
    }
}

impl StatusMetricIconKind {
    fn fallback_label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "MEM",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WorktreeAttentionIndicator {
    pub(crate) label: &'static str,
    pub(crate) short_label: &'static str,
    pub(crate) color: u32,
}

pub(crate) fn worktree_attention_indicator(
    worktree: &WorktreeSummary,
) -> WorktreeAttentionIndicator {
    if worktree.stuck_turn_count >= 2 {
        return WorktreeAttentionIndicator {
            label: "Stuck",
            short_label: "Stuck",
            color: 0xeb6f92,
        };
    }
    if worktree.agent_state == Some(AgentState::Working) {
        return WorktreeAttentionIndicator {
            label: "Working",
            short_label: "Run",
            color: 0xe5c07b,
        };
    }
    if worktree.agent_state == Some(AgentState::Waiting)
        && worktree
            .recent_turns
            .first()
            .and_then(|snapshot| snapshot.diff_summary)
            .is_some_and(|summary| summary.additions > 0 || summary.deletions > 0)
    {
        return WorktreeAttentionIndicator {
            label: "Needs review",
            short_label: "Review",
            color: 0x61afef,
        };
    }
    if worktree.agent_state == Some(AgentState::Waiting) {
        return WorktreeAttentionIndicator {
            label: "Waiting",
            short_label: "Wait",
            color: 0x61afef,
        };
    }
    if !worktree.detected_ports.is_empty() {
        return WorktreeAttentionIndicator {
            label: "Serving",
            short_label: "Ports",
            color: 0x72d69c,
        };
    }
    if worktree.last_activity_unix_ms.is_some_and(|timestamp| {
        current_unix_timestamp_millis()
            .unwrap_or(0)
            .saturating_sub(timestamp)
            <= 15 * 60 * 1000
    }) {
        return WorktreeAttentionIndicator {
            label: "Recent",
            short_label: "Recent",
            color: 0xc0caf5,
        };
    }

    WorktreeAttentionIndicator {
        label: "Idle",
        short_label: "Idle",
        color: 0x7f8490,
    }
}

pub(crate) fn worktree_activity_sparkline(worktree: &WorktreeSummary) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if worktree.recent_turns.is_empty() {
        return String::new();
    }

    let values: Vec<usize> = worktree
        .recent_turns
        .iter()
        .take(5)
        .rev()
        .map(|snapshot| {
            snapshot
                .diff_summary
                .map(|summary| summary.additions + summary.deletions)
                .unwrap_or(0)
        })
        .collect();
    let max_value = values.iter().copied().max().unwrap_or(0);
    if max_value == 0 {
        return "▁▁▁".to_owned();
    }

    values
        .into_iter()
        .map(|value| {
            let index = value.saturating_mul(BARS.len() - 1) / max_value.max(1);
            BARS[index]
        })
        .collect()
}

pub(crate) fn estimated_worktree_hover_popover_card_height(
    worktree: &WorktreeSummary,
    checks_expanded: bool,
) -> Pixels {
    let mut height = 72.;

    if worktree
        .diff_summary
        .is_some_and(|summary| summary.additions > 0 || summary.deletions > 0)
    {
        height += 18.;
    }

    height += 18.;

    if !worktree.recent_turns.is_empty() {
        height += 24. + worktree.recent_turns.iter().take(3).count() as f32 * 18.;
    }

    if !worktree.detected_ports.is_empty() {
        height += 22.;
    }

    if !worktree.recent_agent_sessions.is_empty() {
        let visible_sessions = worktree.recent_agent_sessions.iter().take(4);
        let provider_headers = visible_sessions
            .clone()
            .fold((None, 0usize), |(previous, count), session| {
                if previous == Some(session.provider) {
                    (previous, count)
                } else {
                    (Some(session.provider), count + 1)
                }
            })
            .1;
        height += 24.
            + worktree.recent_agent_sessions.iter().take(4).count() as f32 * 18.
            + provider_headers as f32 * 16.;
    }

    if let Some(pr) = worktree.pr_details.as_ref() {
        height += 110.;
        if checks_expanded
            && !pr.checks.is_empty()
            && matches!(
                pr.state,
                github_service::PrState::Open | github_service::PrState::Draft
            )
        {
            height += pr.checks.len() as f32 * 18.;
        }
    }

    px(height)
}

pub(crate) fn worktree_hover_popover_zone_bounds(
    left_pane_width: f32,
    popover: &WorktreeHoverPopover,
    worktree: &WorktreeSummary,
) -> Bounds<Pixels> {
    let padding = px(WORKTREE_HOVER_POPOVER_ZONE_PADDING_PX);
    Bounds::new(
        point(
            px(left_pane_width) + px(4.) - padding,
            popover.mouse_y - px(8.) - padding,
        ),
        size(
            px(WORKTREE_HOVER_POPOVER_CARD_WIDTH_PX) + padding * 2.,
            estimated_worktree_hover_popover_card_height(worktree, popover.checks_expanded)
                + padding * 2.,
        ),
    )
}

pub(crate) fn worktree_hover_trigger_zone_bounds(
    left_pane_width: f32,
    mouse_y: Pixels,
) -> Bounds<Pixels> {
    let height = px(WORKTREE_HOVER_TRIGGER_ZONE_HEIGHT_PX);
    Bounds::new(
        point(px(0.), mouse_y - height / 2.),
        size(px(left_pane_width), height),
    )
}

pub(crate) fn worktree_hover_safe_zone_contains(
    left_pane_width: f32,
    popover: &WorktreeHoverPopover,
    worktree: &WorktreeSummary,
    position: gpui::Point<Pixels>,
) -> bool {
    worktree_hover_popover_zone_bounds(left_pane_width, popover, worktree).contains(&position)
        || worktree_hover_trigger_zone_bounds(left_pane_width, popover.mouse_y).contains(&position)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use {
        super::*,
        arbor_core::agent::AgentState,
        gpui::{point, px},
    };

    #[test]
    fn attention_indicator_prefers_stuck_state() {
        let mut worktree = worktree_summary::tests::sample_worktree_summary();
        worktree.agent_state = Some(AgentState::Waiting);
        worktree.stuck_turn_count = 2;

        let attention = worktree_attention_indicator(&worktree);
        assert_eq!(attention.label, "Stuck");
    }

    #[test]
    fn worktree_hover_safe_zone_covers_trigger_row_and_popover() {
        let worktree = worktree_summary::tests::sample_worktree_summary();
        let popover = WorktreeHoverPopover {
            worktree_index: 0,
            mouse_y: px(100.),
            checks_expanded: false,
        };

        assert!(worktree_hover_safe_zone_contains(
            290.,
            &popover,
            &worktree,
            point(px(40.), px(100.)),
        ));
        assert!(worktree_hover_safe_zone_contains(
            290.,
            &popover,
            &worktree,
            point(px(320.), px(112.)),
        ));
        assert!(!worktree_hover_safe_zone_contains(
            290.,
            &popover,
            &worktree,
            point(px(700.), px(100.)),
        ));
    }

    #[test]
    fn expanded_checks_increase_worktree_hover_popover_height() {
        let mut worktree = worktree_summary::tests::sample_worktree_summary();
        worktree.pr_details = Some(github_service::PrDetails {
            number: 42,
            title: "Improve hover stability".to_owned(),
            url: "https://example.com/pr/42".to_owned(),
            state: github_service::PrState::Open,
            additions: 12,
            deletions: 4,
            review_decision: github_service::ReviewDecision::Pending,
            mergeable: github_service::MergeableState::Mergeable,
            merge_state_status: github_service::MergeStateStatus::Clean,
            passed_checks: 1,
            checks_status: github_service::CheckStatus::Pending,
            checks: vec![
                ("ci".to_owned(), github_service::CheckStatus::Pending),
                ("lint".to_owned(), github_service::CheckStatus::Success),
            ],
        });

        let collapsed = estimated_worktree_hover_popover_card_height(&worktree, false);
        let expanded = estimated_worktree_hover_popover_card_height(&worktree, true);
        let collapsed_bounds = worktree_hover_popover_zone_bounds(
            290.,
            &WorktreeHoverPopover {
                worktree_index: 0,
                mouse_y: px(120.),
                checks_expanded: false,
            },
            &worktree,
        );
        let expanded_bounds = worktree_hover_popover_zone_bounds(
            290.,
            &WorktreeHoverPopover {
                worktree_index: 0,
                mouse_y: px(120.),
                checks_expanded: true,
            },
            &worktree,
        );

        assert!(expanded > collapsed);
        assert!(expanded_bounds.size.height > collapsed_bounds.size.height);
    }

    #[test]
    fn status_metric_icon_svg_uses_non_private_use_shapes() {
        let cpu_svg = status_metric_icon_svg(StatusMetricIconKind::Cpu, 0x8A8986);
        let memory_svg = status_metric_icon_svg(StatusMetricIconKind::Memory, 0x8A8986);

        assert!(cpu_svg.contains("<svg"));
        assert!(cpu_svg.contains("#8A8986"));
        assert!(!cpu_svg.contains("f2db"));
        assert!(memory_svg.contains("<svg"));
        assert!(memory_svg.contains("#8A8986"));
        assert!(!memory_svg.contains("f538"));
    }
}
