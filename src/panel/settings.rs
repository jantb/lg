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

/// The key table section the settings screen's keys come from right now.
pub fn key_section(hub: &Settings) -> &'static str {
    if hub.editing {
        "Settings: editing"
    } else {
        "Settings"
    }
}

/// Whether the field being edited offers a list to pick from.
pub fn editing_with_picker(hub: &Settings) -> bool {
    hub.editing && picking(hub)
}

/// The groups of settings the screen lists down its left side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Identity,
    Writing,
    Models,
    Agents,
    Sandbox,
    Branches,
    Tools,
    Activity,
    Sessions,
}
impl Category {
    pub const ALL: [Self; 9] = [
        Self::Identity,
        Self::Writing,
        Self::Models,
        Self::Agents,
        Self::Sandbox,
        Self::Branches,
        Self::Tools,
        Self::Activity,
        Self::Sessions,
    ];
    /// The table the category's settings are saved under, and what the
    /// sources of the loaded configuration are keyed by.
    pub fn key(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Writing => "writing",
            Self::Models => "models",
            Self::Agents => "agents",
            Self::Sandbox => "sandbox",
            Self::Branches => "branches",
            Self::Tools => "tools",
            Self::Activity => "activity",
            Self::Sessions => "sessions",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Identity => "Identity",
            Self::Writing => "Writing",
            Self::Models => "Models",
            Self::Agents => "Agents",
            Self::Sandbox => "Sandbox",
            Self::Branches => "Branches & Environments",
            Self::Tools => "Interface & Tools",
            Self::Activity => "Activity & Setup",
            Self::Sessions => "Sessions",
        }
    }
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|category| *category == self)
            .unwrap_or(0)
    }
}
const CATEGORIES: [Category; 9] = Category::ALL;
/// What a notice under the fields reports, which is what colours it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeKind {
    Info,
    Success,
    /// Something is held back until it is confirmed or dealt with.
    Warning,
    Error,
}
/// One line on what each category holds, shown above its fields.
fn describe_category(category: Category) -> &'static str {
    match category {
        Category::Identity => {
            "Who commits made from lg are attributed to, and how widely that applies."
        }
        Category::Writing => {
            "How commit messages, pull request text and reviews are phrased and sized."
        }
        Category::Models => {
            "The language model lg talks to: a local chat endpoint or the claude CLI, whichever provider names."
        }
        Category::Agents => {
            "The coding agents and shells a session can run, and how tightly each is confined."
        }
        Category::Sandbox => "The macOS Seatbelt profile confined agents run under.",
        Category::Branches => {
            "The trunk, the protected branches, the environments that deploy, and how a change is promoted between them."
        }
        Category::Tools => "The programs lg hands off to, and how the interface behaves.",
        Category::Activity => "What lg found on this machine, and the most recent status messages.",
        Category::Sessions => "The agent and terminal sessions currently open in this workspace.",
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
    pub input: crate::panel::text_input::TextInput,
    pub notice: String,
    notice_kind: NoticeKind,
    /// The unsaved-changes warning is showing, so another Esc discards.
    discard_armed: bool,
    pub source: String,
    pub query: String,
    searching: bool,
    reset_pending: bool,
    folder: String,
    /// The file each scope saves to, found when the modal opens: the header
    /// shows it every frame, and locating a checkout means asking git.
    scope_files: Vec<(Scope, String)>,
    editing_folder: bool,
    /// Local branch names, offered when a field names a branch.
    branches: Vec<String>,
    /// The models the endpoint serves, offered when a field names a model.
    models: Vec<String>,
    /// The models the claude CLI offers, for the Claude provider's model.
    claude_models: Vec<crate::llm::ClaudeModel>,
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
    /// A session that is still working, and the key (`x` close, `R` restart)
    /// pressed on it once: pressing it again does it.
    session_armed: Option<(char, crate::session::SessionId)>,
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
            input: Default::default(),
            notice: String::new(),
            notice_kind: NoticeKind::Info,
            discard_armed: false,
            source: String::new(),
            query: String::new(),
            searching: false,
            reset_pending: false,
            folder: String::new(),
            scope_files: Vec::new(),
            editing_folder: false,
            diagnostics: None,
            branches: Vec::new(),
            models: Vec::new(),
            claude_models: Vec::new(),
            editors: Vec::new(),
            shells: Vec::new(),
            choice: None,
            typed: false,
            scroll: Cell::new(0),
            session_armed: None,
        }
    }
}
impl Settings {
    fn current_category(&self) -> Category {
        CATEGORIES[self.category]
    }
    fn key(&self) -> &'static str {
        self.current_category().key()
    }
    /// Show `text` under the fields, coloured by `kind`.
    fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = text.into();
        self.notice_kind = kind;
        self.discard_armed = false;
    }
    fn dirty(&self) -> bool {
        self.draft != self.original || self.editing
    }
    fn reload(&mut self) {
        let loaded = preferences::load();
        let value = serde_json::to_value(&loaded.config).unwrap_or_default();
        self.draft = if self.current_category() == Category::Identity {
            let author = crate::git::author_config().ok();
            json!({"scope":"Repository", "folder":crate::git::repo_root().unwrap_or_default(), "name":author.as_ref().and_then(|a| a.name.clone()).unwrap_or_default(), "email":author.and_then(|a| a.email).unwrap_or_default()})
        } else {
            value.get(self.key()).cloned().unwrap_or(json!({}))
        };
        self.original = self.draft.clone();
        self.source = loaded.sources.get(self.key()).cloned().unwrap_or_else(|| {
            if self.current_category() == Category::Branches {
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
        let kind = if loaded.errors.is_empty() {
            NoticeKind::Info
        } else {
            NoticeKind::Error
        };
        self.notify(kind, loaded.errors.join("\n"));
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
pub fn open(state: &mut AppState, category: Category) {
    state.settings_hub = Settings {
        category: category.index(),
        ..Settings::default()
    };
    state.settings_hub.folder = preferences::default_folder().display().to_string();
    state.settings_hub.scope_files = Scope::ALL
        .into_iter()
        .map(|scope| {
            let path = preferences::scope_path(scope)
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            (scope, path)
        })
        .collect();
    state.settings_hub.branches = local_branches(state);
    if crate::llm::available_models().is_empty() {
        crate::llm::prime_models_async();
    }
    state.settings_hub.models = crate::llm::available_models();
    crate::llm::prime_claude_models_async();
    state.settings_hub.claude_models = crate::llm::claude_models();
    state.settings_hub.editors = programs_on_path(&["VISUAL", "EDITOR"], KNOWN_EDITORS);
    state.settings_hub.shells = programs_on_path(&["SHELL"], KNOWN_SHELLS);
    state.settings_hub.reload();
    load_sessions(state);
    state.modal = Modal::Settings;
}
/// The sessions category edits the live session list rather than a file.
fn load_sessions(state: &mut AppState) {
    if state.settings_hub.current_category() != Category::Sessions {
        return;
    }
    state.settings_hub.draft = Value::Array(state.sessions.iter().map(|s| json!({"id": s.id.to_string(), "label":s.label, "path":s.cwd.display().to_string(), "kind":s.kind.label()})).collect());
    state.settings_hub.original = state.settings_hub.draft.clone();
}
/// Moves to another category, unless edits would be lost on the way.
fn switch_category(state: &mut AppState, category: usize) {
    let hub = &mut state.settings_hub;
    if hub.dirty() {
        hub.notify(
            NoticeKind::Warning,
            "Save or discard edits before changing category or scope.",
        );
        return;
    }
    hub.category = category % CATEGORIES.len();
    hub.query.clear();
    hub.reload();
    load_sessions(state);
}
fn move_selection(state: &mut AppState, down: bool, steps: usize) {
    let hub = &mut state.settings_hub;
    let last = if hub.current_category() == Category::Activity {
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
    hub.current_category() == Category::Branches
        && (field.path == "/base"
            || (field.path.starts_with("/environments/") && field.key == "branch"))
}
/// Whether the field names the model requests are sent to.
fn names_model(hub: &Settings, field: &Field) -> bool {
    hub.current_category() == Category::Models && field.path == "/model"
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
mod tests;
