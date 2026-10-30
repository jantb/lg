//! The header: the tabs, and the filter or owner line under them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::github::PrFilter;
use crate::panel::github::Tab;
use crate::state::AppState;
use crate::ui::palette;

use super::areas::HeaderClick;
use super::{LABEL, spinner};

pub(super) fn draw_tabs(state: &AppState, area: Rect, frame: &mut Frame) {
    let spans: Vec<Span> = header_spans(state, area.width)
        .into_iter()
        .map(|(span, _)| span)
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The header's spans, each with what a click on it does.
pub(super) fn header_spans(
    state: &AppState,
    width: u16,
) -> Vec<(Span<'static>, Option<HeaderClick>)> {
    let tab = |label: &str, tab: Tab| {
        let style = if state.github.tab == tab {
            Style::default()
                .fg(Color::Black)
                .bg(palette::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette::TEXT_IDLE)
        };
        (
            Span::styled(format!(" {label} "), style),
            Some(HeaderClick::Tab(tab)),
        )
    };
    let plain = |span: Span<'static>| (span, None);
    let gh = &state.github;
    let inbox_label = match gh.inbox.counts() {
        (0, _) => "Inbox".to_string(),
        (count, 0) => format!("Inbox {count}"),
        (count, asking) => format!("Inbox {count} ({asking} for you)"),
    };
    let mut spans = vec![
        tab(&inbox_label, Tab::Inbox),
        plain(Span::raw(" ")),
        tab("Pull requests", Tab::PullRequests),
        plain(Span::raw(" ")),
        tab("Repositories", Tab::Repositories),
        plain(Span::raw("   ")),
    ];
    match gh.tab {
        Tab::Inbox => {
            spans.push(plain(Span::styled("across this workspace", LABEL)));
            if gh.inbox.loading.is_some() {
                spans.push(plain(Span::styled(
                    format!("   {} reading GitHub", spinner(state)),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
        Tab::PullRequests => {
            let drafts = if gh.hide_drafts {
                "drafts hidden (h shows)"
            } else {
                "h hides drafts"
            };
            let used: usize = spans.iter().map(|(span, _)| span.width()).sum();
            let room = (width as usize).saturating_sub(used + 3 + drafts.chars().count());
            spans.extend(filter_spans(gh.filter, room));
            spans.push(plain(Span::raw("   ")));
            spans.push((Span::styled(drafts, LABEL), Some(HeaderClick::Drafts)));
            if gh.prs_loading.is_some() {
                spans.push(plain(Span::styled(
                    format!("   {} reading pull requests", spinner(state)),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
        Tab::Repositories => {
            let used: usize = spans.iter().map(|(span, _)| span.width()).sum();
            spans.extend(
                owner_spans(state, (width as usize).saturating_sub(used))
                    .into_iter()
                    .map(plain),
            );
            if gh.repos_loading.is_some() {
                spans.push(plain(Span::styled(
                    format!("   {} reading repositories", spinner(state)),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
    }
    spans
}

/// Whose repositories are listed, among everyone whose could be: every owner
/// by name when they fit in `room`, otherwise the chosen one and where it
/// stands among them.
/// Every filter, the one shown picked out, each a click away; or, with no
/// room for them all, the one shown and a click that steps on from it.
fn filter_spans(chosen: PrFilter, room: usize) -> Vec<(Span<'static>, Option<HeaderClick>)> {
    let picked = Style::default().fg(Color::Cyan);
    let hint = "  (s)";
    let all: usize = PrFilter::ALL
        .iter()
        .map(|filter| filter.label().chars().count() + 2)
        .sum();
    if all + hint.chars().count() > room {
        let next = Some(HeaderClick::NextFilter);
        return vec![
            (Span::styled("showing ", LABEL), next),
            (Span::styled(chosen.label(), picked), next),
            (Span::styled("  (s changes)", LABEL), next),
        ];
    }
    let mut spans: Vec<_> = PrFilter::ALL
        .iter()
        .map(|&filter| {
            let style = if filter == chosen {
                picked.add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette::TEXT_IDLE)
            };
            (
                Span::styled(format!(" {} ", filter.label()), style),
                Some(HeaderClick::Filter(filter)),
            )
        })
        .collect();
    spans.push((Span::styled(hint, LABEL), Some(HeaderClick::NextFilter)));
    spans
}

fn owner_spans(state: &AppState, room: usize) -> Vec<Span<'static>> {
    let gh = &state.github;
    let name = |owner: &Option<String>| match (owner, &gh.owners) {
        (Some(org), _) => org.clone(),
        (None, Some(owners)) if !owners.login.is_empty() => owners.login.clone(),
        (None, _) => "you".to_string(),
    };
    let chosen = Style::default().fg(Color::Cyan);
    let choices = gh.owner_choices();
    let mut spans = vec![Span::styled("owner ", LABEL)];
    if choices.len() < 2 {
        spans.push(Span::styled(name(&gh.owner), chosen));
        return spans;
    }
    let hint = "  (\u{2190}\u{2192} changes)";
    let all: usize = choices
        .iter()
        .map(|owner| name(owner).chars().count() + 2)
        .sum();
    if 6 + all + hint.chars().count() <= room {
        for owner in &choices {
            let style = if *owner == gh.owner {
                chosen.add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette::TEXT_IDLE)
            };
            spans.push(Span::styled(format!(" {} ", name(owner)), style));
        }
    } else {
        let at = choices
            .iter()
            .position(|owner| *owner == gh.owner)
            .unwrap_or(0);
        spans.push(Span::styled(name(&gh.owner), chosen));
        spans.push(Span::styled(
            format!(" {}/{}", at + 1, choices.len()),
            LABEL,
        ));
    }
    spans.push(Span::styled(hint, LABEL));
    spans
}
