use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::state::{AppState, DiffSource, Modal, Pane};

fn footer_spec(state: &AppState) -> (u8, &'static str, &'static [(&'static str, &'static str)]) {
    match state.focus {
        Pane::Status => (
            1,
            "Status",
            &[
                ("j/k", "repo tree"),
                ("Enter", "expand/checkout"),
                ("o", "open IDE"),
                ("r", "remotes"),
                ("Esc", "back"),
                ("f", "fetch"),
                ("a", "author"),
                ("p", "pull"),
                ("?", "help"),
                ("q", "quit"),
            ],
        ),
        Pane::Files => (
            2,
            "Files",
            &[
                ("space", "stage"),
                ("u", "unstage"),
                ("A/U", "all"),
                ("i", "ignore"),
                ("d", "delete"),
                ("o", "open IDE"),
                ("c", "commit"),
                ("a", "author"),
                ("p", "pull"),
                ("P", "push"),
                ("f", "fetch"),
                ("?", "help"),
            ],
        ),
        Pane::Branches => (
            3,
            "Branches",
            &[
                ("Enter", "checkout"),
                ("r", "remotes"),
                ("m", "pull/merge main"),
                ("M", "sync all"),
                ("d", "drop local"),
                ("D", "delete"),
                ("o", "open IDE"),
                ("p", "pull"),
                ("a", "author"),
                ("f", "fetch"),
                ("F", "actions"),
                ("?", "help"),
            ],
        ),
        Pane::Commits => (
            4,
            "Commits",
            &[
                ("j/k", "navigate"),
                ("Enter", "focus diff"),
                ("p", "pull"),
                ("a", "author"),
                ("f", "fetch"),
                ("?", "help"),
            ],
        ),
        Pane::Main => {
            if matches!(state.diff_source, DiffSource::Review) && state.review.is_some() {
                (
                    0,
                    "Review",
                    &[
                        ("j/k", "move"),
                        ("Enter/space", "expand"),
                        ("d", "drill"),
                        ("s", "source"),
                        ("o", "open IDE"),
                        ("l", "explain"),
                        ("C", "chat"),
                        ("g/G", "top/bot"),
                        ("f", "fetch"),
                        ("a", "author"),
                        ("R", "refresh"),
                        ("?", "help"),
                    ],
                )
            } else {
                (
                    0,
                    "Diff",
                    &[
                        ("R", "review mode"),
                        ("o", "open IDE"),
                        ("j/k", "scroll"),
                        ("g/G", "top/bot"),
                        ("p", "pull"),
                        ("a", "author"),
                        ("f", "fetch"),
                        ("?", "help"),
                    ],
                )
            }
        }
    }
}

pub(super) fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let left_spans = match state.modal {
        Modal::None => default_spans(state),
        Modal::Commit => modal_spans(
            "Commit modal ",
            &[
                ("Ctrl+S", "commit"),
                ("Enter", "newline"),
                ("Ctrl+R", "regen"),
                ("Esc", "cancel"),
            ],
            Color::Cyan,
        ),
        Modal::StageAllBeforeCommit => modal_spans(
            "Commit ",
            &[("y", "stage all"), ("n/Esc", "cancel")],
            Color::Yellow,
        ),
        Modal::Push => modal_spans(
            "Push modal ",
            &[("Enter", "push"), ("Esc", "cancel")],
            Color::Cyan,
        ),
        Modal::Author => modal_spans(
            "Author ",
            &[
                ("Tab", "field"),
                ("Enter", "save subtree"),
                ("Ctrl+L", "save local"),
                ("Ctrl+U", "clear subtree"),
                ("Ctrl+X", "clear local"),
                ("Esc", "cancel"),
            ],
            Color::Cyan,
        ),
        Modal::Help => modal_spans("Help ", &[("any key", "close")], Color::Cyan),
        Modal::Flow => {
            let pairs = if state.branch_actions_available() {
                &[("j/k", "select"), ("Enter", "continue"), ("Esc", "back")][..]
            } else {
                &[("Esc", "back")][..]
            };
            modal_spans("Branches ", pairs, Color::Cyan)
        }
        Modal::Conflict => modal_spans(
            "Conflict ",
            &[("v", "validate"), ("a", "abort"), ("Esc", "close")],
            Color::Red,
        ),
        Modal::DeleteBranch => modal_spans(
            "Delete branch ",
            &[
                ("Tab", "field"),
                ("Space", "toggle"),
                ("Enter", "confirm"),
                ("Esc", "cancel"),
            ],
            Color::Red,
        ),
        Modal::ReviewChat => modal_spans(
            "Review chat ",
            &[("Enter", "send"), ("Esc", "close")],
            Color::Cyan,
        ),
    };

    let (right_text, right_color) = status_text(state);
    let right_width = right_text.chars().count().min(area.width as usize) as u16;
    let chunks =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).split(area);

    frame.render_widget(
        Paragraph::new(Line::from(left_spans)).alignment(Alignment::Left),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Span::styled(right_text, Style::default().fg(right_color)))
            .alignment(Alignment::Right),
        chunks[1],
    );
}

fn default_spans(state: &AppState) -> Vec<Span<'static>> {
    let (n, name, pairs) = footer_spec(state);
    let mut spans = vec![Span::styled(
        format!("[{n}] {name} "),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    for (idx, (key, label)) in pairs.iter().enumerate() {
        if *key == "F" && !state.branch_actions_available() {
            continue;
        }
        if *key == "p" && !state.pull_available() {
            continue;
        }
        spans.push(Span::styled(*key, shortcut_style(state, key)));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(*label));
        if pairs.iter().skip(idx + 1).any(|(next_key, _)| {
            (*next_key != "F" || state.branch_actions_available())
                && (*next_key != "p" || state.pull_available())
        }) {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
    }
    spans
}

fn shortcut_style(state: &AppState, key: &str) -> Style {
    if key == "d" && review_drill_available(state) {
        Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    }
}

fn review_drill_available(state: &AppState) -> bool {
    if !matches!(state.focus, Pane::Main) || !matches!(state.diff_source, DiffSource::Review) {
        return false;
    }
    let Some(review) = &state.review else {
        return false;
    };
    let Some(node) = review.nodes.get(state.review_idx) else {
        return false;
    };
    review.nodes.iter().any(|candidate| {
        candidate.parent.as_deref() == Some(node.id.as_str())
            && (candidate.id.contains(":file:") || candidate.id.contains(":entry:"))
    })
}

fn modal_spans(
    title: &'static str,
    pairs: &'static [(&'static str, &'static str)],
    color: Color,
) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        title,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )];
    for (idx, (key, label)) in pairs.iter().enumerate() {
        spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(*label));
        if idx + 1 < pairs.len() {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
    }
    spans
}

fn status_text(state: &AppState) -> (String, Color) {
    match (&state.status, state.activity_label()) {
        (Some(status), Some(label)) if !status.is_error => {
            let spinner = crate::state::SPINNER_FRAMES
                [state.animation_tick % crate::state::SPINNER_FRAMES.len()];
            let text = if status.text.starts_with(label) {
                format!("{spinner} {}", status.text)
            } else {
                format!("{spinner} {label}: {}", status.text)
            };
            (text, Color::Cyan)
        }
        (Some(status), _) => {
            let icon = if status.is_error {
                "\u{2717}"
            } else {
                "\u{2713}"
            };
            (
                format!("{icon} {}", status.text),
                if status.is_error {
                    Color::Red
                } else {
                    Color::Green
                },
            )
        }
        (None, Some(label)) => {
            let spinner = crate::state::SPINNER_FRAMES
                [state.animation_tick % crate::state::SPINNER_FRAMES.len()];
            (format!("{spinner} {label}\u{2026}"), Color::Cyan)
        }
        (None, None) => (
            format!(
                "\u{2022} {}",
                state.branch.as_deref().unwrap_or("no branch")
            ),
            Color::DarkGray,
        ),
    }
}
