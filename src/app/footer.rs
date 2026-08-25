use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::panel::keys::{self, Section, Tone};
use crate::state::{AppState, DiffSource, Modal, Pane};

/// The section this pane's footer is built from.
fn footer_section(state: &AppState) -> Option<&'static Section> {
    keys::active_section(state.focus, state.main_keys())
}

/// The help section a modal's keys are documented in. Its footer is built from
/// the same bindings, so the two cannot say different things.
fn modal_section(modal: Modal) -> Option<&'static str> {
    Some(match modal {
        Modal::None => return None,
        Modal::Commit => "Commit modal",
        Modal::StageAllBeforeCommit => "Stage all",
        Modal::Push => "Push modal",
        Modal::Author => "Author settings",
        Modal::Model => "Settings",
        Modal::Help => "Help overlay",
        Modal::Flow => "Branch actions",
        Modal::Conflict => "Conflict",
        Modal::DeleteBranch => "Delete branch",
        Modal::Worktree => "New worktree",
        Modal::ReviewChat => "Review chat",
        Modal::ConfirmDestructive => "Confirm prompts",
    })
}

fn tone_color(tone: Tone) -> Color {
    match tone {
        Tone::Normal => Color::Cyan,
        Tone::Caution => Color::Yellow,
        Tone::Danger => Color::Red,
    }
}

/// The footer for an open modal, read off the key table.
///
/// The branch-action list is the one modal whose keys depend on the repository:
/// with no branches to act on there is nothing to select or run, so only the
/// way out is offered.
fn modal_footer_spans(state: &AppState, modal: Modal) -> Option<Vec<Span<'static>>> {
    let title = modal_section(modal)?;
    let section = keys::section(title)?;
    let footer = keys::modal_footer(title)?;
    let keys_shown: Vec<&'static str> = if modal == Modal::Flow && !state.branch_actions_available()
    {
        vec!["Esc"]
    } else {
        footer.order.to_vec()
    };
    let pairs: Vec<(&'static str, &'static str)> = keys_shown
        .iter()
        .filter_map(|key| keys::footer_entry(section, key))
        .collect();
    Some(modal_spans(footer.prefix, &pairs, tone_color(footer.tone)))
}

pub(super) fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let left_spans = match state.modal {
        Modal::None if state.session_view().is_some() && state.focus == Pane::Main => {
            session_spans(state)
        }
        Modal::None => default_spans(state),
        modal => modal_footer_spans(state, modal).unwrap_or_default(),
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
    let Some(section) = footer_section(state) else {
        return Vec::new();
    };
    let (n, name) = section.footer_meta.unwrap_or((0, section.title));
    let pairs: Vec<(&'static str, &'static str)> = section
        .footer_order
        .iter()
        .filter_map(|key| keys::footer_entry(section, key))
        .collect();
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
        // Only a session row has a session to close.
        ("x", "close session") => crate::panel::environments::selected_session(state).is_some(),
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
    pairs: &[(&'static str, &'static str)],
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
            let text = match state.activity_detail() {
                Some(step) => format!("{spinner} {label}: {step}"),
                None if status.text.starts_with(label) => format!("{spinner} {}", status.text),
                None => format!("{spinner} {label}: {}", status.text),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every modal lg can open has a footer, and every key that footer prints is
    /// a binding the help documents.
    ///
    /// Modals used to spell their footers out here, in a table nothing checked
    /// against the help. That is how the settings modal came to be documented as
    /// saving on Enter while its footer offered Ctrl+S — both true of different
    /// keys, and only one of them the save.
    #[test]
    fn every_modal_footer_is_built_from_the_key_table() {
        let modals = [
            Modal::Commit,
            Modal::StageAllBeforeCommit,
            Modal::Push,
            Modal::Author,
            Modal::Model,
            Modal::Help,
            Modal::Flow,
            Modal::Conflict,
            Modal::DeleteBranch,
            Modal::Worktree,
            Modal::ReviewChat,
            Modal::ConfirmDestructive,
        ];
        let state = AppState::new();
        for modal in modals {
            let title =
                modal_section(modal).unwrap_or_else(|| panic!("{modal:?} names no help section"));
            let section =
                keys::section(title).unwrap_or_else(|| panic!("{title} is not in the key table"));
            let footer = keys::modal_footer(title)
                .unwrap_or_else(|| panic!("{title} has no footer to print"));
            for key in footer.order {
                assert!(
                    keys::footer_entry(section, key).is_some(),
                    "{title} prints {key:?}, which its own section does not document"
                );
            }
            assert!(
                modal_footer_spans(&state, modal).is_some_and(|spans| spans.len() > 1),
                "{modal:?} draws an empty footer"
            );
        }
        assert!(
            modal_section(Modal::None).is_none(),
            "no modal is open, so there is no modal footer"
        );
    }

    /// The footer and the help overlay ask the same question about which pane is
    /// live, so a pane cannot end up with hints from one section and a help
    /// heading from another.
    #[test]
    fn the_diff_pane_switches_sections_with_its_contents() {
        let mut state = AppState::new();
        state.focus = Pane::Main;
        assert_eq!(
            footer_section(&state).map(|section| section.title),
            Some("Diff pane")
        );
        assert_eq!(
            keys::active_section(Pane::Main, state.main_keys()).map(|section| section.title),
            Some("Diff pane")
        );
    }
}
