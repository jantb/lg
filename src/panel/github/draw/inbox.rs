//! The inbox tab: what waits on the user, then the updates that only tell
//! them something happened, beside what the keys would do with the selected
//! one.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use chrono::Utc;

use super::{LABEL, draw_placeholder, truncate};
use crate::github::{InboxItem, Reason, Section, Subject};
use crate::state::AppState;
use crate::ui::palette;

const REASON_WIDTH: usize = 8;
const AGE_WIDTH: usize = 3;

/// A colour per reason, so a mention or an assignment stands out from the
/// run of review requests, and the updates stay quiet.
fn reason_style(reason: &Reason) -> Style {
    let color = match reason {
        Reason::ReviewRequested => Color::Yellow,
        Reason::Mentioned | Reason::TeamMentioned => Color::Magenta,
        Reason::TeamReviewRequested => Color::LightYellow,
        Reason::Assigned => Color::Cyan,
        Reason::ApprovalRequested => Color::LightRed,
        Reason::SecurityAlert => Color::Red,
        Reason::Authored => Color::Green,
        Reason::Commented => Color::Blue,
        Reason::StateChanged | Reason::Ci | Reason::Other(_) => palette::TEXT_IDLE,
    };
    Style::default().fg(color)
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
    // Each section under a heading of its own, so the few things asked of
    // the user by name are not lost among their teams' and the updates.
    let now = Utc::now();
    let width = area.width.saturating_sub(2) as usize;
    let row_of = |item: &InboxItem| {
        let age = item.age(now).unwrap_or_default();
        let place = item.short_place();
        let (name, number) = match place.split_once('#') {
            Some((name, number)) => (name.to_string(), format!("#{number}")),
            None => (place.clone(), String::new()),
        };
        let room = width.saturating_sub(AGE_WIDTH + REASON_WIDTH + place.chars().count() + 5);
        let title = if item.reason.section() != Section::Updates {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(palette::TEXT_IDLE)
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {age:>AGE_WIDTH$} "), LABEL),
            Span::styled(
                format!(
                    "{:<REASON_WIDTH$} ",
                    truncate(item.reason.short(), REASON_WIDTH)
                ),
                reason_style(&item.reason),
            ),
            Span::styled(name, Style::default().fg(Color::Cyan)),
            Span::styled(number, LABEL),
            Span::raw(" "),
            Span::styled(truncate(&item.title, room), title),
        ]))
    };
    let rows: Vec<ListItem> = layout(state)
        .into_iter()
        .map(|row| match row {
            Row::Item(idx) => row_of(&inbox.items()[idx]),
            Row::Heading(section) => heading(section, inbox.section_len(section)),
            Row::Gap => ListItem::new(""),
        })
        .collect();
    let selected = selected_row(state);
    let list = List::new(rows)
        .highlight_style(palette::selection())
        .highlight_symbol("\u{203a}");
    let visible = area.height as usize;
    let offset = selected.saturating_sub(visible.saturating_sub(1));
    let mut list_state = crate::panel::scroll::list_state(Some(selected), offset);
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn heading(section: Section, count: usize) -> ListItem<'static> {
    let (hint, color) = match section {
        Section::You => ("", Color::Yellow),
        Section::Teams => ("", Color::LightYellow),
        Section::Updates => ("  (D clears them all)", palette::TEXT_IDLE),
    };
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{} \u{b7} {count}", section.title()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(hint, LABEL),
    ]))
}

/// One line of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    Heading(Section),
    Item(usize),
    /// The blank line between two sections.
    Gap,
}

/// The list line by line: each section that has anything, under its heading,
/// a blank line between them. The items are already in section order.
pub(super) fn layout(state: &AppState) -> Vec<Row> {
    let items = state.github.inbox.items();
    let mut rows = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let section = item.reason.section();
        if idx == 0 || items[idx - 1].reason.section() != section {
            if idx > 0 {
                rows.push(Row::Gap);
            }
            rows.push(Row::Heading(section));
        }
        rows.push(Row::Item(idx));
    }
    rows
}

/// The row the selected item is drawn on.
pub(super) fn selected_row(state: &AppState) -> usize {
    let idx = state.github.inbox.idx;
    layout(state)
        .iter()
        .position(|row| *row == Row::Item(idx))
        .unwrap_or(0)
}

/// The item drawn on `row` of the list, if it is an item rather than a
/// heading or the gap between the sections.
pub(super) fn item_at(state: &AppState, row: usize) -> Option<usize> {
    match layout(state).get(row)? {
        Row::Item(idx) => Some(*idx),
        _ => None,
    }
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
        Line::from(vec![
            Span::styled(item.reason.label().to_string(), reason_style(&item.reason)),
            Span::styled(
                item.age(Utc::now())
                    .map(|age| format!("  \u{b7} {age} ago"))
                    .unwrap_or_default(),
                LABEL,
            ),
        ]),
        Line::from(Span::styled(explain(&item.reason), LABEL)),
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
    lines.push(Line::from(Span::styled("o opens it in the browser", LABEL)));
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

/// Why GitHub put it here, in words.
fn explain(reason: &Reason) -> &'static str {
    match reason {
        Reason::ReviewRequested => "your review is asked for, by name",
        Reason::TeamReviewRequested => "a review is asked of a team you are on",
        Reason::TeamMentioned => "a team you are on was mentioned",
        Reason::Mentioned => "you were mentioned",
        Reason::Assigned => "it is assigned to you",
        Reason::ApprovalRequested => "a deployment waits on your approval",
        Reason::SecurityAlert => "GitHub found a vulnerability",
        Reason::Authored => "something happened on a thread you opened",
        Reason::Commented => "something happened on a thread you commented on",
        Reason::StateChanged => "it was closed, merged or reopened",
        Reason::Ci => "a workflow run you started finished",
        Reason::Other(_) => "you are subscribed to it",
    }
}

fn kind(item: &InboxItem) -> String {
    match &item.subject {
        Subject::PullRequest(_) => "pull request".to_string(),
        Subject::Issue(_) => "issue".to_string(),
        Subject::Other(kind) => kind.to_lowercase(),
    }
}
