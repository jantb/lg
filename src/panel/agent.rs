//! Which coding agent to start in the selected checkout.
//!
//! A checkout holds one session of each kind, so this is a choice about what to
//! run rather than about replacing anything: claude, codex and pi can all be
//! open on the same worktree at once. The list is short and the letters are
//! fixed, so `s x` starts codex without ever reading it — the rows are there
//! for the first few times, and for saying which one `s` will hand a conflict
//! to.

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::{
    session::SessionKind,
    state::{AppState, Modal},
    ui,
};

use super::scroll;

/// The two control lines and their border.
const CONTROLS_HEIGHT: u16 = 4;
/// One row per agent, the list's border, and the controls under it.
const MODAL_HEIGHT: u16 = SessionKind::AGENTS.len() as u16 + 2 + CONTROLS_HEIGHT;
const MODAL_WIDTH: u16 = 48;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = ui::centered(
        area,
        MODAL_WIDTH.min(area.width),
        MODAL_HEIGHT.min(area.height),
    );
    frame.render_widget(Clear, modal);

    let items: Vec<ListItem> = SessionKind::AGENTS
        .iter()
        .map(|kind| ListItem::new(agent_line(*kind, *kind == state.preferred_agent)))
        .collect();
    let rows = items.len();
    let title = title(state);
    let list = List::new(items)
        .block(ui::bordered(&title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");
    let mut list_state = scroll::list_state(Some(state.agent_pick_idx.min(rows - 1)), 0);

    let chunks =
        Layout::vertical([Constraint::Min(3), Constraint::Length(CONTROLS_HEIGHT)]).split(modal);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let controls = vec![
        Line::from(vec![
            Span::styled("j/k", Style::default().fg(Color::LightCyan)),
            Span::raw(" select  "),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" start  "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel"),
        ]),
        Line::from(vec![
            Span::styled(
                SessionKind::AGENTS
                    .iter()
                    .map(|kind| kind.pick_key().to_string())
                    .collect::<Vec<_>>()
                    .join("/"),
                Style::default().fg(Color::LightGreen),
            ),
            Span::raw(" start that one outright"),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(controls).block(Block::default().borders(Borders::ALL)),
        chunks[1],
    );
}

/// The frame's title says where the session will land and whether it will be
/// confined, because `s` and `S` open the same box.
fn title(state: &AppState) -> String {
    let sandbox = if state.agent_pick_sandboxed {
        "sandboxed"
    } else {
        "unsandboxed"
    };
    match super::environments::selected_checkout_label(state) {
        Some(label) => format!("Start {sandbox} agent in {label}"),
        None => format!("Start {sandbox} agent"),
    }
}

fn agent_line(kind: SessionKind, preferred: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!("{} ", kind.pick_key()),
            Style::default().fg(Color::LightGreen),
        ),
        Span::styled(
            kind.label().to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if preferred {
        spans.push(Span::styled(
            "  \u{2190} conflicts go here",
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let last = SessionKind::AGENTS.len() - 1;
    state.agent_pick_idx = state.agent_pick_idx.min(last);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.agent_pick_idx = (state.agent_pick_idx + 1).min(last);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.agent_pick_idx = state.agent_pick_idx.saturating_sub(1);
        }
        KeyCode::Enter => start(state, state.picked_agent()),
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Char(pressed) => {
            let pressed = pressed.to_ascii_lowercase();
            if let Some(kind) = SessionKind::AGENTS
                .into_iter()
                .find(|kind| kind.pick_key() == pressed)
            {
                start(state, kind);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Start `kind` in the selected checkout, and remember it as the one to reach
/// for next time — including when a conflict needs an agent.
fn start(state: &mut AppState, kind: SessionKind) {
    state.preferred_agent = kind;
    state.modal = Modal::None;
    let sandboxed = state.agent_pick_sandboxed;
    super::environments::start_session_for_selection(state, kind, sandboxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: ratatui::crossterm::event::KeyEventState::NONE,
        }
    }

    fn picking(sandboxed: bool) -> AppState {
        let mut state = AppState::new();
        state.repo_root = Some("/tmp/checkout".into());
        state.open_agent_picker(sandboxed);
        state
    }

    #[test]
    fn an_agents_own_letter_starts_it_without_walking_the_list() {
        let mut state = picking(true);

        handle_key(&mut state, key(KeyCode::Char('x'))).unwrap();

        assert_eq!(state.preferred_agent, SessionKind::Codex);
        assert!(matches!(
            state.pending_action,
            Some(crate::state::PendingAction::StartSession {
                kind: SessionKind::Codex,
                sandboxed: true,
                ..
            })
        ));
        assert_eq!(state.modal, Modal::None, "the picker is done with");
    }

    #[test]
    fn the_picker_opens_on_the_agent_it_would_start_again() {
        let mut state = AppState::new();
        state.repo_root = Some("/tmp/checkout".into());
        state.preferred_agent = SessionKind::Pi;
        state.open_agent_picker(true);

        assert_eq!(state.picked_agent(), SessionKind::Pi);
    }

    #[test]
    fn which_key_opened_the_picker_decides_whether_the_session_is_confined() {
        let mut state = picking(false);

        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert!(matches!(
            state.pending_action,
            Some(crate::state::PendingAction::StartSession {
                sandboxed: false,
                ..
            })
        ));
    }

    #[test]
    fn walking_the_list_and_confirming_starts_what_is_highlighted() {
        let mut state = picking(true);

        handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
        handle_key(&mut state, key(KeyCode::Enter)).unwrap();

        assert_eq!(state.preferred_agent, SessionKind::Codex);
    }

    #[test]
    fn escaping_the_picker_starts_nothing_and_changes_nothing() {
        let mut state = picking(true);

        handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
        handle_key(&mut state, key(KeyCode::Esc)).unwrap();

        assert_eq!(state.modal, Modal::None);
        assert_eq!(state.preferred_agent, SessionKind::Claude);
        assert!(state.pending_action.is_none());
    }

    /// A letter that is nobody's is not a reason to start something.
    #[test]
    fn a_letter_no_agent_answers_to_does_nothing() {
        let mut state = picking(true);

        handle_key(&mut state, key(KeyCode::Char('z'))).unwrap();

        assert_eq!(state.modal, Modal::Agent);
        assert!(state.pending_action.is_none());
    }
}
