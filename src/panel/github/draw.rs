//! Drawing the GitHub modal: a tab bar, the list beside the selected entry's
//! details, and a bottom line that is whatever the modal is asking for.

use ratatui::Frame;
use ratatui::layout::{Constraint, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::panel::keys;
use crate::state::{AppState, SPINNER_FRAMES};
use crate::ui::{self, palette};

use super::lookup::selected_pr;
use super::{Compose, Mode, Tab};
use areas::{columns, modal_area, too_small};
use forms::{draw_create, draw_merge};
use header::draw_tabs;
use pull_requests::{draw_pr_detail, draw_pr_list, pr_hints};
use repositories::{draw_repo_detail, draw_repo_list};

mod areas;
mod forms;
mod header;
mod inbox;
mod pull_requests;
mod repositories;

pub(super) use areas::{HeaderClick, header_click, list_click};

pub(super) const LABEL: Style = Style::new().fg(Color::DarkGray);

/// The header, the tab's panes, and the bottom line.
const ROWS: [Constraint; 3] = [
    Constraint::Length(1),
    Constraint::Min(3),
    Constraint::Length(1),
];

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = modal_area(area);
    let title = match &state.github.repo {
        Some(repo) => format!("GitHub \u{b7} {}", repo.name),
        None => "GitHub".to_string(),
    };
    let inner = ui::modal_frame(frame, modal, &title);
    if too_small(inner) {
        frame.render_widget(Paragraph::new("Terminal too small for GitHub"), inner);
        return;
    }
    let (rows, mut dividers) = ui::modal_rows(frame, inner, &ROWS);
    draw_tabs(state, rows[0], frame);
    match (&state.github.mode, state.github.tab) {
        (Mode::Create(form), _) => draw_create(form, rows[1], frame),
        (_, Tab::Inbox) => {
            let (panes, gaps) = ui::modal_columns(frame, rows[1], columns(Tab::Inbox));
            dividers.extend(gaps);
            inbox::draw_list(state, panes[0], frame);
            inbox::draw_detail(state, panes[1], frame);
        }
        (_, Tab::PullRequests) => {
            let (panes, gaps) = ui::modal_columns(frame, rows[1], columns(Tab::PullRequests));
            dividers.extend(gaps);
            draw_pr_list(state, panes[0], frame);
            match &state.github.mode {
                Mode::Merge(form) => draw_merge(state, form, panes[1], frame),
                _ => draw_pr_detail(state, panes[1], frame),
            }
        }
        (_, Tab::Repositories) => {
            let (panes, gaps) = ui::modal_columns(frame, rows[1], columns(Tab::Repositories));
            dividers.extend(gaps);
            draw_repo_list(state, panes[0], frame);
            draw_repo_detail(state, panes[1], frame);
        }
    }
    draw_bottom(state, rows[2], frame);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
}

pub(super) fn spinner(state: &AppState) -> &'static str {
    SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()]
}

/// A message filling a pane in place of a list: loading, empty, or failed.
pub(super) fn draw_placeholder(text: &str, error: bool, area: Rect, frame: &mut Frame) {
    let style = if error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(palette::TEXT_IDLE)
    };
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(style)
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub(super) fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn draw_bottom(state: &AppState, area: Rect, frame: &mut Frame) {
    let hints = |section: &str| ui::key_hints(&keys::footer_pairs(section));
    let line = match (&state.github.mode, state.github.tab) {
        (Mode::Compose { kind, text }, _) => {
            let number = selected_pr(state).map(|pr| pr.number).unwrap_or(0);
            let prompt = format!("{} #{number}: ", kind.label());
            let keys = keys::footer_pairs("GitHub review text")
                .iter()
                .map(|(key, label)| format!("{key} {label}"))
                .collect::<Vec<_>>()
                .join("  ");
            let hint = match kind {
                Compose::Approve => format!("  (optional)  {keys}"),
                _ => format!("  {keys}"),
            };
            let x = area.x + (prompt.chars().count() + text.before_cursor().chars().count()) as u16;
            if x < area.x + area.width {
                frame.set_cursor_position(Position::new(x, area.y));
            }
            Line::from(vec![
                Span::styled(prompt, Style::default().fg(Color::Cyan)),
                Span::styled(text.to_string(), Style::default().fg(Color::White)),
                Span::styled(if text.is_empty() { hint } else { String::new() }, LABEL),
            ])
        }
        (Mode::ConfirmClose, _) => {
            let (number, title) = selected_pr(state)
                .map(|pr| (pr.number, pr.title.clone()))
                .unwrap_or_default();
            let mut spans = vec![Span::styled(
                format!("Close #{number} \u{201c}{title}\u{201d} without merging?  "),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )];
            spans.extend(hints("GitHub close").spans);
            Line::from(spans)
        }
        (Mode::Merge(_), _) => hints("GitHub merge"),
        (Mode::Create(_), _) => hints("GitHub new pull request"),
        (Mode::Browse, Tab::Inbox) => hints("GitHub inbox"),
        (Mode::Browse, Tab::PullRequests) => ui::key_hints(&pr_hints(state)),
        (Mode::Browse, Tab::Repositories) => hints("GitHub repositories"),
    };
    frame.render_widget(Paragraph::new(line), area);
}
