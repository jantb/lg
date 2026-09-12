//! Searchable actions, using the same key dispatcher as ordinary shortcuts.
use crate::{
    state::{AppState, Modal},
    ui,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Paragraph, Wrap},
};
#[derive(Debug, Default)]
pub struct Commands {
    pub query: String,
    pub selected: usize,
}
/// What an entry does when run: press a key for it, or open Settings on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    Key(KeyCode),
    Settings(usize),
}
impl Run {
    fn hint(self) -> String {
        match self {
            Run::Key(KeyCode::Char(c)) => c.to_string(),
            Run::Key(code) => format!("{code:?}"),
            Run::Settings(_) => "Settings".into(),
        }
    }
}
const ACTIONS: &[(&str, Run, bool)] = &[
    (
        "Settings — all configuration",
        Run::Key(KeyCode::Char(',')),
        false,
    ),
    ("Identity — Git author", Run::Key(KeyCode::Char('a')), true),
    (
        "Writing and model settings",
        Run::Key(KeyCode::Char('L')),
        false,
    ),
    (
        "Environments — assign branches or promote",
        Run::Key(KeyCode::Char('E')),
        true,
    ),
    ("Commit staged changes", Run::Key(KeyCode::Char('c')), true),
    ("Fetch remote updates", Run::Key(KeyCode::Char('f')), true),
    ("Push current branch", Run::Key(KeyCode::Char('P')), true),
    ("Pull current branch", Run::Key(KeyCode::Char('p')), true),
    (
        "Switch workspace / Git view",
        Run::Key(KeyCode::Char('w')),
        false,
    ),
    ("Help — all shortcuts", Run::Key(KeyCode::Char('?')), false),
    ("Activity history and setup", Run::Settings(7), false),
];
fn actions(state: &AppState) -> Vec<&'static (&'static str, Run, bool)> {
    ACTIONS
        .iter()
        .filter(|(name, _, _)| {
            name.to_lowercase()
                .contains(&state.commands.query.to_lowercase())
        })
        .collect()
}
pub fn open(state: &mut AppState) {
    state.commands = Commands::default();
    state.modal = Modal::Commands;
}
pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = ui::centered(area, 92.min(area.width), 22.min(area.height));
    let inner = ui::modal_frame(frame, modal, "Actions");
    let mut text = format!("Search: {}\n\n", state.commands.query);
    for (i, (name, run, needs_repo)) in actions(state).iter().enumerate() {
        let reason = if *needs_repo && state.repo_root.is_none() {
            " — unavailable: select a Git repository"
        } else {
            ""
        };
        text.push_str(&format!(
            "{} {} [{}]{reason}\n",
            if i == state.commands.selected {
                "›"
            } else {
                " "
            },
            name,
            run.hint()
        ));
    }
    text.push_str("\n↑/↓ select · Enter run · Esc close");
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<KeyEvent> {
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Char(c) => {
            state.commands.query.push(c);
            state.commands.selected = 0;
        }
        KeyCode::Backspace => {
            state.commands.query.pop();
            state.commands.selected = 0;
        }
        KeyCode::Down => {
            state.commands.selected =
                (state.commands.selected + 1).min(actions(state).len().saturating_sub(1))
        }
        KeyCode::Up => state.commands.selected = state.commands.selected.saturating_sub(1),
        KeyCode::Enter => {
            if let Some((_, run, needs_repo)) = actions(state).get(state.commands.selected)
                && (!needs_repo || state.repo_root.is_some())
            {
                state.modal = Modal::None;
                match *run {
                    Run::Key(key) => return Some(KeyEvent::new(key, KeyModifiers::NONE)),
                    Run::Settings(category) => super::settings::open(state, category),
                }
            }
        }
        _ => {}
    }
    None
}
