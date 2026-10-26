//! The inbox tab: what is waiting, beside what Enter and d would do with the
//! selected one.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use super::{LABEL, draw_placeholder, truncate};
use crate::github::{InboxItem, Reason, Subject};
use crate::state::AppState;
use crate::ui::palette;

const REASON_WIDTH: usize = 18;

fn reason_style(reason: &Reason) -> Style {
    if reason.asks_for_action() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(palette::TEXT_IDLE)
    }
}

pub(super) fn draw_list(state: &AppState, area: Rect, frame: &mut Frame) {
    let inbox = &state.github.inbox;
    if inbox.items().is_empty() {
        let (text, error) = match (&inbox.error, inbox.was_read()) {
            (Some(err), _) => (err.clone(), true),
            (None, false) => ("reading GitHub\u{2026}".to_string(), false),
            (None, true) if inbox.read.checkouts.is_empty() => (
                "no checkout in this workspace has a GitHub remote".to_string(),
                false,
            ),
            (None, true) => (
                "nothing on GitHub is waiting on you in this workspace".to_string(),
                false,
            ),
        };
        draw_placeholder(&text, error, area, frame);
        return;
    }
    let width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = inbox
        .items()
        .iter()
        .map(|item| {
            let place = item.place();
            let room = width.saturating_sub(REASON_WIDTH + place.chars().count() + 3);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(
                        " {:<REASON_WIDTH$}",
                        truncate(item.reason.label(), REASON_WIDTH)
                    ),
                    reason_style(&item.reason),
                ),
                Span::styled(format!(" {place} "), Style::default().fg(Color::Cyan)),
                Span::styled(
                    truncate(&item.title, room),
                    Style::default().fg(Color::White),
                ),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(palette::selection())
        .highlight_symbol("\u{203a}");
    let rows = area.height as usize;
    let offset = inbox.idx.saturating_sub(rows.saturating_sub(1));
    let mut list_state = crate::panel::scroll::list_state(Some(inbox.idx), offset);
    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(super) fn draw_detail(state: &AppState, area: Rect, frame: &mut Frame) {
    let inbox = &state.github.inbox;
    let Some(item) = inbox.selected() else {
        return;
    };
    let mut lines = vec![
        Line::from(Span::styled(
            item.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(item.place(), Style::default().fg(Color::Cyan)),
            Span::styled(format!("  {}", kind(item)), LABEL),
        ]),
        Line::from(Span::styled(
            item.reason.label().to_string(),
            reason_style(&item.reason),
        )),
        Line::from(""),
    ];
    match inbox.review_checkout(item) {
        Some(dir) => {
            lines.push(Line::from(Span::styled(
                "Enter opens a guided review in",
                LABEL,
            )));
            lines.push(Line::from(Span::styled(
                dir.display().to_string(),
                Style::default().fg(Color::Green),
            )));
        }
        None => lines.push(Line::from(Span::styled(
            "Enter opens it in the browser",
            LABEL,
        ))),
    }
    let done = match item.thread {
        Some(_) => "d marks it done here and on GitHub",
        None => "d hides it here; GitHub keeps asking until the review is given",
    };
    lines.push(Line::from(Span::styled(done, LABEL)));
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(1),
            ..area
        },
    );
}

fn kind(item: &InboxItem) -> String {
    match &item.subject {
        Subject::PullRequest(_) => "pull request".to_string(),
        Subject::Issue(_) => "issue".to_string(),
        Subject::Other(kind) => kind.to_lowercase(),
    }
}
