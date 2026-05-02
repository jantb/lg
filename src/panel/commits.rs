use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
};

use crate::{
    state::{AppState, Pane},
    ui,
};

pub fn render(state: &AppState, area: Rect, frame: &mut Frame, focused: bool) {
    let count = if state.commits.is_empty() {
        None
    } else {
        Some((state.commits_idx + 1, state.commits.len()))
    };
    let title = state
        .commits_ref
        .as_deref()
        .map(|branch| format!("Commits: {branch}"))
        .unwrap_or_else(|| "Commits".to_string());
    let block = ui::framed_with_activity(
        4,
        &title,
        focused,
        count,
        state.animation_tick,
        state.activity_label().is_some(),
    );

    let items: Vec<ListItem> = state
        .commits
        .iter()
        .map(|c| {
            let subject_style = if state.unpushed_shas.contains(&c.sha) {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            let author_style = Style::default()
                .fg(author_color(&c.author))
                .add_modifier(Modifier::BOLD);
            let merge_style = if c.parent_count > 1 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let merge_marker = if c.parent_count > 1 {
                "\u{25c6} "
            } else {
                "  "
            };
            let line = Line::from(vec![
                Span::styled(merge_marker, merge_style),
                Span::styled(format!("{} ", c.sha), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<12} ", c.author_short), author_style),
                Span::styled(c.subject.clone(), subject_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    let mut list_state = ListState::default();
    if focused && !state.commits.is_empty() {
        list_state.select(Some(state.commits_idx));
    }

    frame.render_stateful_widget(list, area, &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !state.commits.is_empty() && state.commits_idx + 1 < state.commits.len() {
                state.commits_idx += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.commits_idx > 0 {
                state.commits_idx -= 1;
            }
        }
        KeyCode::Enter => {
            state.focus = Pane::Main;
        }
        _ => {}
    }
    Ok(())
}

fn author_color(author: &str) -> Color {
    const COLORS: &[Color] = &[
        Color::Cyan,
        Color::Yellow,
        Color::Green,
        Color::Magenta,
        Color::Blue,
        Color::LightCyan,
        Color::LightYellow,
        Color::LightGreen,
        Color::LightMagenta,
        Color::LightBlue,
    ];
    let hash = author.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    COLORS[hash as usize % COLORS.len()]
}
