//! The pull request list, the selected pull request's details, and the keys
//! offered for it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::github::{Mergeable, PrState, PullRequest, ReviewDecision};
use crate::panel::github::lookup::{is_current_branch, is_mine, selected_pr};
use crate::panel::keys;
use crate::state::AppState;
use crate::ui::palette;

use super::{LABEL, draw_placeholder, truncate};

fn checks_span(pr: &PullRequest) -> Option<Span<'static>> {
    let checks = &pr.checks;
    if checks.is_empty() {
        None
    } else if !checks.failed.is_empty() {
        Some(Span::styled("\u{2717}", Style::default().fg(Color::Red)))
    } else if checks.pending > 0 {
        Some(Span::styled("\u{25cc}", Style::default().fg(Color::Yellow)))
    } else {
        Some(Span::styled("\u{2713}", Style::default().fg(Color::Green)))
    }
}

fn review_style(review: ReviewDecision) -> Style {
    match review {
        ReviewDecision::Approved => Style::default().fg(Color::Green),
        ReviewDecision::ChangesRequested => Style::default().fg(Color::Red),
        ReviewDecision::ReviewRequired | ReviewDecision::None => Style::default().fg(Color::Yellow),
    }
}

fn state_style(state: PrState) -> Style {
    match state {
        PrState::Open => Style::default().fg(Color::Green),
        PrState::Merged => Style::default().fg(Color::Magenta),
        PrState::Closed => Style::default().fg(Color::Red),
    }
}

pub(super) fn draw_pr_list(state: &AppState, area: Rect, frame: &mut Frame) {
    let gh = &state.github;
    if gh.prs.is_empty() {
        let (text, error) = match (&gh.prs_error, gh.prs_loading.is_some()) {
            (Some(err), _) => (err.clone(), true),
            (None, true) => ("reading pull requests\u{2026}".to_string(), false),
            (None, false) => (
                format!(
                    "no pull requests under {} \u{2014} n opens one for this branch",
                    gh.filter.label()
                ),
                false,
            ),
        };
        draw_placeholder(&text, error, area, frame);
        return;
    }
    let width = area.width.saturating_sub(2) as usize;
    // Every number gets the room of the longest, so the titles line up.
    let digits = gh
        .prs
        .iter()
        .map(|pr| pr.number.to_string().len())
        .max()
        .unwrap_or(1);
    // The authors get a column of their own, as wide as the longest name
    // up to a limit; the user's own pull requests just say "you".
    let author_name = |pr: &PullRequest| {
        if is_mine(gh, pr) {
            "you".to_string()
        } else {
            pr.author.clone()
        }
    };
    let author_width = gh
        .prs
        .iter()
        .map(|pr| author_name(pr).chars().count())
        .max()
        .unwrap_or(0)
        .min(14);
    let items: Vec<ListItem> = gh
        .prs
        .iter()
        .map(|pr| {
            let here = is_current_branch(state, pr);
            let mine = is_mine(gh, pr);
            let mut spans = vec![
                Span::styled(
                    if here { "\u{25cf} " } else { "  " },
                    Style::default().fg(palette::ACCENT),
                ),
                Span::styled(format!("#{:<digits$} ", pr.number), state_style(pr.state)),
            ];
            let mut marks: Vec<Span> = Vec::new();
            if author_width > 0 {
                let style = if mine {
                    Style::default()
                        .fg(palette::ACCENT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette::author(&pr.author))
                };
                marks.push(Span::styled(
                    format!(
                        " {:<author_width$}",
                        truncate(&author_name(pr), author_width)
                    ),
                    style,
                ));
            }
            if let Some(checks) = checks_span(pr) {
                marks.push(Span::raw(" "));
                marks.push(checks);
            }
            if pr.review == ReviewDecision::Approved {
                marks.push(Span::styled(" \u{2714}", review_style(pr.review)));
            } else if pr.review == ReviewDecision::ChangesRequested {
                marks.push(Span::styled(" \u{b1}", review_style(pr.review)));
            }
            let used = 2
                + digits
                + 2
                + marks
                    .iter()
                    .map(|s| s.content.chars().count())
                    .sum::<usize>();
            let room = width.saturating_sub(used + 1);
            let title_style = if pr.draft || !pr.is_open() {
                Style::default().fg(palette::TEXT_IDLE)
            } else if mine {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            // A merged or closed one says so in words; the colour of its
            // number alone is easy to miss.
            let tag = match pr.state {
                PrState::Open if pr.draft => Some(("draft \u{b7} ", palette::LANE_TEST)),
                PrState::Open => None,
                PrState::Merged => Some(("merged \u{b7} ", Color::Magenta)),
                PrState::Closed => Some(("closed \u{b7} ", Color::Red)),
            };
            if let Some((label, color)) = tag {
                spans.push(Span::styled(label, Style::default().fg(color)));
                let room = room.saturating_sub(label.chars().count());
                spans.push(Span::styled(
                    format!("{:<room$}", truncate(&pr.title, room)),
                    title_style,
                ));
            } else {
                spans.push(Span::styled(
                    format!("{:<room$}", truncate(&pr.title, room)),
                    title_style,
                ));
            }
            spans.extend(marks);
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(palette::selection())
        .highlight_symbol("\u{203a}");
    let visible = area.height as usize;
    let offset = gh.pr_idx.saturating_sub(visible.saturating_sub(1));
    let mut list_state = crate::panel::scroll::list_state(Some(gh.pr_idx), offset);
    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(super) fn draw_pr_detail(state: &AppState, area: Rect, frame: &mut Frame) {
    let Some(pr) = selected_pr(state) else {
        return;
    };
    let width = area.width.saturating_sub(1);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format!("#{} ", pr.number), state_style(pr.state)),
        Span::styled(
            pr.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let head = match &pr.head_owner {
        Some(owner) => format!("{owner}:{}", pr.head),
        None => pr.head.clone(),
    };
    let author = if is_mine(&state.github, pr) {
        Span::styled(
            format!("{} (you)", pr.author),
            Style::default()
                .fg(palette::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            pr.author.clone(),
            Style::default().fg(palette::author(&pr.author)),
        )
    };
    lines.push(Line::from(vec![
        author,
        Span::styled(" wants to merge ", LABEL),
        Span::styled(head, Style::default().fg(palette::LANE_FEATURE)),
        Span::styled(" \u{2192} ", LABEL),
        Span::styled(pr.base.clone(), Style::default().fg(palette::LANE_MAIN)),
    ]));

    let mut status = vec![Span::styled(pr.state.label(), state_style(pr.state))];
    if pr.draft {
        status.push(Span::styled(" \u{b7} draft", LABEL));
    }
    if let Some(review) = pr.review.label() {
        status.push(Span::styled(" \u{b7} ", LABEL));
        status.push(Span::styled(review, review_style(pr.review)));
    }
    if pr.is_open() {
        match pr.mergeable {
            Mergeable::Yes => status.push(Span::styled(
                " \u{b7} no conflicts",
                Style::default().fg(Color::Green),
            )),
            Mergeable::Conflicting => status.push(Span::styled(
                " \u{b7} has conflicts",
                Style::default().fg(Color::Red),
            )),
            Mergeable::Unknown => {}
        }
    }
    lines.push(Line::from(status));

    lines.push(Line::from(vec![
        Span::styled(
            format!("+{}", pr.additions),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("\u{2212}{}", pr.deletions),
            Style::default().fg(Color::Red),
        ),
        Span::styled(
            format!(
                " in {} file{}",
                pr.changed_files,
                if pr.changed_files == 1 { "" } else { "s" }
            ),
            LABEL,
        ),
        Span::styled(
            pr.updated_at
                .split('T')
                .next()
                .filter(|day| !day.is_empty())
                .map(|day| format!(" \u{b7} updated {day}"))
                .unwrap_or_default(),
            LABEL,
        ),
    ]));

    let checks = &pr.checks;
    if !checks.is_empty() {
        let mut spans = vec![Span::styled("checks ", LABEL)];
        if checks.passed > 0 {
            spans.push(Span::styled(
                format!("{} passed ", checks.passed),
                Style::default().fg(Color::Green),
            ));
        }
        if !checks.failed.is_empty() {
            spans.push(Span::styled(
                format!("{} failed ", checks.failed.len()),
                Style::default().fg(Color::Red),
            ));
        }
        if checks.pending > 0 {
            spans.push(Span::styled(
                format!("{} running", checks.pending),
                Style::default().fg(Color::Yellow),
            ));
        }
        lines.push(Line::from(spans));
        for name in checks.failed.iter().take(5) {
            lines.push(Line::from(Span::styled(
                format!("  \u{2717} {name}"),
                Style::default().fg(Color::Red),
            )));
        }
    }
    if !pr.reviews.is_empty() {
        let mut spans = vec![Span::styled("reviews ", LABEL)];
        for (i, review) in pr.reviews.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(", ", LABEL));
            }
            let (word, style) = match review.state.as_str() {
                "APPROVED" => ("approved", Style::default().fg(Color::Green)),
                "CHANGES_REQUESTED" => ("wants changes", Style::default().fg(Color::Red)),
                "COMMENTED" => ("commented", LABEL),
                "DISMISSED" => ("dismissed", LABEL),
                _ => ("pending", LABEL),
            };
            spans.push(Span::styled(
                review.author.clone(),
                Style::default().fg(palette::author(&review.author)),
            ));
            spans.push(Span::styled(format!(" {word}"), style));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    if pr.body.trim().is_empty() {
        lines.push(Line::from(Span::styled("No description.", LABEL)));
    } else {
        lines.extend(crate::panel::markdown::render(&pr.body, "", width));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((state.github.detail_scroll, 0)),
        Rect {
            x: area.x + 1,
            width,
            ..area
        },
    );
}

/// The pull request tab's keys, without the ones the selected pull request
/// cannot take: a merged or closed one is past reviewing and merging, and a
/// merged one cannot be reopened either.
pub(super) fn pr_hints(state: &AppState) -> Vec<(&'static str, &'static str)> {
    let pairs = keys::footer_pairs("GitHub");
    let Some(pr) = selected_pr(state).filter(|pr| !pr.is_open()) else {
        return pairs;
    };
    let dropped: &[&str] = match pr.state {
        PrState::Merged => &["a", "x", "m", "D", "X"],
        _ => &["a", "x", "m", "D"],
    };
    pairs
        .into_iter()
        .filter(|(key, _)| !dropped.contains(key))
        .map(|(key, label)| match (key, pr.state) {
            ("X", PrState::Closed) => (key, "reopen"),
            _ => (key, label),
        })
        .collect()
}
