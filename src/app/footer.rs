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
                ("f", "fetch"),
                ("a", "author"),
                ("p", "pull"),
                ("F", "flow"),
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
                ("c", "commit"),
                ("a", "author"),
                ("p", "pull"),
                ("P", "push"),
                ("f", "fetch"),
                ("F", "flow"),
                ("?", "help"),
            ],
        ),
        Pane::Branches => (
            3,
            "Branches",
            &[
                ("Enter", "checkout"),
                ("p", "pull"),
                ("a", "author"),
                ("f", "fetch"),
                ("F", "flow"),
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
                ("F", "flow"),
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
                        ("s", "source"),
                        ("l", "explain"),
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
                        ("R", "review"),
                        ("j/k", "scroll"),
                        ("g/G", "top/bot"),
                        ("p", "pull"),
                        ("a", "author"),
                        ("f", "fetch"),
                        ("F", "flow"),
                        ("?", "help"),
                    ],
                )
            }
        }
    }
}

pub(super) fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(40)]).split(area);
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
            let pairs = if state.flow_available() {
                &[("j/k", "select"), ("Enter", "continue"), ("Esc", "back")][..]
            } else {
                &[("Esc", "back")][..]
            };
            modal_spans("Flow ", pairs, Color::Cyan)
        }
        Modal::Conflict => modal_spans(
            "Conflict ",
            &[("v", "validate"), ("a", "abort"), ("Esc", "close")],
            Color::Red,
        ),
    };

    frame.render_widget(
        Paragraph::new(Line::from(left_spans)).alignment(Alignment::Left),
        chunks[0],
    );

    let (right_text, right_color) = status_text(state);
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
        if *key == "F" && !state.flow_available() {
            continue;
        }
        if *key == "p" && !state.pull_available() {
            continue;
        }
        spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(*label));
        if pairs.iter().skip(idx + 1).any(|(next_key, _)| {
            (*next_key != "F" || state.flow_available())
                && (*next_key != "p" || state.pull_available())
        }) {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
    }
    spans
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
