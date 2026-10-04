//! Keys, mouse and pastes into the settings screen.

use super::*;
use crate::panel::text_input::Edit;
use crate::state::{Poll, poll_once, stopped_unexpectedly};

/// Clicks choose categories and fields, a second click on the selected field
/// opens it, and the wheel moves whichever pane it is over.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) {
    let r = regions(area);
    let at = Position::new(m.column, m.row);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if r.categories.contains(at) => {
            let index = usize::from(m.row - r.categories.y);
            if index < CATEGORIES.len() {
                state.settings_hub.focus = Focus::Categories;
                if index != state.settings_hub.category {
                    switch_category(state, index);
                }
            }
        }
        MouseEventKind::Down(MouseButton::Left) if r.fields.contains(at) => {
            let hub = &mut state.settings_hub;
            if hub.editing || hub.current_category() == Category::Activity {
                return;
            }
            let fields = fields(hub);
            let rows = rows(&fields);
            let lines = list_lines(
                hub,
                &fields,
                &rows,
                r.fields.width.saturating_sub(2) as usize,
            );
            let index = hub.scroll.get() + usize::from(m.row - r.fields.y);
            if let Some((_, Some(field))) = lines.get(index) {
                hub.focus = Focus::Fields;
                if *field == hub.selected {
                    start_edit(hub);
                } else {
                    hub.selected = *field;
                }
            }
        }
        MouseEventKind::ScrollDown if r.categories.contains(at) => {
            let next = state.settings_hub.category + 1;
            switch_category(state, next);
        }
        MouseEventKind::ScrollUp if r.categories.contains(at) => {
            let next = state.settings_hub.category + CATEGORIES.len() - 1;
            switch_category(state, next);
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            if (r.fields.contains(at) || r.detail.contains(at)) && !state.settings_hub.editing =>
        {
            move_selection(state, m.kind == MouseEventKind::ScrollDown, 1);
        }
        _ => {}
    }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let result = handle(state, key);
    if let Err(e) = result {
        state
            .settings_hub
            .notify(NoticeKind::Error, format!("{e:#}"));
    }
    Ok(())
}
pub(super) fn handle(state: &mut AppState, key: KeyEvent) -> Result<()> {
    state.settings_hub.branches = local_branches(state);
    let models = crate::llm::available_models();
    if !models.is_empty() {
        state.settings_hub.models = models;
    }
    let claude_models = crate::llm::served_claude_models();
    if !claude_models.is_empty() {
        state.settings_hub.claude_models = claude_models;
    }
    let hub = &mut state.settings_hub;
    // An error is about the key that caused it; the next key leaves its
    // text up but no longer in red.
    if hub.notice_kind == NoticeKind::Error {
        hub.notice_kind = NoticeKind::Info;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Only the very next key can confirm stopping a busy session.
    let armed = hub.session_armed.take();
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
        let picking = picking(hub);
        match key.code {
            KeyCode::Esc => hub.editing = false,
            KeyCode::Down | KeyCode::Up if picking => {
                move_choice(hub, key.code == KeyCode::Down);
                return Ok(());
            }
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown if stepping(hub) => {
                let size = match key.code {
                    KeyCode::PageUp | KeyCode::PageDown => 10,
                    _ if key.modifiers.contains(KeyModifiers::SHIFT) => 10,
                    _ => 1,
                };
                let up = matches!(key.code, KeyCode::Up | KeyCode::PageUp);
                step_number(hub, if up { size } else { -size });
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                hub.input.insert_char('\n');
            }
            KeyCode::Enter => commit_edit(hub)?,
            KeyCode::Char('u') if ctrl => {
                hub.input.clear();
                hub.choice = None;
                hub.typed = true;
            }
            // The carried-over branch name is a suggestion; typing replaces
            // it rather than appending to it.
            KeyCode::Char(_) if !ctrl && picking && !hub.typed => {
                hub.input.clear();
                hub.input.edit_key(key);
                hub.choice = None;
                hub.typed = true;
            }
            _ => {
                if hub.input.edit_key(key) == Edit::Changed {
                    hub.choice = None;
                    hub.typed = true;
                }
            }
        }
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            if hub.dirty() && !hub.discard_armed {
                hub.notify(
                    NoticeKind::Warning,
                    "Unsaved changes. Ctrl-S saves; Esc again discards.",
                );
                hub.discard_armed = true;
            } else {
                state.modal = Modal::None;
            }
        }
        KeyCode::Char('f') if hub.scope == Scope::Folder => {
            hub.input.set(hub.folder.clone());
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
        KeyCode::Tab | KeyCode::BackTab => {
            let next = hub.category
                + if key.code == KeyCode::Tab {
                    1
                } else {
                    CATEGORIES.len() - 1
                };
            switch_category(state, next);
        }
        KeyCode::Char('s') if !ctrl => hub.notify(
            NoticeKind::Warning,
            "Save or discard edits before changing category or scope.",
        ),
        KeyCode::Char('/') => hub.searching = true,
        KeyCode::Left => hub.focus = Focus::Categories,
        KeyCode::Right => hub.focus = Focus::Fields,
        KeyCode::Down | KeyCode::Char('j') if hub.focus == Focus::Categories => {
            let next = hub.category + 1;
            switch_category(state, next);
        }
        KeyCode::Up | KeyCode::Char('k') if hub.focus == Focus::Categories => {
            let next = hub.category + CATEGORIES.len() - 1;
            switch_category(state, next);
        }
        KeyCode::Enter if hub.focus == Focus::Categories => hub.focus = Focus::Fields,
        KeyCode::Down | KeyCode::Char('j') => move_selection(state, true, 1),
        KeyCode::Up | KeyCode::Char('k') => move_selection(state, false, 1),
        KeyCode::PageDown => move_selection(state, true, 10),
        KeyCode::PageUp => move_selection(state, false, 10),
        KeyCode::Home => hub.selected = 0,
        KeyCode::End => move_selection(state, true, usize::MAX / 2),
        KeyCode::Enter => start_edit(hub),
        KeyCode::Char('L')
            if hub.current_category() == Category::Writing
                || hub.current_category() == Category::Models =>
        {
            if hub.dirty() {
                hub.notify(
                    NoticeKind::Warning,
                    "Save or discard edits before opening the model modal.",
                );
            } else {
                crate::app::open_model_modal(state);
            }
        }
        KeyCode::Char('s') if ctrl => {
            if hub.editing {
                commit_edit(hub)?;
            }
            if hub.current_category() == Category::Sessions {
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
            } else if hub.current_category() == Category::Identity {
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
            } else if hub.current_category() == Category::Sandbox
                && hub.draft.get("profile").is_some()
            {
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
            state.ai_assist = preferences::ai_enabled();
            // The footer names the provider and model; a switch to claude
            // shows at once rather than after the next restart.
            state.llm_provider = crate::llm::current_provider();
            state.llm_model = crate::llm::current_model();
            // A new endpoint or provider deserves to be asked again.
            state.model_server_unreachable = false;
            if let Ok(author) = crate::git::author_config() {
                state.commit_author = format!(
                    "{} <{}>",
                    author.name.unwrap_or_default(),
                    author.email.unwrap_or_default()
                );
            }
            hub.reload();
            hub.notify(
                NoticeKind::Success,
                "Saved. New sessions use the updated configuration.",
            );
        }
        KeyCode::Char('r') => {
            if hub.reset_pending {
                if hub.current_category() == Category::Identity {
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
                let text = format!(
                    "Remove the {} override for {} and use inherited settings. Press r again to apply.",
                    hub.scope.label(),
                    hub.key()
                );
                hub.notify(NoticeKind::Warning, text);
            }
        }
        KeyCode::Char('n') if hub.current_category() == Category::Agents => {
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
        KeyCode::Char('n') if hub.current_category() == Category::Branches => {
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
        KeyCode::Char('p') if hub.current_category() == Category::Branches => {
            hub.draft["promotions"]
                .as_array_mut()
                .context("promotions")?
                .push(serde_json::to_value(preferences::Promotion::default())?)
        }
        KeyCode::Char('D')
            if hub.current_category() == Category::Agents
                || hub.current_category() == Category::Branches =>
        {
            if let Some(field) = fields(hub).get(hub.selected) {
                let parts: Vec<_> = field.path.trim_start_matches('/').split('/').collect();
                if hub.current_category() == Category::Agents {
                    if let Some(i) = parts.first().and_then(|p| p.parse::<usize>().ok())
                        && let Some(agents) = hub.draft.as_array_mut()
                        && i < agents.len()
                    {
                        agents.remove(i);
                    }
                } else if parts.len() >= 2
                    && ["environments", "promotions"].contains(&parts[0])
                    && let Ok(i) = parts[1].parse::<usize>()
                    && let Some(rows) = hub.draft[parts[0]].as_array_mut()
                    && i < rows.len()
                {
                    rows.remove(i);
                }
            }
            hub.selected = 0;
        }
        KeyCode::Char('t') if hub.current_category() == Category::Branches => {
            hub.draft = serde_json::to_value(preferences::Branches::default())?
        }
        KeyCode::Char('b') if hub.current_category() == Category::Branches => {
            hub.draft = serde_json::to_value(preferences::detect_branches(&hub.branches))?
        }
        KeyCode::Char('i') if hub.current_category() == Category::Sandbox => {
            crate::terrarium::initialize_current(hub.draft["preset"].as_str().unwrap_or("none"))?;
            hub.notify(
                NoticeKind::Info,
                "Created private profile. Press v to validate or e to inspect permissions.",
            );
        }
        KeyCode::Char('e') if hub.current_category() == Category::Sandbox => {
            hub.draft = json!({"profile":crate::terrarium::read_profile()?});
            hub.original = hub.draft.clone();
            hub.selected = 0;
        }
        KeyCode::Char('v') if hub.current_category() == Category::Sandbox => {
            ::terrarium::validate(std::path::Path::new(&crate::git::repo_root()?))?;
            hub.notify(
                NoticeKind::Success,
                "Profile validated by the bundled Seatbelt backend.",
            );
        }
        KeyCode::Char('v') if hub.current_category() == Category::Agents => {
            let profiles = serde_json::from_value::<Vec<preferences::Agent>>(hub.draft.clone())?;
            hub.diagnostics = Some(diagnose_agents(profiles));
            hub.notify(NoticeKind::Info, "Checking agent versions…");
        }
        KeyCode::Char('R' | 'x') if hub.current_category() == Category::Sessions => {
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
                    let pressed = if key.code == KeyCode::Char('R') {
                        'R'
                    } else {
                        'x'
                    };
                    // Stopping an agent mid-turn, or a shell mid-command,
                    // loses work: the first press says so, the second does it.
                    let busy = state.sessions.get(id).is_some_and(|session| {
                        session.is_running()
                            && (session.kind.is_agent()
                                || session.activity() != crate::session::SessionActivity::Idle)
                    });
                    if busy && armed != Some((pressed, id)) {
                        let verb = if pressed == 'R' { "restarts" } else { "stops" };
                        state.settings_hub.session_armed = Some((pressed, id));
                        state.settings_hub.notify(
                            NoticeKind::Warning,
                            format!(
                                "This session is still running. {pressed} again {verb} it; any other key keeps it."
                            ),
                        );
                        return Ok(());
                    }
                    if pressed == 'R' {
                        state.sessions.restart(id)?;
                    } else {
                        state.sessions.close(id);
                    }
                    open(state, Category::Sessions);
                }
            }
        }
        KeyCode::Char('v') if hub.current_category() == Category::Writing => {
            let writing: preferences::Writing = serde_json::from_value(hub.draft.clone())?;
            hub.notify(
                NoticeKind::Info,
                crate::settings::commit_prompt_prefix(&writing.into()),
            );
        }
        _ => {}
    }
    Ok(())
}

/// Check every agent's executable on a worker of its own; the report is one
/// line per agent.
fn diagnose_agents(profiles: Vec<preferences::Agent>) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(
            profiles
                .iter()
                .map(crate::agents::diagnose)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    });
    rx
}

pub fn poll(state: &mut AppState) {
    let hub = &mut state.settings_hub;
    let Some(rx) = hub.diagnostics.as_ref() else {
        return;
    };
    match poll_once(rx) {
        Poll::Pending => return,
        Poll::Message(report) => hub.notify(NoticeKind::Info, report),
        Poll::Disconnected => hub.notify(
            NoticeKind::Error,
            stopped_unexpectedly("the agent version check"),
        ),
    }
    hub.diagnostics = None;
}

pub(super) fn cursor_position(text: &str, cursor: usize, width: usize) -> (usize, usize) {
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
    let hub = &mut state.settings_hub;
    if picking(hub) && !hub.typed {
        hub.input.clear();
    }
    hub.input.insert_str(text);
    hub.choice = None;
    hub.typed = true;
    true
}
