//! Drawing the settings screen: categories, fields, the editor, the detail pane and the footer.

use super::*;

/// Where everything is, measured from the screen so the drawing and the mouse
/// agree on it.
pub(super) struct Regions {
    pub(super) modal: Rect,
    pub(super) header: Rect,
    pub(super) categories: Rect,
    pub(super) fields: Rect,
    pub(super) detail: Rect,
    pub(super) footer: Rect,
    pub(super) dividers: Vec<Rect>,
}
pub(super) fn regions(area: Rect) -> Regions {
    let modal = ui::centered(
        area,
        area.width.saturating_sub(4).min(130),
        area.height.saturating_sub(2),
    );
    let inner = ui::modal_inner(modal);
    let (bands, mut dividers) = ui::modal_row_areas(
        inner,
        &[
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(5),
        ],
    );
    let (columns, gaps) = ui::modal_column_areas(
        bands[1],
        &[Constraint::Length(CATEGORY_WIDTH), Constraint::Min(30)],
    );
    dividers.extend(gaps);
    let (right, gaps) = ui::modal_row_areas(
        columns[1],
        &[Constraint::Min(3), Constraint::Length(DETAIL_HEIGHT)],
    );
    dividers.extend(gaps);
    Regions {
        modal,
        header: bands[0],
        categories: columns[0],
        fields: right[0],
        detail: right[1],
        footer: bands[2],
        dividers,
    }
}

pub(super) fn muted(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(MUTED))
}
pub(super) fn accent(text: impl Into<String>) -> Span<'static> {
    Span::styled(
        text.into(),
        Style::default()
            .fg(palette::ACCENT)
            .add_modifier(Modifier::BOLD),
    )
}
/// A value coloured by what it is, so a glance tells a flag from a number
/// from a list.
pub(super) fn value_span(value: &Value) -> Span<'static> {
    match value {
        Value::Bool(true) => Span::styled("true", Style::default().fg(OK)),
        Value::Bool(false) => Span::styled("false", Style::default().fg(BAD)),
        Value::Number(n) => Span::styled(n.to_string(), Style::default().fg(palette::LANE_TEST)),
        Value::Null => Span::styled(
            "unset",
            Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
        ),
        Value::String(s) if s.is_empty() => Span::styled(
            "empty",
            Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
        ),
        Value::String(s) => Span::raw(s.replace('\n', " \u{21b5} ")),
        other => Span::styled(other.to_string(), Style::default().fg(palette::LANE_MAIN)),
    }
}
/// A field's row, wrapped to `width` and cut after a few lines: the list is
/// for finding a field, the editor and the detail pane for reading it.
pub(super) fn field_lines(
    hub: &Settings,
    field: &Field,
    depth: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let indent = "  ".repeat(depth);
    let key = format!("{}{}: ", indent, humanize(&field.key));
    let value = value_span(&field.value);
    let room = width.saturating_sub(key.chars().count()).max(8);
    let text = value.content.to_string();
    let mut chunks = if text.chars().count() <= room {
        vec![text]
    } else {
        crate::panel::wrap_words(&text, room)
    };
    if chunks.len() > MAX_VALUE_LINES {
        chunks.truncate(MAX_VALUE_LINES);
        if let Some(last) = chunks.last_mut() {
            last.push('\u{2026}');
        }
    }
    let continuation = " ".repeat(key.chars().count());
    let mut lines = Vec::new();
    for (i, chunk) in chunks.into_iter().enumerate() {
        let mut spans = vec![Span::styled(
            if i == 0 {
                key.clone()
            } else {
                continuation.clone()
            },
            Style::default().fg(KEY_COLOR),
        )];
        spans.push(Span::styled(chunk, value.style));
        if i == 0 && hub.changed(&field.path) {
            spans.push(Span::styled(
                " \u{25cf} edited",
                Style::default().fg(palette::LANE_TEST),
            ));
        }
        lines.push(Line::from(spans));
    }
    if let Some(about) = describe(hub.key(), field) {
        let lead = format!("{indent}  ");
        let room = width.saturating_sub(lead.chars().count()).max(8);
        for chunk in crate::panel::wrap_words(about, room) {
            lines.push(Line::from(vec![Span::raw(lead.clone()), muted(chunk)]));
        }
    }
    lines
}
/// The list drawn as lines, each tagged with the row it belongs to. A row may
/// take several lines once its value wraps.
pub(super) fn list_lines(
    hub: &Settings,
    fields: &[Field],
    rows: &[Row],
    width: usize,
) -> Vec<(Line<'static>, Option<usize>)> {
    let mut lines = Vec::new();
    let about = describe_category(hub.key());
    if !about.is_empty() {
        for chunk in crate::panel::wrap_words(about, width.max(8)) {
            lines.push((Line::from(muted(chunk)), None));
        }
        lines.push((Line::from(""), None));
    }
    for row in rows {
        match row {
            Row::Header { label, depth } => {
                let color = GROUP_COLORS[(*depth).min(GROUP_COLORS.len() - 1)];
                lines.push((
                    Line::from(Span::styled(
                        format!("{}{label}", "  ".repeat(*depth)),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    )),
                    None,
                ));
            }
            Row::Field { index, depth } => {
                for line in field_lines(hub, &fields[*index], *depth, width) {
                    lines.push((line, Some(*index)));
                }
            }
        }
    }
    lines
}
/// Scrolls so the whole of the selected row is in view, moving no further
/// than it has to.
pub(super) fn scroll_to(
    first: usize,
    last: usize,
    total: usize,
    height: usize,
    current: usize,
) -> usize {
    if height == 0 {
        return 0;
    }
    let max = total.saturating_sub(height);
    let mut offset = current.min(max);
    if last >= offset + height {
        offset = last + 1 - height;
    }
    if first < offset {
        offset = first;
    }
    offset.min(max)
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let hub = &state.settings_hub;
    let r = regions(area);
    let title = if hub.dirty() {
        "Settings \u{b7} unsaved changes"
    } else {
        "Settings"
    };
    let block = ui::bordered(title).title_bottom(
        Line::from(Span::styled(
            if hub.editing {
                " Esc cancels the edit "
            } else {
                " Esc close "
            },
            Style::default().fg(MUTED),
        ))
        .alignment(Alignment::Right),
    );
    ui::modal_frame_with(frame, r.modal, block);
    ui::draw_dividers(frame, &r.dividers);

    render_header(hub, r.header, frame);
    render_categories(hub, r.categories, frame);
    ui::section_title(frame, r.categories, "Categories");
    if hub.key() == "activity" {
        render_activity(state, r.fields, frame);
        ui::section_title(frame, r.fields, "Setup & history");
    } else if hub.editing {
        render_editor(hub, r.fields, frame);
    } else {
        render_fields(hub, r.fields, frame);
    }
    render_detail(hub, r.detail, frame);
    render_footer(hub, r.footer, frame);
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, r.modal, &r.dividers, frame);
    }
}

pub(super) fn render_header(hub: &Settings, area: Rect, frame: &mut Frame) {
    let path = if hub.scope == Scope::Folder {
        hub.folder.clone()
    } else {
        preferences::scope_path(hub.scope)
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };
    let third = if hub.searching {
        Line::from(vec![
            Span::styled("Search  ", Style::default().fg(HINT_KEY)),
            Span::raw(hub.query.clone()),
            Span::styled("\u{258f}", Style::default().fg(palette::ACCENT)),
            muted("   Enter keeps the filter, Esc too; clear it with Backspace"),
        ])
    } else if !hub.query.is_empty() {
        Line::from(vec![
            Span::styled("Filter  ", Style::default().fg(HINT_KEY)),
            Span::raw(hub.query.clone()),
            muted("   / to change"),
        ])
    } else {
        Line::from(muted(
            "\u{2190}/\u{2192} switch pane \u{b7} Tab switches category \u{b7} / filters fields",
        ))
    };
    let lines = vec![
        Line::from(vec![
            muted("Scope   "),
            accent(hub.scope.label()),
            muted(format!("   {path}")),
        ]),
        Line::from(vec![muted("Source  "), Span::raw(hub.source.clone())]),
        third,
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_categories(hub: &Settings, area: Rect, frame: &mut Frame) {
    let lines: Vec<Line> = CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, (_, name))| {
            if i == hub.category && hub.focus == Focus::Categories {
                Line::from(vec![accent("\u{203a} "), accent(*name)]).style(palette::selection())
            } else if i == hub.category {
                Line::from(vec![accent("\u{203a} "), accent(*name)])
            } else {
                Line::from(vec![Span::raw("  "), muted(*name)])
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_fields(hub: &Settings, area: Rect, frame: &mut Frame) {
    let fields = fields(hub);
    let count = if hub.query.is_empty() {
        format!("Fields ({})", fields.len())
    } else {
        format!("Fields ({} matching)", fields.len())
    };
    ui::section_title(frame, area, &count);
    if fields.is_empty() {
        let text = if hub.query.is_empty() {
            "Nothing to edit here.".to_string()
        } else {
            format!("No field matches \u{201c}{}\u{201d}.", hub.query)
        };
        frame.render_widget(Paragraph::new(Line::from(muted(text))), area);
        return;
    }
    let selected = hub.selected.min(fields.len() - 1);
    let rows = rows(&fields);
    // Two columns for the marker in front of every line.
    let lines = list_lines(hub, &fields, &rows, area.width.saturating_sub(2) as usize);
    let first = lines
        .iter()
        .position(|(_, row)| *row == Some(selected))
        .unwrap_or(0);
    let last = lines
        .iter()
        .rposition(|(_, row)| *row == Some(selected))
        .unwrap_or(first);
    let offset = scroll_to(
        first,
        last,
        lines.len(),
        area.height as usize,
        hub.scroll.get(),
    );
    hub.scroll.set(offset);
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(offset)
        .take(area.height as usize)
        .enumerate()
        .map(|(i, (line, row))| {
            let is_selected = row == Some(selected);
            let marker = if is_selected && offset + i == first {
                "\u{203a} "
            } else {
                "  "
            };
            let mut spans = vec![Span::raw(marker)];
            spans.extend(line.spans);
            let line = Line::from(spans);
            if is_selected && hub.focus == Focus::Fields {
                line.style(palette::selection())
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(visible), area);
}

pub(super) fn render_editor(hub: &Settings, area: Rect, frame: &mut Frame) {
    let title = if hub.editing_folder {
        "Editing \u{b7} folder".to_string()
    } else {
        fields(hub)
            .get(hub.selected)
            .map(|f| format!("Editing \u{b7} {}", f.label))
            .unwrap_or_else(|| "Editing".into())
    };
    ui::section_title(frame, area, &title);
    if let Some(p) = picker(hub) {
        render_picker(hub, &p, area, frame);
        return;
    }
    let (row, col) = cursor_position(&hub.input, hub.cursor, area.width.max(1) as usize);
    let scroll = row.saturating_sub(area.height.saturating_sub(1) as usize);
    frame.render_widget(
        Paragraph::new(hub.input.as_str())
            .wrap(Wrap { trim: false })
            .scroll((scroll.min(u16::MAX as usize) as u16, 0)),
        area,
    );
    if area.width > 0 && area.height > 0 {
        frame.set_cursor_position((area.x + col as u16, area.y + (row - scroll) as u16));
    }
}

/// The typed name on the first line, and below it the options that contain
/// it, with the highlighted one applied by Enter.
pub(super) fn render_picker(hub: &Settings, picker: &Picker, area: Rect, frame: &mut Frame) {
    let noun = picker.noun;
    let choices = choices(hub);
    let hint = match (choices.is_empty(), picker.fixed) {
        (true, true) => format!("   nothing matches; one of {}", picker.options.join(", ")),
        (true, false) => format!("   no {noun} matches; Enter keeps the typed name"),
        (false, true) => format!("   \u{2191}/\u{2193} choose a {noun}, Enter applies it"),
        (false, false) => format!("   \u{2191}/\u{2193} pick a {noun}, or type your own"),
    };
    let mut lines = vec![Line::from(vec![
        Span::raw(hub.input.clone()),
        Span::styled("\u{258f}", Style::default().fg(palette::ACCENT)),
        muted(hint),
    ])];
    let room = area.height.saturating_sub(1) as usize;
    let first = hub
        .choice
        .map_or(0, |c| c.saturating_sub(room.saturating_sub(1)));
    for (i, name) in choices.iter().enumerate().skip(first).take(room) {
        let line = if hub.choice == Some(i) {
            Line::from(vec![accent("\u{203a} "), accent(name.clone())]).style(palette::selection())
        } else {
            Line::from(vec![Span::raw("  "), Span::raw(name.clone())])
        };
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(lines), area);
    if area.width > 0 && area.height > 0 {
        let col = hub.input.chars().count().min(area.width as usize - 1) as u16;
        frame.set_cursor_position((area.x + col, area.y));
    }
}

pub(super) fn render_detail(hub: &Settings, area: Rect, frame: &mut Frame) {
    ui::section_title(frame, area, "About this field");
    let fields = fields(hub);
    let Some(field) = fields.get(hub.selected.min(fields.len().saturating_sub(1))) else {
        let text = if hub.key() == "activity" {
            describe_category("activity")
        } else {
            "Select a field to see what it does."
        };
        frame.render_widget(Paragraph::new(Line::from(muted(text))), area);
        return;
    };
    let kind = match field.value {
        _ if names_branch(hub, field) => "a local branch; Enter opens the picker",
        _ if names_model(hub, field) => "a model the endpoint serves; Enter opens the picker",
        Value::Bool(_) => "true or false; Enter toggles",
        Value::Number(_) => "a whole number; \u{2191}/\u{2193} step it while editing",
        Value::String(_) => "text; Shift-Enter adds a line",
        Value::Array(_) => "a JSON list, for example [\"--wait\"]",
        _ => "JSON",
    };
    let mut lines = vec![Line::from(vec![
        accent(field.label.clone()),
        muted(format!("   {kind}")),
        if hub.changed(&field.path) {
            Span::styled(
                "   \u{25cf} edited, not saved",
                Style::default().fg(palette::LANE_TEST),
            )
        } else {
            Span::raw("")
        },
    ])];
    match describe(hub.key(), field) {
        Some(text) => lines.push(Line::from(Span::raw(text))),
        None => lines.push(Line::from(muted("Enter edits the value; r removes this scope's override and falls back to the inherited one."))),
    }
    if let Value::String(text) = &field.value
        && text.contains('\n')
    {
        lines.push(Line::from(muted(format!(
            "{} lines \u{b7} Enter opens the whole text",
            text.lines().count()
        ))));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

pub(super) fn render_activity(state: &AppState, area: Rect, frame: &mut Frame) {
    let hub = &state.settings_hub;
    let heading = |text: &str| Line::from(accent(text.to_string()));
    let mut lines = vec![
        heading("Setup"),
        Line::from(vec![
            muted("Author   "),
            Span::raw(format!(
                "{} <{}>",
                state.author_name_input, state.author_email_input
            )),
        ]),
    ];
    let loaded = preferences::load();
    for agent in &loaded.config.agents {
        lines.push(Line::from(vec![
            muted("Agent    "),
            Span::raw(crate::agents::describe(agent)),
        ]));
    }
    let sandbox = crate::terrarium::profile_path(std::path::Path::new(
        state.repo_root.as_deref().unwrap_or("."),
    ))
    .map(|p| {
        if p.exists() {
            Span::raw(p.display().to_string())
        } else {
            Span::styled(
                "not configured \u{2014} open Sandbox",
                Style::default().fg(BAD),
            )
        }
    })
    .unwrap_or_else(|| Span::raw(""));
    lines.push(Line::from(vec![muted("Sandbox  "), sandbox]));
    lines.push(Line::from(""));
    lines.push(heading("History (newest first)"));
    for status in state
        .status_history
        .iter()
        .rev()
        .skip(hub.selected)
        .take(100)
    {
        lines.push(Line::from(vec![
            muted(status.at.format("%H:%M:%S").to_string()),
            if status.is_error {
                Span::styled(" ERROR ", Style::default().fg(BAD))
            } else {
                Span::styled(" \u{2022} ", Style::default().fg(OK))
            },
            Span::raw(status.text.clone()),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// The keys that work in this category, beyond the ones every category has.
pub(super) fn category_keys(
    category: &str,
) -> (&'static [(&'static str, &'static str)], &'static str) {
    match category {
        "identity" => (
            &[],
            "Scope field: Repository or Folder. Folder affects Git repositories below that path. Signing remains inherited from Git.",
        ),
        "writing" => (
            &[("v", "preview prompt"), ("L", "derive style from history")],
            "L opens the model modal, which reads this checkout's commits to suggest language and message shape.",
        ),
        "models" => (
            &[("L", "model picker")],
            "L opens the model modal with the live model list and connectivity check.",
        ),
        "agents" => (
            &[
                ("n", "add agent"),
                ("D", "delete agent"),
                ("v", "check versions"),
            ],
            "adapter: claude, codex, pi or terminal \u{b7} confinement: terrarium, agent or direct",
        ),
        "sandbox" => (
            &[
                ("i", "create profile"),
                ("v", "validate"),
                ("e", "edit profile"),
            ],
            "Bundled Terrarium (macOS Seatbelt); the shared Git directory stays writable.",
        ),
        "branches" => (
            &[
                ("n", "add environment"),
                ("p", "add promotion"),
                ("D", "remove entry"),
                ("t", "trunk template"),
                ("b", "detect from branches"),
            ],
            "Nothing saved means the environments are read off the local branches: develop or dev, test, and main as production. Environments with id dev or test drive the release actions. An environment with an empty branch is hidden.",
        ),
        "tools" => (&[], "Folder scope applies below the folder chosen with f."),
        "activity" => (
            &[("j/k", "scroll history")],
            "Agent availability does not establish authentication; model connectivity is checked when a request runs.",
        ),
        "sessions" => (
            &[("R", "restart session"), ("x", "close session")],
            "Edit a label and Ctrl-S to rename. Restart reuses the original launch command.",
        ),
        _ => (&[], ""),
    }
}
pub(super) fn render_footer(hub: &Settings, area: Rect, frame: &mut Frame) {
    let mut lines = Vec::new();
    if hub.editing && picking(hub) {
        lines.push(ui::key_hints(&[
            ("\u{2191}/\u{2193}", "pick"),
            ("Enter", "apply"),
            ("Ctrl-U", "clear"),
            ("Ctrl-S", "apply and save"),
            ("Esc", "cancel"),
        ]));
    } else if hub.editing {
        lines.push(ui::key_hints(&[
            ("Enter", "apply"),
            ("Shift-Enter", "newline"),
            ("Ctrl-U", "clear"),
            ("Ctrl-S", "apply and save"),
            ("Esc", "cancel"),
        ]));
    } else {
        let (extra, note) = category_keys(hub.key());
        let mut keys = vec![
            ("\u{2190}/\u{2192}", "pane"),
            ("j/k", "move"),
            ("Enter", "edit / toggle"),
            ("Ctrl-S", "save"),
            ("s", "scope"),
            ("f", "folder"),
            ("/", "filter"),
            ("r", "reset override"),
        ];
        keys.extend_from_slice(extra);
        lines.push(ui::key_hints(&keys));
        if !note.is_empty() {
            lines.push(Line::from(muted(note)));
        }
    }
    if !hub.notice.is_empty() {
        let style = if hub.notice_error {
            Style::default().fg(BAD).add_modifier(Modifier::BOLD)
        } else if hub.notice.starts_with("Saved") || hub.notice.starts_with("Profile validated") {
            Style::default().fg(OK)
        } else if hub.reset_pending
            || hub.notice.starts_with("Unsaved")
            || hub.notice.starts_with("Save or")
        {
            Style::default().fg(palette::LANE_TEST)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(hub.notice.clone(), style)));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}
