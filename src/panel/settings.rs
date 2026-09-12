//! One settings surface with explicit scope, inherited values and staged edits.
use crate::{
    preferences::{self, Scope},
    state::{AppState, Modal},
    ui,
};
use anyhow::{Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use serde_json::{Value, json};

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
#[derive(Debug)]
pub struct Settings {
    pub category: usize,
    pub scope: Scope,
    pub selected: usize,
    pub draft: Value,
    original: Value,
    pub editing: bool,
    pub input: String,
    pub notice: String,
    pub source: String,
    pub query: String,
    searching: bool,
    reset_pending: bool,
    folder: String,
    editing_folder: bool,
    cursor: usize,
    diagnostics: Option<std::sync::mpsc::Receiver<String>>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            category: 0,
            scope: Scope::Repository,
            selected: 0,
            draft: json!({}),
            original: json!({}),
            editing: false,
            input: String::new(),
            notice: String::new(),
            source: String::new(),
            query: String::new(),
            searching: false,
            reset_pending: false,
            folder: String::new(),
            editing_folder: false,
            diagnostics: None,
            cursor: 0,
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
        let value = serde_json::to_value(&loaded.config).unwrap();
        self.draft = if self.key() == "identity" {
            let author = crate::git::author_config().ok();
            json!({"scope":"Repository", "folder":crate::git::repo_root().unwrap_or_default(), "name":author.as_ref().and_then(|a| a.name.clone()).unwrap_or_default(), "email":author.and_then(|a| a.email).unwrap_or_default()})
        } else {
            value.get(self.key()).cloned().unwrap_or(json!({}))
        };
        self.original = self.draft.clone();
        self.source = loaded
            .sources
            .get(self.key())
            .cloned()
            .unwrap_or_else(|| "Built-in defaults / inherited Git configuration".into());
        let prefix = format!("{}.", self.key());
        for (key, source) in loaded
            .sources
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
        {
            self.source.push_str(&format!("; {key}: {source}"));
        }
        self.notice = loaded.errors.join("\n");
        self.editing = false;
        self.selected = 0;
        self.reset_pending = false;
    }
}
pub fn open(state: &mut AppState, category: usize) {
    state.settings_hub = Settings {
        category,
        ..Settings::default()
    };
    state.settings_hub.folder = preferences::default_folder().display().to_string();
    state.settings_hub.reload();
    if category == 8 {
        state.settings_hub.draft = Value::Array(state.sessions.iter().map(|s| json!({"id": s.id.to_string(), "label":s.label, "path":s.cwd.display().to_string(), "kind":s.kind.label()})).collect());
        state.settings_hub.original = state.settings_hub.draft.clone();
    }
    state.modal = Modal::Settings;
}
#[derive(Clone)]
struct Field {
    path: String,
    label: String,
    value: Value,
}
fn flatten(value: &Value, path: &str, label: &str, out: &mut Vec<Field>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                flatten(
                    v,
                    &format!("{path}/{key}"),
                    &format!(
                        "{label}{}{}",
                        if label.is_empty() { "" } else { " · " },
                        key.replace('_', " ")
                    ),
                    out,
                );
            }
        }
        Value::Array(items) if items.iter().any(Value::is_object) => {
            for (i, v) in items.iter().enumerate() {
                flatten(
                    v,
                    &format!("{path}/{i}"),
                    &format!("{label} #{}", i + 1),
                    out,
                );
            }
        }
        _ => out.push(Field {
            path: path.into(),
            label: label.into(),
            value: value.clone(),
        }),
    }
}
fn fields(hub: &Settings) -> Vec<Field> {
    let mut fields = Vec::new();
    flatten(&hub.draft, "", "", &mut fields);
    fields.retain(|f| {
        hub.query.is_empty()
            || format!("{} {}", f.label, display(&f.value))
                .to_lowercase()
                .contains(&hub.query.to_lowercase())
    });
    fields
}
fn display(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
fn commit_edit(hub: &mut Settings) -> Result<()> {
    if hub.editing_folder {
        hub.folder = hub.input.clone();
        hub.editing_folder = false;
        hub.editing = false;
        return Ok(());
    }
    let field = fields(hub)
        .get(hub.selected)
        .cloned()
        .context("select a field")?;
    let value = match field.value {
        Value::String(_) => Value::String(hub.input.clone()),
        Value::Bool(_) => Value::Bool(hub.input.parse().context("use true or false")?),
        Value::Number(_) => Value::from(
            hub.input
                .parse::<u64>()
                .context("enter a nonnegative integer")?,
        ),
        _ => serde_json::from_str(&hub.input)
            .context("enter a JSON list, for example [\"--flag\", \"value\"]")?,
    };
    *hub.draft
        .pointer_mut(&field.path)
        .context("field no longer exists")? = value;
    hub.editing = false;
    Ok(())
}
pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let hub = &state.settings_hub;
    let modal = ui::centered(
        area,
        area.width.saturating_sub(4).min(130),
        area.height.saturating_sub(2),
    );
    let inner = ui::modal_frame(
        frame,
        modal,
        if hub.dirty() {
            "Settings — unsaved changes"
        } else {
            "Settings"
        },
    );
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(6),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(format!(
            "Editing scope: {}  ·  {}\nSource: {}\n{}",
            hub.scope.label(),
            if hub.scope == Scope::Folder {
                hub.folder.clone()
            } else {
                preferences::scope_path(hub.scope)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            },
            hub.source,
            if hub.searching {
                format!("Search: {}", hub.query)
            } else {
                String::new()
            }
        )),
        vertical[0],
    );
    let columns =
        Layout::horizontal([Constraint::Length(26), Constraint::Min(30)]).split(vertical[1]);
    let mut categories = ListState::default().with_selected(Some(hub.category));
    frame.render_stateful_widget(
        List::new(
            CATEGORIES
                .iter()
                .map(|(_, name)| ListItem::new(*name))
                .collect::<Vec<_>>(),
        )
        .highlight_style(Style::default().fg(Color::Cyan))
        .highlight_symbol("› "),
        columns[0],
        &mut categories,
    );
    if hub.key() == "activity" {
        let mut lines = vec![
            Line::from("Setup"),
            Line::from(format!(
                "Author: {} <{}>",
                state.author_name_input, state.author_email_input
            )),
        ];
        let loaded = preferences::load();
        for agent in &loaded.config.agents {
            lines.push(Line::from(crate::agents::describe(agent)));
        }
        lines.push(Line::from(format!(
            "Sandbox: {}",
            crate::terrarium::profile_path(std::path::Path::new(
                state.repo_root.as_deref().unwrap_or(".")
            ))
            .map(|p| if p.exists() {
                p.display().to_string()
            } else {
                "not configured — open Sandbox".into()
            })
            .unwrap_or_default()
        )));
        lines.push(Line::from("History (newest first)"));
        for status in state
            .status_history
            .iter()
            .rev()
            .skip(hub.selected)
            .take(100)
        {
            lines.push(Line::from(format!(
                "{} {} {}",
                status.at.format("%H:%M:%S"),
                if status.is_error { "ERROR" } else { "•" },
                status.text
            )));
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), columns[1]);
    } else if hub.editing {
        let block =
            ui::bordered("Editing — Enter apply · Shift-Enter newline · Esc cancel · Ctrl-U clear");
        let editor = block.inner(columns[1]);
        let (row, col) = cursor_position(&hub.input, hub.cursor, editor.width.max(1) as usize);
        let scroll = row.saturating_sub(editor.height.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(hub.input.as_str())
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((scroll.min(u16::MAX as usize) as u16, 0)),
            columns[1],
        );
        if editor.width > 0 && editor.height > 0 {
            frame.set_cursor_position((editor.x + col as u16, editor.y + (row - scroll) as u16));
        }
    } else {
        let fields = fields(hub);
        let items = fields
            .iter()
            .map(|f| {
                ListItem::new(format!(
                    "{}: {}",
                    f.label,
                    display(&f.value).replace('\n', " ↵ ")
                ))
            })
            .collect::<Vec<_>>();
        let mut selected = ListState::default()
            .with_selected(Some(hub.selected.min(items.len().saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
                .highlight_symbol("› "),
            columns[1],
            &mut selected,
        );
    }
    let help = match hub.key() {
        "identity" => {
            "Scope field: Repository or Folder. Folder affects Git repositories below that path. Signing remains inherited from Git."
        }
        "agents" => {
            "n add agent · D delete agent · adapter: claude/codex/pi/terminal · confinement: terrarium/agent/direct · v diagnostics"
        }
        "sandbox" => {
            "i initialize private profile · v validate · e edit profile · Bundled Terrarium (macOS Seatbelt); shared Git directory is writable."
        }
        "branches" => {
            "n add environment · p add promotion · D remove selected entry · t trunk template · b environment template · empty branch = deployment source unknown"
        }
        "writing" => {
            "v preview prompt for staged changes · Language/style suggestions remain available with L."
        }
        "sessions" => {
            "Edit labels and Ctrl-S to rename. R restarts the selected session; x closes it. Restart uses the original launch command, not an agent resume protocol."
        }
        "tools" => {
            "Decorative animations: true/false. quiet author emails: [\"me@client.com\", \"*@client.com\"]. Folder scope applies below the folder selected with f."
        }
        "activity" => {
            "j/k scroll history. Agent availability does not establish authentication. Model connectivity is checked when a request runs."
        }
        _ => {
            "Argument lists use JSON, e.g. [\"--wait\"]. Paths and values are passed directly, without a shell."
        }
    };
    frame.render_widget(Paragraph::new(format!("Tab/Shift-Tab category · j/k field · Enter edit · Ctrl-S save · s scope · f folder · / search · Esc close\nr reset override (preview, then r again) · {}\n{}", help, hub.notice)).wrap(Wrap { trim: false }), vertical[2]);
}
pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let result = handle(state, key);
    if let Err(e) = result {
        state.settings_hub.notice = format!("{e:#}");
    }
    Ok(())
}
fn handle(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let hub = &mut state.settings_hub;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if hub.searching {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => hub.searching = false,
            KeyCode::Backspace => {
                hub.query.pop();
            }
            KeyCode::Char(c) => hub.query.push(c),
            _ => {}
        }
        hub.selected = 0;
        return Ok(());
    }
    if hub.editing && !(ctrl && key.code == KeyCode::Char('s')) {
        match key.code {
            KeyCode::Esc => hub.editing = false,
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => insert(hub, "\n"),
            KeyCode::Enter => commit_edit(hub)?,
            KeyCode::Backspace if hub.cursor > 0 => {
                let mut chars: Vec<_> = hub.input.chars().collect();
                chars.remove(hub.cursor - 1);
                hub.cursor -= 1;
                hub.input = chars.into_iter().collect();
            }
            KeyCode::Delete if hub.cursor < hub.input.chars().count() => {
                let mut chars: Vec<_> = hub.input.chars().collect();
                chars.remove(hub.cursor);
                hub.input = chars.into_iter().collect();
            }
            KeyCode::Left => hub.cursor = hub.cursor.saturating_sub(1),
            KeyCode::Right => hub.cursor = (hub.cursor + 1).min(hub.input.chars().count()),
            KeyCode::Home => hub.cursor = 0,
            KeyCode::End => hub.cursor = hub.input.chars().count(),
            KeyCode::Char('u') if ctrl => {
                hub.input.clear();
                hub.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => insert(hub, &c.to_string()),
            _ => {}
        }
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            if hub.dirty() && hub.notice != "Unsaved changes. Ctrl-S saves; Esc again discards." {
                hub.notice = "Unsaved changes. Ctrl-S saves; Esc again discards.".into();
            } else {
                state.modal = Modal::None;
            }
        }
        KeyCode::Char('f') if hub.scope == Scope::Folder => {
            hub.input = hub.folder.clone();
            hub.cursor = hub.input.chars().count();
            hub.editing_folder = true;
            hub.editing = true;
        }
        KeyCode::Char('s') if !ctrl && !hub.dirty() => {
            hub.scope = match hub.scope {
                Scope::User => Scope::Folder,
                Scope::Folder => Scope::Repository,
                Scope::Repository => Scope::Worktree,
                Scope::Worktree => Scope::User,
            };
        }
        KeyCode::Tab | KeyCode::BackTab if !hub.dirty() => {
            hub.category = (hub.category
                + if key.code == KeyCode::Tab {
                    1
                } else {
                    CATEGORIES.len() - 1
                })
                % CATEGORIES.len();
            hub.query.clear();
            hub.reload();
            if hub.category == 8 {
                hub.draft = Value::Array(state.sessions.iter().map(|s| json!({"id": s.id.to_string(), "label":s.label, "path":s.cwd.display().to_string(), "kind":s.kind.label()})).collect());
                hub.original = hub.draft.clone();
            }
        }
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('s') if !ctrl => {
            hub.notice = "Save or discard edits before changing category or scope.".into()
        }
        KeyCode::Char('/') => hub.searching = true,
        KeyCode::Down | KeyCode::Char('j') => {
            hub.selected = (hub.selected + 1).min(if hub.key() == "activity" {
                state.status_history.len().saturating_sub(1)
            } else {
                fields(hub).len().saturating_sub(1)
            })
        }
        KeyCode::Up | KeyCode::Char('k') => hub.selected = hub.selected.saturating_sub(1),
        KeyCode::Enter => {
            if let Some(f) = fields(hub).get(hub.selected) {
                hub.input = display(&f.value);
                hub.cursor = hub.input.chars().count();
                hub.editing = true;
            }
        }
        KeyCode::Char('s') if ctrl => {
            if hub.editing {
                commit_edit(hub)?;
            }
            if hub.key() == "sessions" {
                for row in hub.draft.as_array().context("sessions")? {
                    let id = state
                        .sessions
                        .iter()
                        .find(|s| Some(s.id.to_string().as_str()) == row["id"].as_str())
                        .map(|s| s.id);
                    if let Some(id) = id {
                        state
                            .sessions
                            .rename(id, row["label"].as_str().unwrap_or_default())?;
                    }
                }
            } else if hub.key() == "identity" {
                let name = hub.draft["name"].as_str().unwrap_or_default();
                let email = hub.draft["email"].as_str().unwrap_or_default();
                match hub.draft["scope"].as_str() {
                    Some("Repository") => crate::git::set_local_author(name, email)?,
                    Some("Folder") => crate::git::set_subtree_author(
                        hub.draft["folder"].as_str().unwrap_or_default(),
                        name,
                        email,
                    )?,
                    _ => anyhow::bail!("Identity scope must be Repository or Folder"),
                }
            } else if hub.key() == "sandbox" && hub.draft.get("profile").is_some() {
                crate::terrarium::save_profile(hub.draft["profile"].as_str().unwrap_or_default())?;
            } else if hub.scope == Scope::Folder {
                preferences::save_folder(
                    std::path::Path::new(&hub.folder),
                    hub.key(),
                    hub.draft.clone(),
                )?;
            } else {
                preferences::save_category(hub.scope, hub.key(), hub.draft.clone())?;
            }
            state.decorative_animations = preferences::animations_enabled();
            if let Ok(author) = crate::git::author_config() {
                state.commit_author = format!(
                    "{} <{}>",
                    author.name.unwrap_or_default(),
                    author.email.unwrap_or_default()
                );
            }
            hub.reload();
            hub.notice = "Saved. New sessions use the updated configuration.".into();
        }
        KeyCode::Char('r') => {
            if hub.reset_pending {
                if hub.key() == "identity" {
                    if hub.draft["scope"] == "Folder" {
                        crate::git::clear_subtree_author(
                            hub.draft["folder"].as_str().unwrap_or_default(),
                        )?;
                    } else {
                        crate::git::clear_local_author()?;
                    }
                } else if hub.scope == Scope::Folder {
                    preferences::reset_folder(std::path::Path::new(&hub.folder), hub.key())?;
                } else {
                    preferences::reset_category(hub.scope, hub.key())?;
                }
                hub.reload();
            } else {
                hub.reset_pending = true;
                hub.notice = format!(
                    "Remove the {} override for {} and use inherited settings. Press r again to apply.",
                    hub.scope.label(),
                    hub.key()
                );
            }
        }
        KeyCode::Char('n') if hub.key() == "agents" => {
            let n = hub.draft.as_array().map_or(0, Vec::len) + 1;
            hub.draft
                .as_array_mut()
                .context("agents list")?
                .push(serde_json::to_value(preferences::Agent {
                    name: format!("Agent {n}"),
                    executable: "agent".into(),
                    ..preferences::Agent::default()
                })?);
        }
        KeyCode::Char('n') if hub.key() == "branches" => {
            let n = hub.draft["environments"].as_array().map_or(0, Vec::len) + 1;
            hub.draft["environments"]
                .as_array_mut()
                .context("environments")?
                .push(serde_json::to_value(preferences::Environment {
                    id: format!("env{n}"),
                    name: format!("Environment {n}"),
                    ..preferences::Environment::default()
                })?);
        }
        KeyCode::Char('p') if hub.key() == "branches" => hub.draft["promotions"]
            .as_array_mut()
            .context("promotions")?
            .push(serde_json::to_value(preferences::Promotion::default())?),
        KeyCode::Char('D') if hub.key() == "agents" || hub.key() == "branches" => {
            if let Some(field) = fields(hub).get(hub.selected) {
                let parts: Vec<_> = field.path.trim_start_matches('/').split('/').collect();
                if hub.key() == "agents" {
                    if let Some(i) = parts.first().and_then(|p| p.parse::<usize>().ok()) {
                        hub.draft.as_array_mut().unwrap().remove(i);
                    }
                } else if parts.len() >= 2
                    && ["environments", "promotions"].contains(&parts[0])
                    && let Ok(i) = parts[1].parse::<usize>()
                {
                    hub.draft[parts[0]].as_array_mut().unwrap().remove(i);
                }
            }
            hub.selected = 0;
        }
        KeyCode::Char('t') if hub.key() == "branches" => {
            hub.draft = serde_json::to_value(preferences::Branches {
                environments: vec![],
                promotions: vec![],
                protected: vec!["main".into()],
                ..preferences::Branches::default()
            })?
        }
        KeyCode::Char('b') if hub.key() == "branches" => {
            hub.draft = serde_json::to_value(preferences::Branches::default())?
        }
        KeyCode::Char('i') if hub.key() == "sandbox" => {
            crate::terrarium::initialize_current(hub.draft["preset"].as_str().unwrap_or("none"))?;
            hub.notice =
                "Created private profile. Press v to validate or e to inspect permissions.".into();
        }
        KeyCode::Char('e') if hub.key() == "sandbox" => {
            hub.draft = json!({"profile":crate::terrarium::read_profile()?});
            hub.original = hub.draft.clone();
            hub.selected = 0;
        }
        KeyCode::Char('v') if hub.key() == "sandbox" => {
            ::terrarium::validate(std::path::Path::new(&crate::git::repo_root()?))?;
            hub.notice = "Profile validated by the bundled Seatbelt backend.".into();
        }
        KeyCode::Char('v') if hub.key() == "agents" => {
            let profiles = serde_json::from_value::<Vec<preferences::Agent>>(hub.draft.clone())?;
            let (tx, rx) = std::sync::mpsc::channel();
            hub.diagnostics = Some(rx);
            hub.notice = "Checking agent versions…".into();
            std::thread::spawn(move || {
                let _ = tx.send(
                    profiles
                        .iter()
                        .map(crate::agents::diagnose)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            });
        }
        KeyCode::Char('R' | 'x') if hub.key() == "sessions" => {
            let row = fields(hub)
                .get(hub.selected)
                .and_then(|f| f.path.trim_start_matches('/').split('/').next())
                .and_then(|s| s.parse::<usize>().ok());
            if let Some(row) = row {
                let id_text = hub.draft[row]["id"].as_str().unwrap_or_default();
                let id = state
                    .sessions
                    .iter()
                    .find(|s| s.id.to_string() == id_text)
                    .map(|s| s.id);
                if let Some(id) = id {
                    if key.code == KeyCode::Char('R') {
                        state.sessions.restart(id)?;
                    } else {
                        state.sessions.close(id);
                    }
                    open(state, 8);
                }
            }
        }
        KeyCode::Char('v') if hub.key() == "writing" => {
            let writing: preferences::Writing = serde_json::from_value(hub.draft.clone())?;
            let mut settings = crate::settings::load();
            settings.pr_language = writing.language;
            settings.comment_style = writing.comment_style;
            settings.commit_subject_max_chars = writing.subject_max;
            settings.commit_body_max_lines = writing.body_lines;
            settings.commit_prompt = writing.commit_prompt;
            hub.notice = crate::settings::commit_prompt_prefix(&settings);
        }
        _ => {}
    }
    Ok(())
}

pub fn poll(state: &mut AppState) {
    if let Some(message) = state
        .settings_hub
        .diagnostics
        .as_ref()
        .and_then(|rx| rx.try_recv().ok())
    {
        state.settings_hub.notice = message;
        state.settings_hub.diagnostics = None;
    }
}

fn insert(hub: &mut Settings, text: &str) {
    let at = hub
        .input
        .char_indices()
        .nth(hub.cursor)
        .map_or(hub.input.len(), |(i, _)| i);
    hub.input.insert_str(at, text);
    hub.cursor += text.chars().count();
}
fn cursor_position(text: &str, cursor: usize, width: usize) -> (usize, usize) {
    let mut row = 0;
    let mut col = 0;
    for c in text.chars().take(cursor) {
        if c == '\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
            if col >= width {
                row += 1;
                col = 0;
            }
        }
    }
    (row, col)
}
pub fn handle_paste(state: &mut AppState, text: &str) -> bool {
    if state.modal != Modal::Settings || !state.settings_hub.editing {
        return false;
    }
    insert(&mut state.settings_hub, text);
    true
}
