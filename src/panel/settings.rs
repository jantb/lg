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
const HINT_KEY: Color = palette::LANE_TEST;
const OK: Color = palette::LANE_FEATURE;
const BAD: Color = palette::LANE_LOST;

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
    notice_error: bool,
    pub source: String,
    pub query: String,
    searching: bool,
    reset_pending: bool,
    folder: String,
    editing_folder: bool,
    cursor: usize,
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
fn start_edit(hub: &mut Settings) {
    if let Some(f) = fields(hub).get(hub.selected) {
        hub.input = display(&f.value);
        hub.cursor = hub.input.chars().count();
        hub.editing = true;
    }
}

/// One editable leaf of the category's value tree.
#[derive(Clone)]
struct Field {
    path: String,
    /// The headings above the leaf: `["environments", "#1 Development"]`.
    groups: Vec<String>,
    /// The leaf's own key as written in the file.
    key: String,
    /// Everything a search may match on.
    label: String,
    value: Value,
}
/// What an item in a list of objects is called on its heading: its name or
/// id if it has one, or the move a promotion makes.
fn summary(item: &Value) -> String {
    for key in ["name", "label", "id"] {
        if let Some(text) = item.get(key).and_then(Value::as_str)
            && !text.is_empty()
        {
            return text.into();
        }
    }
    match (
        item.get("from").and_then(Value::as_str),
        item.get("to").and_then(Value::as_str),
    ) {
        (Some(from), Some(to)) => format!("{from} \u{2192} {to}"),
        _ => String::new(),
    }
}
fn humanize(key: &str) -> String {
    key.replace('_', " ")
}
fn flatten(value: &Value, path: &str, groups: &[String], out: &mut Vec<Field>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                let path = format!("{path}/{key}");
                let nested = v.is_object()
                    || v.as_array()
                        .is_some_and(|items| items.iter().any(Value::is_object));
                if nested {
                    let mut groups = groups.to_vec();
                    groups.push(humanize(key));
                    flatten(v, &path, &groups, out);
                } else {
                    let label = groups
                        .iter()
                        .map(String::as_str)
                        .chain([humanize(key).as_str()])
                        .collect::<Vec<_>>()
                        .join(" \u{b7} ");
                    out.push(Field {
                        path,
                        groups: groups.to_vec(),
                        key: key.clone(),
                        label,
                        value: v.clone(),
                    });
                }
            }
        }
        Value::Array(items) if items.iter().any(Value::is_object) => {
            for (i, v) in items.iter().enumerate() {
                let mut groups = groups.to_vec();
                groups.push(format!("#{} {}", i + 1, summary(v)).trim_end().to_string());
                flatten(v, &format!("{path}/{i}"), &groups, out);
            }
        }
        _ => out.push(Field {
            path: path.into(),
            groups: groups.to_vec(),
            key: path.rsplit('/').next().unwrap_or_default().into(),
            label: groups.join(" \u{b7} "),
            value: value.clone(),
        }),
    }
}
fn fields(hub: &Settings) -> Vec<Field> {
    let mut fields = Vec::new();
    flatten(&hub.draft, "", &[], &mut fields);
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

/// A row of the field list: a heading over a run of nested fields, or a
/// field. Only fields can be selected; the arrow keys skip the headings.
enum Row {
    Header { label: String, depth: usize },
    Field { index: usize, depth: usize },
}
fn rows(fields: &[Field]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut open: &[String] = &[];
    for (index, field) in fields.iter().enumerate() {
        let shared = open
            .iter()
            .zip(&field.groups)
            .take_while(|(a, b)| a == b)
            .count();
        for (depth, label) in field.groups.iter().enumerate().skip(shared) {
            rows.push(Row::Header {
                label: label.clone(),
                depth,
            });
        }
        rows.push(Row::Field {
            index,
            depth: field.groups.len(),
        });
        open = &field.groups;
    }
    rows
}

/// What a field means, in a sentence, for the pane under the list.
fn describe(category: &str, field: &Field) -> Option<&'static str> {
    let group = field.groups.first().map(String::as_str).unwrap_or_default();
    Some(match (category, group, field.key.as_str()) {
        ("identity", _, "scope") => {
            "Repository writes the author into this checkout's .git/config. Folder writes a Git includeIf for every repository below the folder."
        }
        ("identity", _, "folder") => {
            "Where the Folder scope applies. Every repository under this path gets the author below."
        }
        ("identity", _, "name") => {
            "Author name for commits made from lg. Signing keys stay in Git's own configuration."
        }
        ("identity", _, "email") => {
            "Author email for commits made from lg. Matches the quiet author emails list in Interface & Tools, which hides your own commits from activity."
        }
        ("writing", _, "language") => {
            "Language commit messages and pull request text are written in."
        }
        ("writing", _, "comment_style") => {
            "The shape of a commit message: Conventional Commits, plain imperative, and so on. Press L to derive it from this checkout's history."
        }
        ("writing", _, "subject_max") => {
            "Longest subject line the commit writer may produce, in characters."
        }
        ("writing", _, "body_lines") => "Most lines the commit body may run to.",
        ("writing", _, "commit_prompt") => {
            "The instructions given to the model before the staged diff. Enter opens the whole text in the editor; v previews the assembled prompt."
        }
        ("writing", _, "review_style") => "House rules the reviewer checks changes against.",
        ("models", _, "model") => {
            "Model used for commit messages, reviews and summaries. L opens the model picker with connectivity checks."
        }
        ("models", _, "endpoint") => "Chat completions endpoint the model is reached at.",
        ("agents", _, "name") => "How the agent is listed in the session picker.",
        ("agents", _, "adapter") => {
            "Which integration drives it: claude, codex, pi, or terminal for a plain shell with no agent protocol."
        }
        ("agents", _, "executable") => "Program to launch, found on PATH or given as a full path.",
        ("agents", _, "args") => {
            "Extra arguments, as a JSON list: [\"--flag\", \"value\"]. Passed directly, without a shell."
        }
        ("agents", _, "model") => {
            "Model the agent is asked to use. Empty leaves the agent's own default."
        }
        ("agents", _, "confinement") => {
            "terrarium runs it in the sandbox profile, agent trusts the agent's own sandbox, direct runs it unconfined."
        }
        ("agents", _, "default") => {
            "The agent a new session starts with when none is chosen. One agent should be true."
        }
        ("sandbox", _, "preset") => {
            "Terrarium preset the private sandbox profile is generated from; i creates it, v validates it, e opens it."
        }
        ("sandbox", _, "profile") => "The Seatbelt profile itself. Ctrl-S writes it back.",
        ("branches", _, "base") => {
            "The trunk: feature branches start here and merge back here. The deployment block measures every environment against it."
        }
        ("branches", _, "protected") => {
            "Branches lg will not delete or force-push, as a JSON list."
        }
        ("branches", "environments", "id") => {
            "Key the rest of lg refers to the environment by. Release actions and the deployment block are wired for dev and test; other ids are stored but not shown there yet."
        }
        ("branches", "environments", "name") => {
            "Label shown in the deployment block and the branch action menu."
        }
        ("branches", "environments", "branch") => {
            "The branch that deploys this environment, for example develop or test. Empty means lg cannot tell what is deployed and hides the environment."
        }
        ("branches", "environments", "remote") => {
            "Remote whose copy of the branch is the deployed one."
        }
        ("branches", _, "remote") => {
            "Remote whose copies of the deploy branches count as released, so a stale local checkout does not report a release that never landed."
        }
        ("branches", "environments", "url") => "Where the environment runs. Informational.",
        ("branches", "promotions", "from") => {
            "Source of the promotion: an environment id, or the word feature for the branch you are on."
        }
        ("branches", "promotions", "to") => "Environment id the source is promoted into.",
        ("branches", "promotions", "strategy") => {
            "How the promotion lands: merge (a merge commit), squash (one commit), or ff-only (refuse unless fast-forward)."
        }
        ("branches", "promotions", "push") => {
            "Push the environment branch to its remote after promoting."
        }
        ("tools", _, "decorative_animations") => {
            "Pulsing frames and travelling light. false holds every frame still."
        }
        ("tools", _, "quiet_author_emails") => {
            "Authors whose commits are not announced in activity, as a JSON list; * matches any prefix: [\"*@client.com\"]."
        }
        ("tools", _, "editor") => "Program opened for a file with e. Empty uses $EDITOR.",
        ("tools", _, "editor_args") => {
            "Arguments for the editor, as a JSON list. The file path is appended."
        }
        ("tools", _, "terminal") => "Terminal program used when a session is opened outside lg.",
        ("tools", _, "terminal_args") => "Arguments for that terminal, as a JSON list.",
        ("sessions", _, "label") => {
            "Name shown for the session in the workspace tree. Ctrl-S renames it."
        }
        ("sessions", _, "path") => "Working directory the session runs in.",
        ("sessions", _, "kind") => "What is running: an agent or a plain terminal.",
        ("sessions", _, "id") => "Internal identifier.",
        _ => return None,
    })
}

/// Where everything is, measured from the screen so the drawing and the mouse
/// agree on it.
struct Regions {
    modal: Rect,
    header: Rect,
    categories: Rect,
    fields: Rect,
    detail: Rect,
    footer: Rect,
    dividers: Vec<Rect>,
}
fn regions(area: Rect) -> Regions {
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

fn muted(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(MUTED))
}
fn accent(text: impl Into<String>) -> Span<'static> {
    Span::styled(
        text.into(),
        Style::default()
            .fg(palette::ACCENT)
            .add_modifier(Modifier::BOLD),
    )
}
/// A value coloured by what it is, so a glance tells a flag from a number
/// from a list.
fn value_span(value: &Value) -> Span<'static> {
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
fn field_lines(hub: &Settings, field: &Field, depth: usize, width: usize) -> Vec<Line<'static>> {
    let indent = "  ".repeat(depth);
    let key = format!("{}{}: ", indent, humanize(&field.key));
    let value = value_span(&field.value);
    let room = width.saturating_sub(key.chars().count()).max(8);
    let text = value.content.to_string();
    let mut chunks = if text.chars().count() <= room {
        vec![text]
    } else {
        super::wrap_words(&text, room)
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
    lines
}
/// The list drawn as lines, each tagged with the row it belongs to. A row may
/// take several lines once its value wraps.
fn list_lines(
    hub: &Settings,
    fields: &[Field],
    rows: &[Row],
    width: usize,
) -> Vec<(Line<'static>, Option<usize>)> {
    let mut lines = Vec::new();
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
fn scroll_to(first: usize, last: usize, total: usize, height: usize, current: usize) -> usize {
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

fn render_header(hub: &Settings, area: Rect, frame: &mut Frame) {
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
            "Click a category or a field \u{b7} Tab switches category \u{b7} / filters fields",
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

fn render_categories(hub: &Settings, area: Rect, frame: &mut Frame) {
    let lines: Vec<Line> = CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, (_, name))| {
            if i == hub.category {
                Line::from(vec![accent("\u{203a} "), accent(*name)]).style(palette::selection())
            } else {
                Line::from(vec![Span::raw("  "), muted(*name)])
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_fields(hub: &Settings, area: Rect, frame: &mut Frame) {
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
            if is_selected {
                line.style(palette::selection())
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn render_editor(hub: &Settings, area: Rect, frame: &mut Frame) {
    let title = if hub.editing_folder {
        "Editing \u{b7} folder".to_string()
    } else {
        fields(hub)
            .get(hub.selected)
            .map(|f| format!("Editing \u{b7} {}", f.label))
            .unwrap_or_else(|| "Editing".into())
    };
    ui::section_title(frame, area, &title);
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

fn render_detail(hub: &Settings, area: Rect, frame: &mut Frame) {
    ui::section_title(frame, area, "About this field");
    let fields = fields(hub);
    let Some(field) = fields.get(hub.selected.min(fields.len().saturating_sub(1))) else {
        let text = if hub.key() == "activity" {
            "What lg found on this machine, and the most recent status messages."
        } else {
            "Select a field to see what it does."
        };
        frame.render_widget(Paragraph::new(Line::from(muted(text))), area);
        return;
    };
    let kind = match field.value {
        Value::Bool(_) => "true or false",
        Value::Number(_) => "a whole number",
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

fn render_activity(state: &AppState, area: Rect, frame: &mut Frame) {
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
fn category_keys(category: &str) -> (&'static [(&'static str, &'static str)], &'static str) {
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
                ("b", "environment template"),
            ],
            "Environments with id dev or test drive the release actions and the deployment block. An environment with an empty branch is hidden.",
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
fn hint_line(keys: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, what)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(muted("  "));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(HINT_KEY).add_modifier(Modifier::BOLD),
        ));
        spans.push(muted(format!(" {what}")));
    }
    Line::from(spans)
}
fn render_footer(hub: &Settings, area: Rect, frame: &mut Frame) {
    let mut lines = Vec::new();
    if hub.editing {
        lines.push(hint_line(&[
            ("Enter", "apply"),
            ("Shift-Enter", "newline"),
            ("Ctrl-U", "clear"),
            ("Ctrl-S", "apply and save"),
            ("Esc", "cancel"),
        ]));
    } else {
        let (extra, note) = category_keys(hub.key());
        let mut keys = vec![
            ("Tab", "category"),
            ("j/k", "field"),
            ("Enter", "edit"),
            ("Ctrl-S", "save"),
            ("s", "scope"),
            ("f", "folder"),
            ("/", "filter"),
            ("r", "reset override"),
        ];
        keys.extend_from_slice(extra);
        lines.push(hint_line(&keys));
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

/// Clicks choose categories and fields, a second click on the selected field
/// opens it, and the wheel moves whichever pane it is over.
pub fn handle_mouse(state: &mut AppState, area: Rect, m: &MouseEvent) {
    let r = regions(area);
    let at = Position::new(m.column, m.row);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if r.categories.contains(at) => {
            let index = usize::from(m.row - r.categories.y);
            if index < CATEGORIES.len() && index != state.settings_hub.category {
                switch_category(state, index);
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
fn handle(state: &mut AppState, key: KeyEvent) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    /// A hub over the built-in branches configuration, with no file read.
    fn branches_hub() -> AppState {
        let mut state = AppState::default();
        state.decorative_animations = false;
        state.settings_hub = Settings {
            category: 5,
            draft: serde_json::to_value(preferences::Branches::default()).unwrap(),
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
        let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
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
        let area = Rect::new(0, 0, 120, 60);
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
