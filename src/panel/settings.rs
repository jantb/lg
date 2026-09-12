//! One settings surface with explicit scope, inherited values and staged edits.
//!
//! The modal is one box: categories down the left, the fields of the chosen
//! category on the right with a description of the selected one beneath them,
//! and the keys along the bottom. Fields are shown as a tree — an agent, an
//! environment, a promotion each get a heading with their fields indented
//! under it — and long values wrap rather than run off the pane.
use crate::{
    preferences::{self, Scope},
    state::{AppState, Modal},
    ui::{self, palette},
};
use anyhow::{Context, Result};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use serde_json::{Value, json};
use std::cell::Cell;

mod draw;
mod fields;
mod input;
pub use draw::render;
use draw::*;
use fields::*;
use input::*;
pub use input::{handle_key, handle_mouse, handle_paste, poll};

const CATEGORIES: &[(&str, &str)] = &[
    ("identity", "Identity"),
    ("writing", "Writing"),
    ("models", "Models"),
    ("agents", "Agents"),
    ("sandbox", "Sandbox"),
    ("branches", "Branches & Environments"),
    ("tools", "Interface & Tools"),
    ("activity", "Activity & Setup"),
    ("sessions", "Sessions"),
];
/// One line on what each category holds, shown above its fields.
fn describe_category(key: &str) -> &'static str {
    match key {
        "identity" => "Who commits made from lg are attributed to, and how widely that applies.",
        "writing" => "How commit messages, pull request text and reviews are phrased and sized.",
        "models" => "The language model lg talks to, and where it is reached.",
        "agents" => {
            "The coding agents and shells a session can run, and how tightly each is confined."
        }
        "sandbox" => "The macOS Seatbelt profile confined agents run under.",
        "branches" => {
            "The trunk, the protected branches, the environments that deploy, and how a change is promoted between them."
        }
        "tools" => "The programs lg hands off to, and how the interface behaves.",
        "activity" => "What lg found on this machine, and the most recent status messages.",
        "sessions" => "The agent and terminal sessions currently open in this workspace.",
        _ => "",
    }
}
/// Width of the category column, including its marker.
const CATEGORY_WIDTH: u16 = 26;
/// Rows the description pane takes under the field list: a heading line, a
/// two-line description and room for a note.
const DETAIL_HEIGHT: u16 = 5;
/// Lines a wrapped value may take in the list before it is cut with an
/// ellipsis; the whole value is always shown in the editor and described in
/// the detail pane.
const MAX_VALUE_LINES: usize = 4;

/// The colour a heading of nested fields is drawn in, by depth: an agent or an
/// environment at the top, a promotion's fields one step in.
const GROUP_COLORS: [Color; 2] = [palette::ACCENT, palette::LANE_MAIN];
const KEY_COLOR: Color = Color::Rgb(220, 224, 232);
const MUTED: Color = palette::TEXT_IDLE;
const HINT_KEY: Color = palette::HINT_KEY;
const OK: Color = palette::OK;
const BAD: Color = palette::BAD;

/// Editors worth offering when one is found on PATH. `$VISUAL` and `$EDITOR`
/// go in front of these, since they are what the person already chose.
const KNOWN_EDITORS: &[&str] = &[
    "code", "cursor", "zed", "windsurf", "subl", "idea", "nvim", "vim", "vi", "hx", "micro",
    "nano", "emacs", "kak",
];
/// Shells worth offering for a terminal session; `$SHELL` goes in front.
const KNOWN_SHELLS: &[&str] = &["zsh", "bash", "fish", "nu", "sh", "dash", "ksh", "tcsh"];

/// Which pane the arrow keys move: the category list or its fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Categories,
    Fields,
}

#[derive(Debug)]
pub struct Settings {
    pub category: usize,
    pub scope: Scope,
    pub selected: usize,
    pub focus: Focus,
    pub draft: Value,
    original: Value,
    pub editing: bool,
    pub input: String,
    pub notice: String,
    notice_error: bool,
    pub source: String,
    pub query: String,
    searching: bool,
    reset_pending: bool,
    folder: String,
    editing_folder: bool,
    cursor: usize,
    /// Local branch names, offered when a field names a branch.
    branches: Vec<String>,
    /// The models the endpoint serves, offered when a field names a model.
    models: Vec<String>,
    /// Editors found on this machine, offered for the editor field.
    editors: Vec<String>,
    /// Shells found on this machine, offered for the terminal field.
    shells: Vec<String>,
    /// The highlighted option in the picker, an index into `choices`.
    choice: Option<usize>,
    /// Whether the picker's text has been typed rather than carried over from
    /// the field; only typed text narrows the list.
    typed: bool,
    /// First list line in view. Set while drawing, so the frame that moved
    /// the selection is the frame that scrolls to it, and read back when a
    /// click has to be mapped onto a row.
    scroll: Cell<usize>,
    diagnostics: Option<std::sync::mpsc::Receiver<String>>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            category: 0,
            scope: Scope::Repository,
            selected: 0,
            focus: Focus::Fields,
            draft: json!({}),
            original: json!({}),
            editing: false,
            input: String::new(),
            notice: String::new(),
            notice_error: false,
            source: String::new(),
            query: String::new(),
            searching: false,
            reset_pending: false,
            folder: String::new(),
            editing_folder: false,
            diagnostics: None,
            cursor: 0,
            branches: Vec::new(),
            models: Vec::new(),
            editors: Vec::new(),
            shells: Vec::new(),
            choice: None,
            typed: false,
            scroll: Cell::new(0),
        }
    }
}
impl Settings {
    fn key(&self) -> &'static str {
        CATEGORIES[self.category].0
    }
    fn dirty(&self) -> bool {
        self.draft != self.original || self.editing
    }
    fn reload(&mut self) {
        let loaded = preferences::load();
        let value = serde_json::to_value(&loaded.config).unwrap_or_default();
        self.draft = if self.key() == "identity" {
            let author = crate::git::author_config().ok();
            json!({"scope":"Repository", "folder":crate::git::repo_root().unwrap_or_default(), "name":author.as_ref().and_then(|a| a.name.clone()).unwrap_or_default(), "email":author.and_then(|a| a.email).unwrap_or_default()})
        } else {
            value.get(self.key()).cloned().unwrap_or(json!({}))
        };
        self.original = self.draft.clone();
        self.source = loaded.sources.get(self.key()).cloned().unwrap_or_else(|| {
            if self.key() == "branches" {
                preferences::DETECTED_SOURCE.into()
            } else {
                "Built-in defaults / inherited Git configuration".into()
            }
        });
        let prefix = format!("{}.", self.key());
        for (key, source) in loaded
            .sources
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
        {
            self.source.push_str(&format!("; {key}: {source}"));
        }
        self.notice = loaded.errors.join("\n");
        self.notice_error = !loaded.errors.is_empty();
        self.editing = false;
        self.selected = 0;
        self.scroll.set(0);
        self.reset_pending = false;
    }
    /// Whether the value at `path` differs from what was loaded.
    fn changed(&self, path: &str) -> bool {
        self.draft.pointer(path) != self.original.pointer(path)
    }
}
pub fn open(state: &mut AppState, category: usize) {
    state.settings_hub = Settings {
        category,
        ..Settings::default()
    };
    state.settings_hub.folder = preferences::default_folder().display().to_string();
    state.settings_hub.branches = local_branches(state);
    if crate::llm::available_models().is_empty() {
        crate::llm::prime_models_async();
    }
    state.settings_hub.models = crate::llm::available_models();
    state.settings_hub.editors = programs_on_path(&["VISUAL", "EDITOR"], KNOWN_EDITORS);
    state.settings_hub.shells = programs_on_path(&["SHELL"], KNOWN_SHELLS);
    state.settings_hub.reload();
    load_sessions(state);
    state.modal = Modal::Settings;
}
/// The sessions category edits the live session list rather than a file.
fn load_sessions(state: &mut AppState) {
    if state.settings_hub.category != 8 {
        return;
    }
    state.settings_hub.draft = Value::Array(state.sessions.iter().map(|s| json!({"id": s.id.to_string(), "label":s.label, "path":s.cwd.display().to_string(), "kind":s.kind.label()})).collect());
    state.settings_hub.original = state.settings_hub.draft.clone();
}
/// Moves to another category, unless edits would be lost on the way.
fn switch_category(state: &mut AppState, category: usize) {
    let hub = &mut state.settings_hub;
    if hub.dirty() {
        hub.notice = "Save or discard edits before changing category or scope.".into();
        return;
    }
    hub.category = category % CATEGORIES.len();
    hub.query.clear();
    hub.reload();
    load_sessions(state);
}
fn move_selection(state: &mut AppState, down: bool, steps: usize) {
    let hub = &mut state.settings_hub;
    let last = if hub.key() == "activity" {
        state.status_history.len().saturating_sub(1)
    } else {
        fields(hub).len().saturating_sub(1)
    };
    hub.selected = if down {
        hub.selected.saturating_add(steps).min(last)
    } else {
        hub.selected.saturating_sub(steps)
    };
}
fn local_branches(state: &AppState) -> Vec<String> {
    state.branches.iter().map(|b| b.name.clone()).collect()
}
/// Whether the field names a branch of this repository: the integration
/// branch or the branch an environment deploys.
fn names_branch(hub: &Settings, field: &Field) -> bool {
    hub.key() == "branches"
        && (field.path == "/base"
            || (field.path.starts_with("/environments/") && field.key == "branch"))
}
/// Whether the field names the model requests are sent to.
fn names_model(hub: &Settings, field: &Field) -> bool {
    hub.key() == "models" && field.path == "/model"
}
/// The programs named by `env` variables and found on PATH out of `known`,
/// in that order, each once. What the editor and terminal fields offer: the
/// value has to be a program on this machine, so the list is what is here.
fn programs_on_path(env: &[&str], known: &[&str]) -> Vec<String> {
    let dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let mut found = Vec::new();
    for name in env.iter().filter_map(|var| std::env::var(var).ok()) {
        let name = name.trim().to_string();
        if !name.is_empty() && !found.contains(&name) {
            found.push(name);
        }
    }
    for name in known {
        if dirs.iter().any(|dir| dir.join(name).is_file()) && !found.iter().any(|f| f == name) {
            found.push((*name).to_string());
        }
    }
    found
}
/// The ids of the environments in the draft, for the promotion fields that
/// name one.
fn environment_ids(hub: &Settings) -> Vec<String> {
    hub.draft["environments"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|e| e["id"].as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    /// A hub over the configuration detected for a checkout with develop and
    /// test branches, with no file read.
    fn branches_hub() -> AppState {
        let mut state = AppState::default();
        state.decorative_animations = false;
        let detected =
            preferences::detect_branches(&["main".into(), "develop".into(), "test".into()]);
        state.settings_hub = Settings {
            category: 5,
            draft: serde_json::to_value(detected).unwrap(),
            ..Settings::default()
        };
        state.settings_hub.original = state.settings_hub.draft.clone();
        state
    }

    fn drawn(state: &AppState, area: Rect) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal.draw(|frame| render(state, area, frame)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn nested_entries_are_shown_under_their_own_heading() {
        let state = branches_hub();
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 120)));
        assert!(screen.contains("#1 Development"), "{screen}");
        assert!(screen.contains("#1 feature \u{2192} dev"), "{screen}");
        assert!(
            !screen.contains("environments #1 \u{b7} branch"),
            "the flat labels are gone: {screen}"
        );
    }

    #[test]
    fn a_long_value_wraps_instead_of_running_off_the_pane() {
        let mut state = branches_hub();
        state.settings_hub.draft["base"] = Value::String("word ".repeat(40).trim().into());
        let screen = text(&drawn(&state, Rect::new(0, 0, 100, 40)));
        let value_rows = screen
            .lines()
            .filter(|line| line.contains("word word"))
            .count();
        assert!(value_rows >= 2, "{screen}");
    }

    #[test]
    fn clicking_a_field_selects_it_and_clicking_again_edits_it() {
        let mut state = branches_hub();
        let area = Rect::new(0, 0, 120, 120);
        let r = regions(area);
        // Draw once so the scroll position is settled, then find `remote` on screen.
        let buf = drawn(&state, area);
        let row = (r.fields.y..r.fields.y + r.fields.height)
            .find(|y| {
                // Past the two marker columns; the top-level field is not indented.
                (r.fields.x + 2..r.fields.x + r.fields.width)
                    .map(|x| buf[(x, *y)].symbol().to_string())
                    .collect::<String>()
                    .starts_with("remote: origin")
            })
            .expect("remote is on screen");
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.fields.x + 4,
            row,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(&mut state, area, &click);
        let selected = fields(&state.settings_hub)[state.settings_hub.selected].clone();
        assert_eq!(selected.path, "/remote");
        assert!(!state.settings_hub.editing);
        handle_mouse(&mut state, area, &click);
        assert!(state.settings_hub.editing);
        assert_eq!(state.settings_hub.input, "origin");
    }

    #[test]
    fn clicking_a_category_switches_to_it() {
        let mut state = branches_hub();
        let area = Rect::new(0, 0, 120, 40);
        let r = regions(area);
        handle_mouse(
            &mut state,
            area,
            &MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: r.categories.x + 3,
                row: r.categories.y + 3,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(state.settings_hub.key(), "agents");
    }

    fn press(state: &mut AppState, code: KeyCode) {
        handle_key(state, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    /// A hub over the writing category with the default preferences.
    fn writing_hub() -> AppState {
        let mut state = AppState::default();
        state.decorative_animations = false;
        state.settings_hub = Settings {
            category: 1,
            draft: serde_json::to_value(preferences::Preferences::default()).unwrap(),
            ..Settings::default()
        };
        state.settings_hub.original = state.settings_hub.draft.clone();
        state
    }

    #[test]
    fn arrows_step_a_number_while_editing_it() {
        let mut state = writing_hub();
        select(&mut state, "/writing/body_lines");
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::PageUp);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.settings_hub.draft["writing"]["body_lines"], json!(19));
        assert!(!state.settings_hub.editing);
    }

    #[test]
    fn a_number_never_steps_below_zero() {
        let mut state = writing_hub();
        select(&mut state, "/writing/subject_max");
        press(&mut state, KeyCode::Enter);
        for _ in 0..10 {
            press(&mut state, KeyCode::PageDown);
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.settings_hub.draft["writing"]["subject_max"], json!(0));
    }

    #[test]
    fn language_is_picked_from_the_supported_languages() {
        let mut state = writing_hub();
        select(&mut state, "/writing/language");
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        assert_eq!(
            state.settings_hub.draft["writing"]["language"],
            json!("Norwegian")
        );
        press(&mut state, KeyCode::Enter);
        for c in "Klingon".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);
        assert!(
            state.settings_hub.editing,
            "an unsupported language is refused"
        );
        assert!(
            state.settings_hub.notice_error,
            "{}",
            state.settings_hub.notice
        );
        assert_eq!(
            state.settings_hub.draft["writing"]["language"],
            json!("Norwegian")
        );
    }

    #[test]
    fn enter_on_a_boolean_flips_it_without_opening_the_editor() {
        let mut state = branches_hub();
        state.settings_hub.draft["flag"] = Value::Bool(false);
        state.settings_hub.original = state.settings_hub.draft.clone();
        state.settings_hub.selected = fields(&state.settings_hub)
            .iter()
            .position(|f| f.path == "/flag")
            .unwrap();
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.settings_hub.draft["flag"], Value::Bool(true));
        assert!(!state.settings_hub.editing);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.settings_hub.draft["flag"], Value::Bool(false));
    }

    #[test]
    fn left_arrow_moves_to_the_categories_and_up_down_switch_them() {
        let mut state = branches_hub();
        let before = state.settings_hub.key();
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Down);
        assert_ne!(state.settings_hub.key(), before);
        press(&mut state, KeyCode::Up);
        assert_eq!(state.settings_hub.key(), before);
        // Back in the fields, Down moves the field selection, not the category.
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Down);
        assert_eq!(state.settings_hub.key(), before);
        assert_eq!(state.settings_hub.selected, 1);
    }

    fn with_local_branches(state: &mut AppState, names: &[&str]) {
        state.branches = names
            .iter()
            .map(|name| crate::git::Branch {
                name: (*name).into(),
                is_current: false,
                upstream: None,
                upstream_gone: false,
                ahead: 0,
                behind: 0,
                behind_main: 0,
                last_commit_unix: None,
            })
            .collect();
    }

    fn select(state: &mut AppState, path: &str) {
        state.settings_hub.selected = fields(&state.settings_hub)
            .iter()
            .position(|f| f.path == path)
            .unwrap();
    }

    /// A field that takes one of a few words is chosen from those words, and
    /// a word that is not one of them never reaches the draft.
    #[test]
    fn a_promotion_strategy_is_chosen_from_the_allowed_values() {
        let mut state = branches_hub();
        select(&mut state, "/promotions/0/strategy");
        let before = state.settings_hub.draft["promotions"][0]["strategy"].clone();
        press(&mut state, KeyCode::Enter);
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
        for option in preferences::STRATEGIES {
            assert!(screen.contains(option), "{option} is offered: {screen}");
        }
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        assert!(!state.settings_hub.editing);
        let after = state.settings_hub.draft["promotions"][0]["strategy"].clone();
        assert_ne!(after, before);
        assert!(preferences::STRATEGIES.contains(&after.as_str().unwrap()));

        press(&mut state, KeyCode::Enter);
        for c in "rebase".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.settings_hub.draft["promotions"][0]["strategy"], after);
        assert!(
            state.settings_hub.notice.contains("one of"),
            "{}",
            state.settings_hub.notice
        );
    }

    /// Typing the whole word is as good as highlighting it.
    #[test]
    fn a_typed_allowed_value_is_applied() {
        let mut state = branches_hub();
        select(&mut state, "/promotions/0/strategy");
        press(&mut state, KeyCode::Enter);
        for c in "squash".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(
            state.settings_hub.draft["promotions"][0]["strategy"],
            Value::String("squash".into())
        );
    }

    /// An agent's confinement is one of three words, offered as a list.
    #[test]
    fn confinement_is_chosen_from_a_list() {
        let mut state = AppState::default();
        state.decorative_animations = false;
        let config = preferences::Preferences::default();
        state.settings_hub = Settings {
            category: 3,
            draft: serde_json::to_value(&config.agents).unwrap(),
            ..Settings::default()
        };
        state.settings_hub.original = state.settings_hub.draft.clone();
        select(&mut state, "/0/confinement");
        press(&mut state, KeyCode::Enter);
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
        for option in preferences::CONFINEMENTS {
            assert!(screen.contains(option), "{option} is offered: {screen}");
        }
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        let chosen = state.settings_hub.draft[0]["confinement"].clone();
        assert!(preferences::CONFINEMENTS.contains(&chosen.as_str().unwrap()));
    }

    /// The editor is picked from the programs on this machine, and a program
    /// that is not listed may still be typed.
    #[test]
    fn the_editor_is_offered_from_the_programs_found() {
        let mut state = AppState::default();
        state.decorative_animations = false;
        let config = preferences::Preferences::default();
        state.settings_hub = Settings {
            category: 6,
            draft: serde_json::to_value(&config.tools).unwrap(),
            ..Settings::default()
        };
        state.settings_hub.original = state.settings_hub.draft.clone();
        state.settings_hub.editors = vec!["hx".into(), "nvim".into()];
        select(&mut state, "/editor");
        press(&mut state, KeyCode::Enter);
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
        assert!(screen.contains("nvim"), "{screen}");
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        assert_eq!(
            state.settings_hub.draft["editor"],
            Value::String("hx".into())
        );

        press(&mut state, KeyCode::Enter);
        for c in "mate".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(
            state.settings_hub.draft["editor"],
            Value::String("mate".into())
        );
    }

    #[test]
    fn an_environment_branch_is_picked_from_the_local_branches() {
        let mut state = branches_hub();
        with_local_branches(&mut state, &["main", "staging", "feature/x"]);
        select(&mut state, "/environments/1/branch");
        press(&mut state, KeyCode::Enter);
        assert!(state.settings_hub.editing);
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
        assert!(screen.contains("staging"), "{screen}");
        for c in "stag".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        assert!(!state.settings_hub.editing);
        assert_eq!(
            state.settings_hub.draft["environments"][1]["branch"],
            Value::String("staging".into())
        );
    }

    #[test]
    fn the_model_is_picked_from_what_the_endpoint_serves() {
        let mut state = AppState::default();
        state.decorative_animations = false;
        state.settings_hub = Settings {
            category: 2,
            draft: json!({"endpoint": "http://localhost:8000/v1/chat/completions", "model": "old"}),
            models: vec!["Qwen3-27B".into(), "gpt-oss-120b".into()],
            ..Settings::default()
        };
        state.settings_hub.original = state.settings_hub.draft.clone();
        select(&mut state, "/model");
        press(&mut state, KeyCode::Enter);
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
        assert!(screen.contains("Qwen3-27B"), "{screen}");
        assert!(screen.contains("gpt-oss-120b"), "{screen}");
        for c in "gpt".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        assert!(!state.settings_hub.editing);
        assert_eq!(
            state.settings_hub.draft["model"],
            Value::String("gpt-oss-120b".into())
        );
    }

    #[test]
    fn a_typed_branch_name_is_kept_when_nothing_is_picked() {
        let mut state = branches_hub();
        with_local_branches(&mut state, &["main"]);
        select(&mut state, "/base");
        press(&mut state, KeyCode::Enter);
        for c in "trunk".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(
            state.settings_hub.draft["base"],
            Value::String("trunk".into())
        );
    }

    #[test]
    fn assigning_an_environment_branch_makes_it_a_release_target() {
        let mut state = branches_hub();
        with_local_branches(&mut state, &["staging"]);
        press(&mut state, KeyCode::Char('n'));
        let index = state.settings_hub.draft["environments"]
            .as_array()
            .unwrap()
            .len()
            - 1;
        let id = state.settings_hub.draft["environments"][index]["id"]
            .as_str()
            .unwrap()
            .to_string();
        select(&mut state, &format!("/environments/{index}/branch"));
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        let draft = &state.settings_hub.draft;
        assert!(
            draft["protected"]
                .as_array()
                .unwrap()
                .contains(&Value::String("staging".into())),
            "{draft}"
        );
        assert!(
            draft["promotions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["to"].as_str() == Some(id.as_str())),
            "{draft}"
        );
    }

    #[test]
    fn the_selected_field_is_explained_below_the_list() {
        let mut state = branches_hub();
        state.settings_hub.selected = fields(&state.settings_hub)
            .iter()
            .position(|f| f.path == "/environments/0/id")
            .unwrap();
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
        assert!(screen.contains("dev and test"), "{screen}");
    }
}
