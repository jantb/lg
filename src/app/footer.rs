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
                ("n", "new worktree"),
                ("m", "land worktree"),
                ("b", "branch home"),
                ("s", "claude session"),
                ("o", "open IDE"),
                ("r", "remotes"),
                ("Esc", "back"),
                ("f", "fetch"),
                ("a", "author"),
                ("L", "model"),
                ("p", "pull"),
                ("F2", "workspace"),
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
                ("r", "rollback"),
                ("i", "ignore"),
                ("d", "delete"),
                ("o", "open IDE"),
                ("c", "commit"),
                ("a", "author"),
                ("L", "model"),
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
                ("u", "set upstream"),
                ("p", "pull"),
                ("a", "author"),
                ("L", "model"),
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
                ("L", "model"),
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
                        ("Enter/s", "source"),
                        ("space", "expand"),
                        ("d", "drill"),
                        ("n/N", "notes"),
                        ("o", "open IDE"),
                        ("l", "explain"),
                        ("C", "chat"),
                        ("g/G", "top/bot"),
                        ("v", "view"),
                        ("f", "flag"),
                        ("a", "author"),
                        ("L", "model"),
                        ("R", "refresh"),
                        ("Esc", "cancel"),
                        ("?", "help"),
                    ],
                )
            } else {
                (
                    0,
                    "Diff",
                    &[
                        ("R", "review mode"),
                        ("v", "view"),
                        ("o", "open IDE"),
                        ("j/k", "scroll"),
                        ("g/G", "top/bot"),
                        ("p", "pull"),
                        ("a", "author"),
                        ("L", "model"),
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
        Modal::None if state.session_view().is_some() && state.focus == Pane::Main => {
            session_spans(state)
        }
        Modal::None => default_spans(state),
        Modal::Commit => modal_spans(
            "Commit modal ",
            &[
                ("Ctrl+S", "commit"),
                ("Enter", "newline"),
                ("Ctrl+R", "regen"),
                ("Ctrl+U", "clear"),
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
        Modal::Worktree => modal_spans(
            "New worktree ",
            &[("Tab", "field"), ("Enter", "create"), ("Esc", "cancel")],
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
        Modal::Model => modal_spans(
            "Settings ",
            &[
                ("Up/Down", "select"),
                ("Enter", "open/confirm"),
                ("Ctrl+S", "save"),
                ("Ctrl+U", "reset"),
                ("Esc", "cancel"),
            ],
            Color::Cyan,
        ),
        Modal::Help => modal_spans(
            "Help ",
            &[("j/k", "scroll"), ("g/G", "top/bot"), ("q/Esc", "close")],
            Color::Cyan,
        ),
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
            &[
                ("j/k", "select"),
                ("o/Enter", "open"),
                ("v", "validate"),
                ("a", "abort"),
                ("Esc", "close"),
            ],
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
        Modal::ConfirmDestructive => modal_spans(
            "Confirm ",
            &[("y", "confirm"), ("n/Esc", "cancel")],
            Color::Red,
        ),
        Modal::ReviewChat => modal_spans(
            "Review chat ",
            &[("Enter", "send"), ("Esc", "close")],
            Color::Cyan,
        ),
    };

    let (right_text, right_color) = status_text(state);
    // Never let a long status swallow the whole bar; the shortcut hints must stay readable.
    let status_budget = (area.width as usize).div_ceil(2).max(1);
    let right_width = right_text.chars().count().min(status_budget) as u16;
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

/// Footer for the session pane. Which way the keyboard is pointing is the one
/// thing that must never be in doubt.
fn session_spans(state: &AppState) -> Vec<Span<'static>> {
    let (title, color, pairs): (_, _, &[(&str, &str)]) = if state.session_capture {
        (
            "input \u{2192} claude ",
            Color::Green,
            &[(
                "Ctrl-]",
                "keyboard back to lg \u{2014} then x closes it, Ctrl-n/p switches",
            )],
        )
    } else {
        (
            "Session ",
            Color::Cyan,
            &[
                ("i", "type into it"),
                ("x", "close"),
                ("Backspace", "back to diff"),
                ("F2", "git view"),
                ("?", "help"),
                ("q", "quit"),
            ],
        )
    };
    modal_spans(title, pairs, color)
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
        if !shortcut_visible(state, key, label) {
            continue;
        }
        spans.push(Span::styled(*key, shortcut_style(state, key)));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(*label));
        if pairs
            .iter()
            .skip(idx + 1)
            .any(|(next_key, next_label)| shortcut_visible(state, next_key, next_label))
        {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
    }
    spans
}

/// Hide shortcuts that would do nothing if pressed right now.
fn shortcut_visible(state: &AppState, key: &str, label: &str) -> bool {
    match (key, label) {
        ("F", _) => state.branch_actions_available(),
        ("p", _) => state.pull_available(),
        ("v", _) => diff_view_toggle_available(state),
        // Handing a branch over needs a worktree to hand it over from.
        ("m", "land worktree") | ("b", "branch home") => {
            crate::panel::environments::selected_linked_worktree(state).is_some()
        }
        // Distinguished from the Status pane's Esc, which means "back".
        ("Esc", "cancel") => state.llm_job_running(),
        _ => true,
    }
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

fn diff_view_toggle_available(state: &AppState) -> bool {
    matches!(state.focus, Pane::Main)
        && !matches!(state.diff_source, DiffSource::Branch(_))
        && (!matches!(state.diff_source, DiffSource::Review) || state.review.is_some())
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
                "llm {}/{} \u{2022} {}",
                state.llm_provider.label(),
                compact_model(&state.llm_model),
                state.branch.as_deref().unwrap_or("no branch")
            ),
            Color::DarkGray,
        ),
    }
}

fn compact_model(model: &str) -> String {
    const MAX: usize = 34;
    if model.chars().count() <= MAX {
        return model.to_string();
    }
    let mut out = String::new();
    for ch in model.chars().take(MAX.saturating_sub(3)) {
        out.push(ch);
    }
    out.push_str("...");
    out
}
