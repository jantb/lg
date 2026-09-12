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
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::{
    preferences::Agent,
    session::SessionKind,
    state::{AppState, Modal},
    ui,
};

use super::scroll;

/// Where the highlighted agent runs, what confines it, and the keys.
const DETAILS_HEIGHT: u16 = 5;
const MODAL_WIDTH: u16 = 92;
const NAME_WIDTH: usize = 8;
const CONFINEMENT_WIDTH: usize = 34;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let rows = state.agent_profiles.len().max(SessionKind::AGENTS.len()) as u16;
    // Rows, the frame, the divider, and the details underneath.
    let height = rows + 3 + DETAILS_HEIGHT;
    let modal = ui::centered(area, MODAL_WIDTH.min(area.width), height.min(area.height));
    let inner = ui::modal_frame(frame, modal, &title(state));
    let (chunks, dividers) = ui::modal_rows(
        frame,
        inner,
        &[Constraint::Min(3), Constraint::Length(DETAILS_HEIGHT)],
    );

    let items: Vec<ListItem> = if state.agent_profiles.is_empty() {
        SessionKind::AGENTS
            .iter()
            .map(|kind| ListItem::new(agent_line(*kind, state.agent_pick_sandboxed)))
            .collect()
    } else {
        state
            .agent_profiles
            .iter()
            .map(|profile| ListItem::new(profile_line(profile)))
            .collect()
    };
    let rows = items.len();
    let list = List::new(items)
        .highlight_style(crate::ui::palette::selection())
        .highlight_symbol("\u{203a} ");
    let mut list_state = scroll::list_state(Some(state.agent_pick_idx.min(rows - 1)), 0);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    frame.render_widget(Paragraph::new(details(state)), chunks[1]);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
}

/// The frame's title says where the session will land, because that is the one
/// thing the rows cannot.
fn title(state: &AppState) -> String {
    match super::environments::selected_checkout_label(state) {
        Some(label) => format!("Start agent in {label}"),
        None => "Start agent".to_string(),
    }
}

fn dim(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(Color::DarkGray))
}

fn key(text: &'static str) -> Span<'static> {
    Span::styled(text, Style::default().fg(Color::LightCyan))
}

/// One configured agent: its name, how it will be confined, and what lg can do
/// with it — or that it is not installed, which is the one thing worth saying
/// before Enter fails.
fn profile_line(profile: &Agent) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!("{:<NAME_WIDTH$}", profile.name),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "{:<CONFINEMENT_WIDTH$}",
            crate::agents::confinement_label(&profile.confinement)
        )),
    ];
    if crate::agents::resolve(&profile.executable).is_some() {
        spans.push(dim(crate::agents::capabilities(profile)));
    } else {
        spans.push(Span::styled(
            format!("not installed: {} not found", profile.executable),
            Style::default().fg(Color::Red),
        ));
    }
    Line::from(spans)
}

/// The built-in agents, when no profiles are configured yet.
fn agent_line(kind: SessionKind, sandboxed: bool) -> Line<'static> {
    let confinement = if sandboxed {
        "Terrarium sandbox"
    } else {
        "no sandbox"
    };
    Line::from(vec![
        Span::styled(
            format!("{:<NAME_WIDTH$}", kind.label()),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("{confinement:<CONFINEMENT_WIDTH$}")),
        dim(match kind {
            SessionKind::Claude => "prompt, live activity, resume",
            _ => "prompt only; activity and resume not tracked",
        }),
    ])
}

/// The lines under the list: where the highlighted agent runs, its command,
/// and the keys. The command is here rather than in the row so the rows stay
/// readable — nobody picks an agent by its install path.
fn details(state: &AppState) -> Vec<Line<'static>> {
    let checkout = super::environments::selected_checkout(state)
        .map(|(path, _)| path)
        .unwrap_or_default();
    let mut lines = vec![Line::from(vec![dim("Checkout  "), Span::raw(checkout)])];
    let mut command = match state.agent_profiles.get(state.agent_pick_idx) {
        Some(profile) => match crate::agents::resolve(&profile.executable) {
            Some(path) => {
                let mut parts = vec![path.display().to_string()];
                parts.extend(profile.args.iter().cloned());
                if !profile.model.is_empty() {
                    parts.push(format!("--model {}", profile.model));
                }
                parts.join(" ")
            }
            None => format!("{} is not on PATH", profile.executable),
        },
        None => state.picked_agent().label().to_string(),
    };
    if state.preferred_agent
        == state
            .agent_profiles
            .get(state.agent_pick_idx)
            .map(crate::agents::kind)
            .unwrap_or_else(|| state.picked_agent())
    {
        command.push_str("   (conflicts are handed to this one)");
    }
    lines.push(Line::from(vec![dim("Command   "), Span::raw(command)]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        key("Enter"),
        Span::raw(" start  "),
        key("Space"),
        Span::raw(" sandbox on/off  "),
        key(","),
        Span::raw(" configure agents  "),
        key("n"),
        Span::raw(" new worktree first  "),
        key("Esc"),
        Span::raw(" cancel"),
    ]));
    let mut direct: Vec<Span<'static>> = vec![dim("Or press a letter: ")];
    for kind in SessionKind::AGENTS {
        direct.push(Span::styled(
            kind.pick_key().to_string(),
            Style::default().fg(Color::LightGreen),
        ));
        direct.push(dim(format!(" {}  ", kind.label())));
    }
    lines.push(Line::from(direct));
    lines
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let last = if state.agent_profiles.is_empty() {
        SessionKind::AGENTS.len()
    } else {
        state.agent_profiles.len()
    }
    .saturating_sub(1);
    state.agent_pick_idx = state.agent_pick_idx.min(last);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.agent_pick_idx = (state.agent_pick_idx + 1).min(last);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.agent_pick_idx = state.agent_pick_idx.saturating_sub(1);
        }
        KeyCode::Enter if !state.agent_profiles.is_empty() => {
            if let Some(profile) = state.agent_profiles.get(state.agent_pick_idx).cloned() {
                if crate::agents::resolve(&profile.executable).is_none() {
                    state.set_status(
                        format!(
                            "{} is not installed; set its executable with , (Settings → Agents)",
                            profile.name
                        ),
                        true,
                    );
                } else if let Some((path, label)) = super::environments::selected_checkout(state) {
                    state.preferred_agent = crate::agents::kind(&profile);
                    state.pending_action = Some(crate::state::PendingAction::StartAgent {
                        path,
                        label,
                        profile,
                    });
                    state.modal = Modal::None;
                }
            }
        }
        KeyCode::Enter => start(state, state.picked_agent()),
        KeyCode::Char(',') => super::settings::open(state, 3),
        KeyCode::Char('n') => super::environments::open_new_worktree_form(state),
        KeyCode::Char(' ') => toggle_sandbox(state),
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

/// One switch for the whole picker: every agent it offers is either confined
/// by Terrarium or left to its own devices, so a letter pressed next starts
/// the agent the way the title says.
fn toggle_sandbox(state: &mut AppState) {
    state.agent_pick_sandboxed = !state.agent_pick_sandboxed;
    let sandboxed = state.agent_pick_sandboxed;
    for profile in &mut state.agent_profiles {
        profile.confinement = if sandboxed {
            "terrarium"
        } else if ["claude", "codex"].contains(&profile.adapter.as_str()) {
            "agent"
        } else {
            "direct"
        }
        .into();
    }
}

/// Start `kind` in the selected checkout, and remember it as the one to reach
/// for next time — including when a conflict needs an agent.
fn start(state: &mut AppState, kind: SessionKind) {
    if let Some(profile) = state
        .agent_profiles
        .iter()
        .find(|p| crate::agents::kind(p) == kind)
        .cloned()
    {
        if let Some((path, label)) = super::environments::selected_checkout(state) {
            state.pending_action = Some(crate::state::PendingAction::StartAgent {
                path,
                label,
                profile,
            });
            state.preferred_agent = kind;
            state.modal = Modal::None;
        }
        return;
    }
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
        state.agent_profiles.clear();
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

        assert_eq!(
            state.agent_profiles[state.agent_pick_idx].name, "claude",
            "the saved default wins when opening a configured picker"
        );
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

    /// Space flips the sandbox for whatever letter or Enter comes next, so the
    /// picker opened with `s` can still start an unconfined agent.
    #[test]
    fn space_turns_the_sandbox_off_for_the_agent_started_next() {
        let mut state = picking(true);

        handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
        handle_key(&mut state, key(KeyCode::Char('c'))).unwrap();

        assert!(matches!(
            state.pending_action,
            Some(crate::state::PendingAction::StartSession {
                sandboxed: false,
                ..
            })
        ));
    }

    /// The row says an agent is missing before Enter has to fail on it.
    #[test]
    fn a_missing_executable_is_named_on_its_row() {
        let profile = Agent {
            name: "codex".into(),
            adapter: "codex".into(),
            executable: "/nowhere/codex-not-here".into(),
            ..Agent::default()
        };

        let text: String = profile_line(&profile)
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();

        assert!(text.contains("not installed"), "{text}");
        assert!(text.contains("codex-not-here"), "{text}");
    }
}
