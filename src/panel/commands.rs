//! Searchable actions, run through the same key dispatcher as ordinary
//! shortcuts.
//!
//! The list is the key table: every binding the help documents is an entry,
//! found by what it does, and shown with its key. An entry for a pane's key
//! runs by focusing that pane and pressing the key there. One whose key only
//! means something inside a modal, or in a state lg is not in, cannot be run
//! from here, and says which key it is and where instead of pretending.
use crate::{
    panel::keys::{SECTIONS, Section},
    panel::text_input::TextInput,
    state::{AppState, MainKeys, Modal},
    ui,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

#[derive(Debug, Default)]
pub struct Commands {
    pub query: TextInput,
    pub selected: usize,
}

/// What an entry does when run: press keys for it, or open Settings on a page.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Run {
    Keys(Vec<KeyEvent>),
    Settings(super::settings::Category),
}

/// One line of the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// What it does, which is what the search reads first.
    name: String,
    /// The key that does it, as the help names it.
    key: String,
    /// Where the key works, for one that does not work everywhere.
    context: Option<&'static str>,
    /// How to run it from here, when it can be.
    run: Option<Run>,
    needs_repo: bool,
}

/// The entries that are not a key of their own, or that deserve a name the
/// key table's terse help does not give them.
const SPECIAL: &[(&str, &str, bool)] = &[
    ("Settings \u{2014} all configuration", ",", false),
    ("Identity \u{2014} Git author", "a", true),
    ("Writing and model settings", "L", false),
    (
        "Environments \u{2014} assign branches or promote",
        "E",
        true,
    ),
    (
        "GitHub \u{2014} pull requests, review, merge, clone",
        "H",
        false,
    ),
    (
        "Guided review \u{2014} walk the branch (or, on main, uncommitted changes) hunk by hunk",
        "V",
        true,
    ),
    ("Commit staged changes", "c", true),
    ("Fetch remote updates", "f", true),
    ("Push current branch", "P", true),
    ("Pull current branch", "p", true),
    ("Switch workspace / Git view", "w", false),
    ("Help \u{2014} all shortcuts", "?", false),
];

/// The key a pane is focused with, for the sections that belong to one.
fn focus_key(section: &Section, state: &AppState) -> Option<Option<char>> {
    // `None` inside: a key that works from anywhere, with nothing to focus.
    match section.title {
        "Global" => Some(None),
        "Repositories" => Some(Some('1')),
        "Files" | "Branches" | "Commits" if !state.git_panes_visible() => None,
        "Files" => Some(Some('2')),
        "Branches" => Some(Some('3')),
        "Commits" => Some(Some('4')),
        // The main pane listens for one of three sets of keys, and only the
        // set it is showing can be pressed there.
        "Diff pane" if state.main_keys() == MainKeys::Diff => Some(Some('0')),
        "Review mode" if state.main_keys() == MainKeys::Review => Some(Some('0')),
        _ => None,
    }
}

/// The key a binding's key column names first, as a key press. `None` for a
/// gesture or a description rather than a key.
fn parse_key(spec: &str) -> Option<KeyEvent> {
    let first = spec.split(" / ").next()?.split(", ").next()?.trim();
    let (modifiers, rest) = match first
        .strip_prefix("Ctrl-")
        .or_else(|| first.strip_prefix("Ctrl+"))
    {
        Some(rest) => (KeyModifiers::CONTROL, rest),
        None => (KeyModifiers::NONE, first),
    };
    let code = match rest {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "space" | "Space" => KeyCode::Char(' '),
        "PgDn" => KeyCode::PageDown,
        "PgUp" => KeyCode::PageUp,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            match c {
                '\u{2190}' => KeyCode::Left,
                '\u{2192}' => KeyCode::Right,
                '\u{2191}' => KeyCode::Up,
                '\u{2193}' => KeyCode::Down,
                c if modifiers == KeyModifiers::CONTROL => KeyCode::Char(c.to_ascii_lowercase()),
                c => KeyCode::Char(c),
            }
        }
    };
    Some(KeyEvent::new(code, modifiers))
}

fn special_entries() -> Vec<Entry> {
    let mut entries: Vec<Entry> = SPECIAL
        .iter()
        .map(|(name, key, needs_repo)| Entry {
            name: (*name).to_string(),
            key: (*key).to_string(),
            context: None,
            run: parse_key(key).map(|key| Run::Keys(vec![key])),
            needs_repo: *needs_repo,
        })
        .collect();
    entries.push(Entry {
        name: "Activity history and setup".to_string(),
        key: "Settings".to_string(),
        context: None,
        run: Some(Run::Settings(super::settings::Category::Activity)),
        needs_repo: false,
    });
    entries
}

/// Every entry, special ones first, then the key table in help order.
fn all_entries(state: &AppState) -> Vec<Entry> {
    let mut entries = special_entries();
    for section in SECTIONS {
        // The palette's own keys are the ones being pressed right now.
        if section.title == "Actions" {
            continue;
        }
        let focus = focus_key(section, state);
        for binding in section.bindings {
            // A global key the special entries already name.
            if section.title == "Global"
                && (binding.key == ":" || SPECIAL.iter().any(|(_, key, _)| *key == binding.key))
            {
                continue;
            }
            let run = match (focus, parse_key(binding.key)) {
                (Some(focus), Some(key)) => {
                    let mut keys: Vec<KeyEvent> = focus
                        .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
                        .into_iter()
                        .collect();
                    keys.push(key);
                    Some(Run::Keys(keys))
                }
                _ => None,
            };
            entries.push(Entry {
                name: binding.help.to_string(),
                key: binding.key.to_string(),
                context: (section.title != "Global").then_some(section.title),
                run,
                needs_repo: false,
            });
        }
    }
    entries
}

/// The entries the search leaves: every word of it in the name, the key or
/// the place, whatever the case.
fn actions(state: &AppState) -> Vec<Entry> {
    let query = state.commands.query.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    all_entries(state)
        .into_iter()
        .filter(|entry| {
            let haystack = format!(
                "{} {} {}",
                entry.name.to_lowercase(),
                entry.key.to_lowercase(),
                entry.context.unwrap_or_default().to_lowercase()
            );
            words.iter().all(|word| haystack.contains(word))
        })
        .collect()
}

pub fn open(state: &mut AppState) {
    state.commands = Commands::default();
    state.modal = Modal::Commands;
}

/// Where the palette sits in `area`.
pub fn modal_area(area: Rect) -> Rect {
    ui::centered(area, 92.min(area.width), 22.min(area.height))
}

/// Rows of the list, below the search line and its gap, above the hint line.
fn list_area(area: Rect) -> Rect {
    let inner = ui::modal_inner(modal_area(area));
    Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(4),
        ..inner
    }
}

/// First entry shown, so the selected one is always on screen.
fn first_shown(selected: usize, rows: usize) -> usize {
    selected.saturating_sub(rows.saturating_sub(1))
}

/// The entry drawn on screen row `row`, if any.
pub fn entry_at(state: &AppState, area: Rect, column: u16, row: u16) -> Option<usize> {
    let list = list_area(area);
    if column < list.x
        || column >= list.x + list.width
        || row < list.y
        || row >= list.y + list.height
    {
        return None;
    }
    let at = first_shown(state.commands.selected, list.height as usize) + (row - list.y) as usize;
    (at < actions(state).len()).then_some(at)
}

/// A click on an entry selects it; the wheel moves through them. Running one
/// stays on Enter, which says why when it cannot run from here.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &ratatui::crossterm::event::MouseEvent) {
    if let Some(down) = super::pointer::wheel(m) {
        move_selection(state, down, 3);
    } else if super::pointer::left_click(m)
        && let Some(at) = entry_at(state, area, m.column, m.row)
    {
        state.commands.selected = at;
    }
}

/// Move the selection by `by` entries, staying inside the list.
pub fn move_selection(state: &mut AppState, down: bool, by: usize) {
    let len = actions(state).len();
    state.commands.selected = if down {
        state
            .commands
            .selected
            .saturating_add(by)
            .min(len.saturating_sub(1))
    } else {
        state.commands.selected.saturating_sub(by)
    };
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = modal_area(area);
    let inner = ui::modal_frame(frame, modal, "Actions");
    let list = list_area(area);
    let entries = actions(state);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Search: ", Style::default().fg(Color::Yellow)),
            Span::raw(state.commands.query.to_string()),
        ]),
        Line::from(""),
    ];
    let rows = list.height as usize;
    let first = first_shown(state.commands.selected, rows);
    for (i, entry) in entries.iter().enumerate().skip(first).take(rows) {
        let selected = i == state.commands.selected;
        let where_and_why = match (&entry.run, entry.context) {
            _ if entry.needs_repo && state.repo_root.is_none() => {
                " \u{2014} unavailable: select a Git repository".to_string()
            }
            (Some(_), Some(context)) => format!(" \u{b7} {context}"),
            (Some(_), None) => String::new(),
            (None, Some(context)) => format!(" \u{b7} in {context}"),
            (None, None) => " \u{b7} not a key".to_string(),
        };
        let style = if entry.run.is_some() {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut line = Line::from(vec![
            Span::raw(if selected { "\u{203a} " } else { "  " }),
            Span::styled(entry.name.clone(), style),
            Span::styled(
                format!(" [{}]", entry.key),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(where_and_why, Style::default().fg(Color::DarkGray)),
        ]);
        if selected {
            line = line.style(Style::default().add_modifier(Modifier::BOLD));
        }
        lines.push(line);
    }
    while lines.len() < rows + 2 {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(""));
    lines.push(ui::key_hints(&crate::panel::keys::footer_pairs("Actions")));
    frame.render_widget(Paragraph::new(lines), inner);
    let column = ("Search: ".len() + state.commands.query.before_cursor().chars().count()) as u16;
    if column < inner.width {
        frame.set_cursor_position(Position::new(inner.x + column, inner.y));
    }
}

/// Run the selected entry: the keys to press for it, handed back to the
/// dispatcher, or nothing when it ran here or cannot run from here at all.
pub fn run_selected(state: &mut AppState) -> Vec<KeyEvent> {
    let Some(entry) = actions(state).get(state.commands.selected).cloned() else {
        return Vec::new();
    };
    if entry.needs_repo && state.repo_root.is_none() {
        state.set_status("select a Git repository first", false);
        return Vec::new();
    }
    match entry.run {
        Some(Run::Keys(keys)) => {
            state.modal = Modal::None;
            keys
        }
        Some(Run::Settings(category)) => {
            state.modal = Modal::None;
            super::settings::open(state, category);
            Vec::new()
        }
        None => {
            let place = entry
                .context
                .map_or(String::new(), |context| format!(" in {context}"));
            state.set_status(
                format!(
                    "{} is {}{place} \u{2014} it cannot run from here",
                    entry.name, entry.key
                ),
                false,
            );
            Vec::new()
        }
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Vec<KeyEvent> {
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Down => move_selection(state, true, 1),
        KeyCode::Up => move_selection(state, false, 1),
        KeyCode::PageDown => move_selection(state, true, 10),
        KeyCode::PageUp => move_selection(state, false, 10),
        KeyCode::Enter => return run_selected(state),
        _ => {
            if state.commands.query.edit_key(key) == super::text_input::Edit::Changed {
                state.commands.selected = 0;
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_query(state: &mut AppState, text: &str) {
        for c in text.chars() {
            handle_key(state, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// Every key the help documents can be found by what it does.
    #[test]
    fn a_pane_key_is_found_by_what_it_does_and_runs_in_its_pane() {
        let mut state = AppState::new();
        open(&mut state);
        type_query(&mut state, "roll back");

        let entries = actions(&state);
        let first = entries.first().expect("an entry");
        assert_eq!(first.key, "r");
        assert_eq!(first.context, Some("Files"));

        assert_eq!(run_selected(&mut state), vec![key('2'), key('r')]);
        assert_eq!(state.modal, Modal::None);
    }

    /// A key that only means something inside a modal says where it is
    /// instead of pressing it somewhere it means something else.
    #[test]
    fn a_modal_key_shows_where_it_works_and_is_not_pressed() {
        let mut state = AppState::new();
        open(&mut state);
        type_query(&mut state, "approve");
        let at = actions(&state)
            .iter()
            .position(|entry| entry.context == Some("GitHub"))
            .expect("the GitHub approve key is listed");
        state.commands.selected = at;

        assert!(run_selected(&mut state).is_empty());
        assert_eq!(state.modal, Modal::Commands, "the palette stays open");
        let status = state.status.as_ref().expect("a status");
        assert!(status.text.contains("in GitHub"), "{}", status.text);
    }

    #[test]
    fn the_search_box_edits_at_its_cursor() {
        let mut state = AppState::new();
        open(&mut state);
        type_query(&mut state, "psh");
        handle_key(&mut state, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        handle_key(&mut state, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        type_query(&mut state, "u");

        assert_eq!(state.commands.query, "push");
    }

    #[test]
    fn keys_named_in_the_table_parse_to_key_presses() {
        assert_eq!(parse_key("space / y"), Some(key(' ')));
        assert_eq!(
            parse_key("Ctrl-d / Ctrl-u"),
            Some(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        );
        assert_eq!(
            parse_key("Enter / l / \u{2192}"),
            Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        );
        assert_eq!(parse_key("drag divider"), None);
        assert_eq!(parse_key("wheel"), None);
    }
}
