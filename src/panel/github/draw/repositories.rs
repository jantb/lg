//! The repository list and the selected repository's details.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::panel::github::lookup::{clone_destination, clone_request, visible_repos};
use crate::state::AppState;
use crate::ui::palette;

use super::areas::repo_list_areas;
use super::{LABEL, draw_placeholder, truncate};

pub(super) fn draw_repo_list(state: &AppState, area: Rect, frame: &mut Frame) {
    let gh = &state.github;
    let [search, list_area] = repo_list_areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Search ", LABEL),
            Span::styled(gh.query.to_string(), Style::default().fg(Color::White)),
        ])),
        search,
    );
    frame.set_cursor_position(Position::new(
        search.x + 8 + gh.query.before_cursor().chars().count() as u16,
        search.y,
    ));

    let visible = visible_repos(gh);
    if visible.is_empty() {
        let (text, error) = match (&gh.repos_error, gh.repos_loading.is_some()) {
            (Some(err), _) => (err.clone(), true),
            (None, true) => ("reading repositories\u{2026}".to_string(), false),
            (None, false) if gh.query.contains('/') => (
                format!("Enter clones {} from GitHub", gh.query.trim()),
                false,
            ),
            (None, false) if gh.query.trim().is_empty() => (
                "no repositories here you can see \u{2014} \u{2190}\u{2192} lists another owner's"
                    .to_string(),
                false,
            ),
            (None, false) => (
                "nothing matches \u{2014} type owner/name to clone any repository".to_string(),
                false,
            ),
        };
        draw_placeholder(&text, error, list_area, frame);
        return;
    }
    let width = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = visible
        .iter()
        .map(|repo| {
            let mut tags = Vec::new();
            if repo.private {
                tags.push("private");
            }
            if repo.fork {
                tags.push("fork");
            }
            if repo.archived {
                tags.push("archived");
            }
            let tags = if tags.is_empty() {
                String::new()
            } else {
                format!(" {}", tags.join(" "))
            };
            let room = width.saturating_sub(tags.chars().count() + 1);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {:<room$}", truncate(&repo.name, room)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(tags, LABEL),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(palette::selection())
        .highlight_symbol("\u{203a}");
    let rows = list_area.height as usize;
    let offset = gh.repo_idx.saturating_sub(rows.saturating_sub(1));
    let mut list_state = crate::panel::scroll::list_state(Some(gh.repo_idx), offset);
    frame.render_stateful_widget(list, list_area, &mut list_state);
}

pub(super) fn draw_repo_detail(state: &AppState, area: Rect, frame: &mut Frame) {
    let Some(repo) = clone_request(state) else {
        return;
    };
    let description = visible_repos(&state.github)
        .get(state.github.repo_idx)
        .map(|listed| listed.description.clone())
        .unwrap_or_default();
    let mut lines = vec![
        Line::from(Span::styled(
            repo.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    if !description.is_empty() {
        lines.push(Line::from(description));
        lines.push(Line::from(""));
    }
    match clone_destination(state) {
        Some(dir) if dir.exists() => {
            lines.push(Line::from(Span::styled(
                "Already cloned at",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(dir.display().to_string()));
            lines.push(Line::from(Span::styled("Enter opens it", LABEL)));
        }
        Some(dir) => {
            lines.push(Line::from(Span::styled("Enter clones it into", LABEL)));
            lines.push(Line::from(Span::styled(
                dir.display().to_string(),
                Style::default().fg(Color::Yellow),
            )));
            lines.push(Line::from(Span::styled("and switches lg to it", LABEL)));
        }
        None => lines.push(Line::from(Span::styled(
            "no folder to clone into",
            Style::default().fg(Color::Red),
        ))),
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(1),
            ..area
        },
    );
}
