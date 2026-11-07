//! A pull request's conversations: how many are left to resolve over the list
//! of them, and the selected one in full beside it.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::github::{ReviewThread, ThreadCount};
use crate::panel::github::threads::ThreadsView;
use crate::panel::keys;
use crate::ui::{self, palette};

use super::{LABEL, draw_placeholder, truncate};

const OPEN: Style = Style::new().fg(Color::Yellow);
const DONE: Style = Style::new().fg(Color::Green);

/// `3 of 7 left to resolve`, or that none are, styled to say which.
pub(super) fn count_spans(count: ThreadCount) -> Vec<Span<'static>> {
    match count {
        ThreadCount { total: 0, .. } => vec![Span::styled("no conversations", LABEL)],
        ThreadCount {
            unresolved: 0,
            total,
        } => vec![Span::styled(format!("\u{2713} all {total} resolved"), DONE)],
        ThreadCount { unresolved, total } => vec![
            Span::styled(
                format!("{unresolved} of {total}"),
                OPEN.add_modifier(Modifier::BOLD),
            ),
            Span::styled(" left to resolve", OPEN),
        ],
    }
}

pub(super) fn draw_list(view: &ThreadsView, area: Rect, frame: &mut Frame, spinner: &str) {
    let [head, list] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(2),
        ratatui::layout::Constraint::Min(1),
    ])
    .areas(area);
    let mut spans = vec![Span::styled(
        format!("#{} ", view.number),
        Style::default().fg(Color::Green),
    )];
    if view.loaded {
        spans.extend(count_spans(view.count()));
    } else {
        spans.push(Span::styled(format!("{spinner} reading"), LABEL));
    }
    if !view.show_resolved && view.count().total > view.count().unresolved {
        spans.push(Span::styled("  h shows resolved", LABEL));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), head);

    let visible = view.visible();
    if visible.is_empty() {
        let (text, error) = match (&view.error, view.loaded) {
            (Some(err), _) => (err.clone(), true),
            (None, false) => ("reading conversations\u{2026}".to_string(), false),
            (None, true) if view.threads.is_empty() => (
                "nobody has commented on a line of this pull request".into(),
                false,
            ),
            (None, true) => (
                "every conversation is resolved \u{2014} h shows them, Esc goes back".into(),
                false,
            ),
        };
        draw_placeholder(&text, error, list, frame);
        return;
    }
    let width = list.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = visible
        .iter()
        .map(|&i| list_row(&view.threads[i], width))
        .collect();
    let rows = List::new(items)
        .highlight_style(palette::selection())
        .highlight_symbol("\u{203a}");
    let shown = list.height as usize;
    let offset = view.idx.saturating_sub(shown.saturating_sub(1));
    let mut state = crate::panel::scroll::list_state(Some(view.idx), offset);
    frame.render_stateful_widget(rows, list, &mut state);
}

fn list_row(thread: &ReviewThread, width: usize) -> ListItem<'static> {
    let (mark, style) = if thread.resolved {
        ("\u{2713} ", DONE)
    } else {
        ("\u{25cf} ", OPEN)
    };
    let author = thread.author().to_string();
    let replies = thread.comments.len().saturating_sub(1);
    let mut tail = vec![Span::styled(
        format!(" {author}"),
        Style::default().fg(palette::author(&author)),
    )];
    if replies > 0 {
        tail.push(Span::styled(format!(" +{replies}"), LABEL));
    }
    if thread.outdated {
        tail.push(Span::styled(" outdated", LABEL));
    }
    let used = 2 + tail
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>();
    let room = width.saturating_sub(used);
    let text_style = if thread.resolved {
        Style::default().fg(palette::TEXT_IDLE)
    } else {
        Style::default().fg(Color::White)
    };
    let mut spans = vec![
        Span::styled(mark, style),
        Span::styled(
            format!("{:<room$}", truncate_start(&thread.location(), room)),
            text_style,
        ),
    ];
    spans.extend(tail);
    ListItem::new(Line::from(spans))
}

/// The end of `text` that fits in `width`: for a path, the file name is the
/// part worth keeping.
fn truncate_start(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let tail: String = text.chars().skip(len - keep).collect();
    format!("\u{2026}{tail}")
}

/// The diff lines the conversation was left on: the last few of the hunk,
/// which end at the line commented on.
const HUNK_LINES: usize = 6;

pub(super) fn draw_conversation(view: &ThreadsView, area: Rect, frame: &mut Frame) {
    let Some(thread) = view.selected() else {
        return;
    };
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    let width = inner.width;
    let mut lines: Vec<Line> = Vec::new();
    let mut status = vec![Span::styled(
        thread.location(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    if thread.resolved {
        let by = thread
            .resolved_by
            .as_ref()
            .map(|who| format!(" by {who}"))
            .unwrap_or_default();
        status.push(Span::styled(format!("  resolved{by}"), DONE));
    } else {
        status.push(Span::styled("  open", OPEN));
    }
    if thread.outdated {
        status.push(Span::styled(" \u{b7} outdated", LABEL));
    }
    lines.push(Line::from(status));

    let hunk: Vec<&str> = thread.diff_hunk.lines().collect();
    if !hunk.is_empty() {
        lines.push(Line::from(""));
        for row in &hunk[hunk.len().saturating_sub(HUNK_LINES)..] {
            let style = match row.chars().next() {
                Some('+') => Style::default().fg(Color::Green),
                Some('-') => Style::default().fg(Color::Red),
                Some('@') => Style::default().fg(Color::Cyan),
                _ => Style::default().fg(palette::TEXT_IDLE),
            };
            lines.push(Line::from(Span::styled(
                truncate(row, width as usize),
                style,
            )));
        }
    }

    for comment in &thread.comments {
        lines.push(Line::from(""));
        let when = comment
            .created_at
            .split('T')
            .next()
            .filter(|day| !day.is_empty())
            .map(|day| format!("  {day}"))
            .unwrap_or_else(|| "  sending\u{2026}".into());
        lines.push(Line::from(vec![
            Span::styled(
                comment.author.clone(),
                Style::default()
                    .fg(palette::author(&comment.author))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(when, LABEL),
        ]));
        lines.extend(crate::panel::markdown::render(&comment.body, "  ", width));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((view.scroll, 0)),
        inner,
    );
}

/// Claude beside the conversations, with what it is on and how to reach it.
pub(super) fn draw_chat(
    state: &crate::state::AppState,
    view: &ThreadsView,
    id: crate::session::SessionId,
    area: Rect,
    frame: &mut Frame,
) {
    let (Some(session), Some(chat)) = (state.sessions.get(id), &state.github.threads_chat) else {
        return;
    };
    let on_another = view.selected().is_some_and(|t| t.id != chat.thread);
    let (title, hint) = if chat.focused {
        (
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            "  Ctrl-] back to the conversations",
        )
    } else if on_another {
        (
            Style::default().fg(Color::Gray),
            "  C hands it the selected one",
        )
    } else {
        (Style::default().fg(Color::Gray), "  C or click to type")
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("Claude \u{b7} {}", chat.location), title),
            Span::styled(hint, LABEL),
        ])),
        Rect { height: 1, ..area },
    );
    let screen = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    crate::term::render_screen(session.screen(), screen, frame.buffer_mut());
    if chat.focused
        && let Some((row, col)) = session.cursor_position()
        && row < screen.height
        && col < screen.width
    {
        frame.set_cursor_position((screen.x + col, screen.y + row));
    }
}

/// The bottom line: the keys, or the reply being written with its cursor.
pub(super) fn bottom_line(view: &ThreadsView, area: Rect, frame: &mut Frame) -> Line<'static> {
    let Some(text) = &view.reply else {
        return ui::key_hints(&keys::footer_pairs("GitHub conversations"));
    };
    let place = view
        .selected()
        .map(ReviewThread::location)
        .unwrap_or_default();
    let prompt = format!("Reply on {place}: ");
    let x = area.x + (prompt.chars().count() + text.before_cursor().chars().count()) as u16;
    if x < area.x + area.width {
        frame.set_cursor_position(Position::new(x, area.y));
    }
    let hint = if text.is_empty() {
        let keys = keys::footer_pairs("GitHub reply")
            .iter()
            .map(|(key, label)| format!("{key} {label}"))
            .collect::<Vec<_>>()
            .join("  ");
        format!("  {keys}")
    } else {
        String::new()
    };
    Line::from(vec![
        Span::styled(prompt, Style::default().fg(Color::Cyan)),
        Span::styled(text.to_string(), Style::default().fg(Color::White)),
        Span::styled(hint, LABEL),
    ])
}
