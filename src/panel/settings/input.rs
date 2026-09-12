//! Keys, mouse and pastes into the settings screen.

use super::*;

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
            if hub.editing || hub.key() == "activity" {
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
        state.settings_hub.notice = format!("{e:#}");
        state.settings_hub.notice_error = true;
    }
    Ok(())
}
pub(super) fn handle(state: &mut AppState, key: KeyEvent) -> Result<()> {
    state.settings_hub.branches = local_branches(state);
    let models = crate::llm::available_models();
    if !models.is_empty() {
        state.settings_hub.models = models;
    }
    let hub = &mut state.settings_hub;
    hub.notice_error = false;
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
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => insert(hub, "\n"),
            KeyCode::Enter => commit_edit(hub)?,
            KeyCode::Backspace if hub.cursor > 0 => {
                let mut chars: Vec<_> = hub.input.chars().collect();
                chars.remove(hub.cursor - 1);
                hub.cursor -= 1;
                hub.input = chars.into_iter().collect();
                hub.choice = None;
                hub.typed = true;
            }
            KeyCode::Delete if hub.cursor < hub.input.chars().count() => {
                let mut chars: Vec<_> = hub.input.chars().collect();
                chars.remove(hub.cursor);
                hub.input = chars.into_iter().collect();
                hub.choice = None;
                hub.typed = true;
            }
            KeyCode::Left => hub.cursor = hub.cursor.saturating_sub(1),
            KeyCode::Right => hub.cursor = (hub.cursor + 1).min(hub.input.chars().count()),
            KeyCode::Home => hub.cursor = 0,
            KeyCode::End => hub.cursor = hub.input.chars().count(),
            KeyCode::Char('u') if ctrl => {
                hub.input.clear();
                hub.cursor = 0;
                hub.choice = None;
                hub.typed = true;
            }
            KeyCode::Char(c) if !ctrl => {
                // The carried-over branch name is a suggestion; typing
                // replaces it rather than appending to it.
                if picking && !hub.typed {
                    hub.input.clear();
                    hub.cursor = 0;
                }
                insert(hub, &c.to_string());
                hub.choice = None;
                hub.typed = true;
            }
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
        KeyCode::Tab | KeyCode::BackTab => {
            let next = hub.category
                + if key.code == KeyCode::Tab {
                    1
                } else {
                    CATEGORIES.len() - 1
                };
            switch_category(state, next);
        }
        KeyCode::Char('s') if !ctrl => {
            hub.notice = "Save or discard edits before changing category or scope.".into()
        }
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
        KeyCode::Char('L') if hub.key() == "writing" || hub.key() == "models" => {
            if hub.dirty() {
                hub.notice = "Save or discard edits before opening the model modal.".into();
            } else {
                crate::app::open_model_modal(state);
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
        KeyCode::Char('t') if hub.key() == "branches" => {
            hub.draft = serde_json::to_value(preferences::Branches::default())?
        }
        KeyCode::Char('b') if hub.key() == "branches" => {
            hub.draft = serde_json::to_value(preferences::detect_branches(&hub.branches))?
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

pub(super) fn insert(hub: &mut Settings, text: &str) {
    let at = hub
        .input
        .char_indices()
        .nth(hub.cursor)
        .map_or(hub.input.len(), |(i, _)| i);
    hub.input.insert_str(at, text);
    hub.cursor += text.chars().count();
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
        hub.cursor = 0;
    }
    insert(hub, text);
    hub.choice = None;
    hub.typed = true;
    true
}
