//! The flat list of editable fields a category shows, and how a value is picked, stepped and committed.

use super::*;

/// What a field is chosen from.
pub(super) struct Picker {
    pub(super) options: Vec<String>,
    /// What one option is called in the hints: "local branch", "strategy".
    pub(super) noun: &'static str,
    /// Whether the value must be one of the options. An enumeration is; a
    /// branch or a program is a suggestion, and a typed name is kept.
    pub(super) fixed: bool,
}
impl Picker {
    pub(super) fn fixed(options: &[&str], noun: &'static str) -> Self {
        Self {
            options: options.iter().map(|o| (*o).to_string()).collect(),
            noun,
            fixed: true,
        }
    }
    pub(super) fn suggested(options: &[String], noun: &'static str) -> Self {
        Self {
            options: options.to_vec(),
            noun,
            fixed: false,
        }
    }
}
/// What a field may be picked from: local branches for a branch, the
/// endpoint's models for the model, the programs on this machine for the
/// editor and terminal, and the allowed words for a field that takes one of a
/// few. Fields that take free text have none.
pub(super) fn options(hub: &Settings, field: &Field) -> Option<Picker> {
    if names_branch(hub, field) {
        return Some(Picker::suggested(&hub.branches, "local branch"));
    }
    if names_model(hub, field) {
        return Some(Picker::suggested(&hub.models, "model"));
    }
    let group = field.groups.first().map(String::as_str).unwrap_or_default();
    Some(match (hub.key(), group, field.key.as_str()) {
        ("writing", _, "language") => Picker::fixed(preferences::LANGUAGES, "language"),
        ("agents", _, "adapter") => Picker::fixed(preferences::ADAPTERS, "adapter"),
        ("agents", _, "confinement") => Picker::fixed(preferences::CONFINEMENTS, "confinement"),
        ("branches", "promotions", "strategy") => {
            Picker::fixed(preferences::STRATEGIES, "strategy")
        }
        ("branches", "promotions", "from") => Picker {
            options: std::iter::once("feature".to_string())
                .chain(environment_ids(hub))
                .collect(),
            noun: "source",
            fixed: true,
        },
        ("branches", "promotions", "to") => Picker {
            options: environment_ids(hub),
            noun: "environment",
            fixed: true,
        },
        ("tools", _, "editor") => Picker::suggested(&hub.editors, "editor"),
        ("tools", _, "terminal") => Picker::suggested(&hub.shells, "shell"),
        _ => return None,
    })
}
/// The picker over the selected field, if that field has one.
pub(super) fn picker(hub: &Settings) -> Option<Picker> {
    if hub.editing_folder {
        return None;
    }
    fields(hub).get(hub.selected).and_then(|f| options(hub, f))
}
/// The options matching what has been typed so far; all of them while the
/// text is still the field's own value.
pub(super) fn choices(hub: &Settings) -> Vec<String> {
    let query = if hub.typed {
        hub.input.to_lowercase()
    } else {
        String::new()
    };
    picker(hub)
        .map(|p| p.options)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.to_lowercase().contains(&query))
        .collect()
}
/// Whether the field being edited offers a picker.
pub(super) fn picking(hub: &Settings) -> bool {
    picker(hub).is_some()
}
/// Whether the field being edited holds a number, so the arrows step it.
pub(super) fn stepping(hub: &Settings) -> bool {
    fields(hub)
        .get(hub.selected)
        .is_some_and(|f| matches!(f.value, Value::Number(_)))
}
/// Moves the number being edited by `delta`, never below zero; text that is
/// not a number yet starts from the field's saved value.
pub(super) fn step_number(hub: &mut Settings, delta: i64) {
    let current = hub
        .input
        .trim()
        .parse::<i64>()
        .ok()
        .or_else(|| fields(hub).get(hub.selected).and_then(|f| f.value.as_i64()));
    let next = current.unwrap_or_default().saturating_add(delta).max(0);
    hub.input = next.to_string();
    hub.cursor = hub.input.chars().count();
    hub.typed = true;
}
pub(super) fn move_choice(hub: &mut Settings, down: bool) {
    let count = choices(hub).len();
    if count == 0 {
        hub.choice = None;
        return;
    }
    hub.choice = Some(match (hub.choice, down) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(i), true) => (i + 1) % count,
        (Some(i), false) => (i + count - 1) % count,
    });
}
/// Opens the selected field: a boolean flips in place, anything else gets
/// the text editor.
pub(super) fn start_edit(hub: &mut Settings) {
    let Some(f) = fields(hub).get(hub.selected).cloned() else {
        return;
    };
    if let Value::Bool(current) = f.value {
        if let Some(slot) = hub.draft.pointer_mut(&f.path) {
            *slot = Value::Bool(!current);
        }
        return;
    }
    hub.input = display(&f.value);
    hub.cursor = hub.input.chars().count();
    hub.choice = None;
    hub.typed = false;
    if let Some(p) = options(hub, &f) {
        hub.choice = p.options.iter().position(|b| *b == hub.input);
        // A fixed list always has something highlighted, so Enter always
        // lands on an allowed value.
        if p.fixed && hub.choice.is_none() && !p.options.is_empty() {
            hub.choice = Some(0);
        }
    }
    hub.editing = true;
}

/// One editable leaf of the category's value tree.
#[derive(Clone)]
pub(super) struct Field {
    pub(super) path: String,
    /// The headings above the leaf: `["environments", "#1 Development"]`.
    pub(super) groups: Vec<String>,
    /// The leaf's own key as written in the file.
    pub(super) key: String,
    /// Everything a search may match on.
    pub(super) label: String,
    pub(super) value: Value,
}
/// What an item in a list of objects is called on its heading: its name or
/// id if it has one, or the move a promotion makes.
pub(super) fn summary(item: &Value) -> String {
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
pub(super) fn humanize(key: &str) -> String {
    key.replace('_', " ")
}
pub(super) fn flatten(value: &Value, path: &str, groups: &[String], out: &mut Vec<Field>) {
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
pub(super) fn fields(hub: &Settings) -> Vec<Field> {
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
pub(super) fn display(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}

/// A row of the field list: a heading over a run of nested fields, or a
/// field. Only fields can be selected; the arrow keys skip the headings.
pub(super) enum Row {
    Header { label: String, depth: usize },
    Field { index: usize, depth: usize },
}
pub(super) fn rows(fields: &[Field]) -> Vec<Row> {
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
pub(super) fn describe(category: &str, field: &Field) -> Option<&'static str> {
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
            "Model used for commit messages, reviews and summaries. Enter picks from the models the endpoint serves; L opens the model modal with connectivity checks."
        }
        ("models", _, "endpoint") => "Chat completions endpoint the model is reached at.",
        ("agents", _, "name") => "How the agent is listed in the session picker.",
        ("agents", _, "adapter") => {
            "Which integration drives it: claude, codex, pi, or terminal for a plain shell with no agent protocol. Enter chooses from the list."
        }
        ("agents", _, "executable") => "Program to launch, found on PATH or given as a full path.",
        ("agents", _, "args") => {
            "Extra arguments, as a JSON list: [\"--flag\", \"value\"]. Passed directly, without a shell."
        }
        ("agents", _, "model") => {
            "Model the agent is asked to use. Empty leaves the agent's own default."
        }
        ("agents", _, "confinement") => {
            "terrarium runs it in the sandbox profile, agent trusts the agent's own harness (the default for claude and codex), direct runs it unconfined. A terminal is never sandboxed. Enter chooses from the list."
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
            "How the promotion lands: merge (a merge commit), squash (one commit), or ff-only (refuse unless fast-forward). Enter chooses from the list."
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
        ("tools", _, "editor") => {
            "Program opened for a file with e. Enter offers the editors found on this machine; any other program may be typed. Empty uses $EDITOR."
        }
        ("tools", _, "editor_args") => {
            "Arguments for the editor, as a JSON list. The file path is appended."
        }
        ("tools", _, "terminal") => {
            "Shell a terminal session runs. Enter offers the shells found on this machine; any other program may be typed. Empty uses $SHELL."
        }
        ("tools", _, "terminal_args") => "Arguments for that shell, as a JSON list.",
        ("sessions", _, "label") => {
            "Name shown for the session in the workspace tree. Ctrl-S renames it."
        }
        ("sessions", _, "path") => "Working directory the session runs in.",
        ("sessions", _, "kind") => "What is running: an agent or a plain terminal.",
        ("sessions", _, "id") => "Internal identifier.",
        _ => return None,
    })
}

pub(super) fn commit_edit(hub: &mut Settings) -> Result<()> {
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
    let picker = options(hub, &field);
    let matching = choices(hub);
    let mut picked = picker
        .as_ref()
        .and(hub.choice)
        .and_then(|i| matching.get(i).cloned());
    if let Some(p) = &picker
        && p.fixed
    {
        // Typing the whole word, or enough of it to leave one match, is as
        // good as highlighting it. Anything else is not a value this field
        // takes, and is refused here rather than by the file on save.
        if picked.is_none() {
            picked = p
                .options
                .iter()
                .find(|o| **o == hub.input)
                .cloned()
                .or_else(|| (matching.len() == 1).then(|| matching[0].clone()));
        }
        if picked.is_none() {
            anyhow::bail!("{} must be one of: {}", field.label, p.options.join(", "));
        }
    }
    let value = match field.value {
        Value::String(_) => Value::String(picked.clone().unwrap_or_else(|| hub.input.clone())),
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
        .context("field no longer exists")? = value.clone();
    hub.editing = false;
    if let (Some(id), Value::String(branch)) = (environment_id(hub, &field), &value)
        && !branch.is_empty()
    {
        wire_environment(hub, &id, branch);
    }
    Ok(())
}
/// The id of the environment whose branch field this is.
pub(super) fn environment_id(hub: &Settings, field: &Field) -> Option<String> {
    if !(hub.key() == "branches"
        && field.path.starts_with("/environments/")
        && field.key == "branch")
    {
        return None;
    }
    let parent = field.path.rsplit_once('/')?.0;
    hub.draft
        .pointer(&format!("{parent}/id"))?
        .as_str()
        .map(str::to_string)
}
/// Makes a newly assigned environment branch a release target: protected
/// from feature-branch actions, and reachable by a feature promotion, so the
/// flow actions work without further setup.
pub(super) fn wire_environment(hub: &mut Settings, id: &str, branch: &str) {
    let mut added = Vec::new();
    if let Some(protected) = hub.draft["protected"].as_array_mut()
        && !protected.iter().any(|p| p.as_str() == Some(branch))
    {
        protected.push(Value::String(branch.into()));
        added.push(format!("protected {branch}"));
    }
    if let Some(promotions) = hub.draft["promotions"].as_array_mut()
        && !promotions.iter().any(|p| p["to"].as_str() == Some(id))
        && let Ok(rule) = serde_json::to_value(preferences::Promotion {
            to: id.into(),
            ..preferences::Promotion::default()
        })
    {
        promotions.push(rule);
        added.push(format!("promotion feature \u{2192} {id}"));
    }
    if !added.is_empty() {
        hub.notice = format!("Added {}.", added.join(" and "));
    }
}
