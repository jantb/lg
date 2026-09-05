//! Drawing the review tree: one line per node, with its diff and its notes.

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use std::collections::HashSet;

use crate::{
    panel::markdown,
    state::{AppState, ReviewStyleSeverity},
    ui,
};

use super::super::source::{RenderCache, owned_spans, review_source_context_lines};
use super::*;

const SUSPICIOUS_REVIEW_BG: Color = Color::Rgb(78, 57, 18);
const OK_REVIEW_STYLE_BG: Color = Color::Rgb(24, 54, 34);
const FAIL_REVIEW_STYLE_BG: Color = Color::Rgb(70, 24, 28);
/// The background behind a path being flagged breathes between these two,
/// trough and peak, once per `ACTIVE_REVIEW_PULSE_MS`.
const ACTIVE_REVIEW_STYLE_DIM: (u8, u8, u8) = (22, 46, 56);
const ACTIVE_REVIEW_STYLE_BRIGHT: (u8, u8, u8) = (42, 106, 120);
const ACTIVE_REVIEW_PULSE_MS: u64 = 1_440;

pub(in crate::panel::main) fn render(
    state: &AppState,
    area: Rect,
    frame: &mut Frame,
    focused: bool,
) {
    let title = if side_by_side_diff_enabled(state) {
        "Review: side-by-side"
    } else {
        "Review"
    };
    let block = ui::framed_with_activity(
        0,
        title,
        focused,
        None,
        state.animation_ms,
        state.activity_label().is_some(),
    )
    .title_bottom(
        Line::from(Span::styled(
            "j/k move  ↑/↓ source changes  Enter/s source  space expand  d drill  v view  f flag  l llm/pr  y copy  C chat  R refresh",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))
        .alignment(Alignment::Right),
    );
    let Some(_) = &state.review else {
        return;
    };
    let lines = render_lines(state, focused, area.width.saturating_sub(2));

    // Use the lines we just built as the source of truth for the scroll bound.
    // Avoids a second walk over every visible node (each of which would
    // re-read source files and re-parse diff overlays) and stays correct when
    // markdown::render word-wraps assist output into more lines than the raw
    // source had.
    let max_offset = crate::panel::main::scroll_bound(lines.len(), area.height.saturating_sub(2));
    let offset = state.diff_offset.min(max_offset);
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));
    frame.render_widget(para, area);
}

pub(in crate::panel::main) fn render_lines(
    state: &AppState,
    focused: bool,
    wrap_width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(review) = &state.review else {
        return lines;
    };

    // Precompute parent membership sets so the inner loop is O(1) per node
    // instead of repeatedly scanning every node. Built once per frame.
    let mut parents_with_child: HashSet<&str> = HashSet::new();
    let mut parents_with_drill_child: HashSet<&str> = HashSet::new();
    for candidate in &review.nodes {
        if let Some(parent) = candidate.parent.as_deref() {
            parents_with_child.insert(parent);
            if is_review_file_node(&candidate.id) || is_review_entry_node(&candidate.id) {
                parents_with_drill_child.insert(parent);
            }
        }
    }

    // File-read cache shared across this frame's source-context renders.
    let mut cache = RenderCache::default();
    let body_style = Style::default().fg(Color::DarkGray);

    for idx in visible_review_node_indices(state) {
        let node = &review.nodes[idx];
        let selected = focused && state.review_idx == idx;
        let node_id = node.id.as_str();
        let has_children = parents_with_child.contains(node_id);
        let has_body = renders_review_body(node_id) && !node.body.is_empty();
        let drillable = parents_with_drill_child.contains(node_id);
        let expanded = !state.review_collapsed.contains(node_id);
        let context_open = state.review_context_open.contains(node_id);
        let assist = review_assist_text(state, node_id);
        let marker = if has_children || has_body {
            if expanded { "▾" } else { "▸" }
        } else {
            " "
        };
        let indent = review_indent(node.depth);
        lines.push(review_title_line(
            &indent,
            marker,
            &node.title,
            node.depth,
            selected,
            drillable,
            state,
        ));

        if expanded || context_open || assist.is_some() {
            let syntax_path = review_node_syntax_path(&node.title);
            // Each section/marker line repeats this prefix — render it once.
            let body_prefix = format!("{indent}  │ ");
            if expanded && renders_review_body(node_id) {
                if let Some(path) = syntax_path {
                    if side_by_side_diff_enabled(state) {
                        lines.extend(prefixed_side_by_side_diff_lines(
                            &node.body,
                            path,
                            &body_prefix,
                            body_style,
                            wrap_width,
                        ));
                    } else {
                        lines.extend(prefixed_unified_diff_lines(
                            &node.body,
                            path,
                            &body_prefix,
                            body_style,
                            wrap_width,
                        ));
                    }
                } else {
                    lines.extend(markdown::render(
                        &node.body.join("\n"),
                        &body_prefix,
                        wrap_width,
                    ));
                }
            }
            if context_open {
                lines.extend(review_source_context_lines(
                    &mut cache,
                    state,
                    review,
                    node,
                    syntax_path,
                    &indent,
                ));
            }
            if let Some(assist) = assist {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  │ llm"),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.extend(markdown::render(assist, &body_prefix, wrap_width));
            }
            if (expanded && has_body) || context_open || assist.is_some() {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  └─"),
                    body_style,
                )));
            }
        }
    }
    lines
}

fn prefixed_side_by_side_diff_lines(
    body: &[String],
    path: &str,
    prefix: &str,
    prefix_style: Style,
    width: u16,
) -> Vec<Line<'static>> {
    prefixed_diff_lines(
        ui::highlight_side_by_side_diff_text_for_path(
            &body.join("\n"),
            diff_body_width(width, prefix),
            path,
        ),
        prefix,
        prefix_style,
    )
}

/// Unified diff body, wrapped to what is left of the pane beside the tree
/// prefix so long lines stay readable instead of running off the edge.
fn prefixed_unified_diff_lines(
    body: &[String],
    path: &str,
    prefix: &str,
    prefix_style: Style,
    width: u16,
) -> Vec<Line<'static>> {
    let diff_width = diff_body_width(width, prefix);
    prefixed_diff_lines(
        body.iter()
            .flat_map(|line| ui::highlight_diff_line_wrapped_for_path(line, path, diff_width))
            .collect(),
        prefix,
        prefix_style,
    )
}

fn prefixed_diff_lines(
    lines: Vec<Line<'static>>,
    prefix: &str,
    prefix_style: Style,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
            spans.extend(owned_spans(line));
            Line::from(spans)
        })
        .collect()
}

/// Width a diff body gets once the tree prefix has taken its share.
pub(in crate::panel::main) fn diff_body_width(width: u16, prefix: &str) -> u16 {
    width.saturating_sub(prefix.chars().count().min(u16::MAX as usize) as u16)
}

pub(in crate::panel::main) fn review_title_line(
    indent: &str,
    marker: &str,
    title: &str,
    depth: u16,
    selected: bool,
    drillable: bool,
    state: &AppState,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{indent}{marker} "),
        selected_style(
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
            selected,
        ),
    )];
    spans.extend(review_title_spans(title, depth, selected, state));
    if drillable {
        spans.push(Span::styled(
            " ↳".to_string(),
            selected_style(
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
                selected,
            ),
        ));
    }
    Line::from(spans)
}

fn review_title_spans(
    title: &str,
    depth: u16,
    selected: bool,
    state: &AppState,
) -> Vec<Span<'static>> {
    if depth == 0 {
        return vec![Span::styled(
            title.to_string(),
            selected_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                selected,
            ),
        )];
    }

    if let Some((path, rest)) = title.split_once(" in ")
        && let Some((symbol, description)) = rest.split_once(" - ")
    {
        let mut spans = styled_file_path(path, selected, review_path_style(path, state));
        spans.push(Span::styled(
            " in ".to_string(),
            selected_style(Style::default().fg(Color::DarkGray), selected),
        ));
        spans.push(Span::styled(
            symbol.to_string(),
            selected_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                selected,
            ),
        ));
        spans.push(Span::styled(
            " - ".to_string(),
            selected_style(Style::default().fg(Color::DarkGray), selected),
        ));
        spans.extend(styled_review_description(description, selected));
        return spans;
    }

    if let Some((location, description)) = title.split_once(" - ") {
        let mut spans = styled_file_location(location, selected, state);
        spans.push(Span::styled(
            " - ".to_string(),
            selected_style(Style::default().fg(Color::DarkGray), selected),
        ));
        spans.extend(styled_review_description(description, selected));
        return spans;
    }

    styled_review_description(title, selected)
}

fn styled_file_location(location: &str, selected: bool, state: &AppState) -> Vec<Span<'static>> {
    let Some((path, line)) = location.rsplit_once(':') else {
        return styled_file_path(location, selected, review_path_style(location, state));
    };
    if line.chars().all(|ch| ch.is_ascii_digit()) {
        let mut spans = styled_file_path(path, selected, review_path_style(path, state));
        spans.push(Span::styled(
            format!(":{line}"),
            selected_style(Style::default().fg(Color::LightBlue), selected),
        ));
        spans
    } else {
        styled_file_path(location, selected, review_path_style(location, state))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewPathStyle {
    Normal,
    /// Being flagged right now; carries the animation clock in milliseconds.
    Active(u64),
    Finding(ReviewStyleSeverity),
}

fn review_path_style(path: &str, state: &AppState) -> ReviewPathStyle {
    if let Some(finding) = state.review_style_findings.get(path) {
        ReviewPathStyle::Finding(finding.severity)
    } else if state.review_flag_active_path.as_deref() == Some(path) {
        ReviewPathStyle::Active(state.animation_ms)
    } else {
        ReviewPathStyle::Normal
    }
}

fn styled_file_path(path: &str, selected: bool, path_style: ReviewPathStyle) -> Vec<Span<'static>> {
    let mut file_style = if is_test_review_node(path) {
        Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD)
    };
    match path_style {
        ReviewPathStyle::Finding(severity) => {
            file_style = file_style.bg(review_style_bg(severity));
        }
        ReviewPathStyle::Active(clock_ms) => {
            file_style = file_style
                .bg(active_review_style_bg(clock_ms))
                .add_modifier(Modifier::BOLD);
        }
        ReviewPathStyle::Normal => {}
    }
    let style = if path_style != ReviewPathStyle::Normal && selected {
        file_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        selected_style(file_style, selected)
    };
    let mut spans = Vec::new();
    if let ReviewPathStyle::Active(clock_ms) = path_style {
        spans.push(Span::styled(
            format!(" {} FLAGGING ", active_review_marker(clock_ms)),
            Style::default()
                .fg(Color::Black)
                .bg(active_review_style_bg(clock_ms))
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(path.to_string(), style));
    spans
}

pub(in crate::panel::main) fn active_review_style_bg(clock_ms: u64) -> Color {
    crate::ui::palette::breathe(
        ACTIVE_REVIEW_STYLE_DIM,
        ACTIVE_REVIEW_STYLE_BRIGHT,
        clock_ms,
        ACTIVE_REVIEW_PULSE_MS,
    )
}

fn active_review_marker(clock_ms: u64) -> &'static str {
    let tick = clock_ms / (2 * crate::config::ANIMATION_STEP_MS);
    match tick % 4 {
        0 => "◌",
        1 => "◐",
        2 => "●",
        _ => "◑",
    }
}

pub(in crate::panel::main) fn review_style_bg(severity: ReviewStyleSeverity) -> Color {
    match severity {
        ReviewStyleSeverity::Ok => OK_REVIEW_STYLE_BG,
        ReviewStyleSeverity::Warn => SUSPICIOUS_REVIEW_BG,
        ReviewStyleSeverity::Fail => FAIL_REVIEW_STYLE_BG,
    }
}

fn styled_review_description(description: &str, selected: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = description;
    while let Some(start) = rest.find('(') {
        let (prefix, tail) = rest.split_at(start);
        if !prefix.is_empty() {
            spans.push(Span::styled(
                prefix.to_string(),
                selected_style(Style::default().fg(Color::Gray), selected),
            ));
        }
        if let Some(end) = tail.find(')') {
            let token = &tail[..=end];
            spans.extend(styled_change_token(token, selected));
            rest = &tail[end + 1..];
        } else {
            spans.push(Span::styled(
                tail.to_string(),
                selected_style(Style::default().fg(Color::Gray), selected),
            ));
            return spans;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(
            rest.to_string(),
            selected_style(Style::default().fg(Color::Gray), selected),
        ));
    }
    spans
}

fn styled_change_token(token: &str, selected: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for part in token.split_inclusive(' ') {
        let trimmed = part.trim_matches(|ch| ch == '(' || ch == ')' || ch == ' ');
        let style = if trimmed.starts_with('+') {
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else if trimmed.starts_with('-') {
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(
            part.to_string(),
            selected_style(style, selected),
        ));
    }
    spans
}

fn selected_style(style: Style, selected: bool) -> Style {
    if selected {
        style.bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    } else {
        style
    }
}
