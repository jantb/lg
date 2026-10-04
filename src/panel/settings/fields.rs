//! The flat list of editable fields a category shows, and how a value is picked, stepped and committed.

use super::*;
use crate::llm::LlmProvider;

/// What a field is chosen from.
pub(super) struct Picker {
    pub(super) options: Vec<String>,
    /// What one option is called in the hints: "local branch", "strategy".
    pub(super) noun: &'static str,
    /// Whether the value must be one of the options. An enumeration is; a
    /// branch or a program is a suggestion, and a typed name is kept.
    pub(super) fixed: bool,
    /// A dim note beside an option, by the option it belongs to: what an
    /// alias stands for.
    pub(super) notes: Vec<(String, String)>,
}
impl Picker {
    pub(super) fn fixed(options: &[&str], noun: &'static str) -> Self {
        Self {
            options: options.iter().map(|o| (*o).to_string()).collect(),
            noun,
            fixed: true,
            notes: Vec::new(),
        }
    }
    pub(super) fn suggested(options: &[String], noun: &'static str) -> Self {
        Self {
            options: options.to_vec(),
            noun,
            fixed: false,
            notes: Vec::new(),
        }
    }
    /// The note shown beside `option`, if it has one.
    pub(super) fn note(&self, option: &str) -> Option<&str> {
        self.notes
            .iter()
            .find(|(name, _)| name == option)
            .map(|(_, note)| note.as_str())
            .filter(|note| !note.is_empty())
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
    Some(match (hub.current_category(), group, field.key.as_str()) {
        (Category::Writing, _, "language") => Picker::fixed(preferences::LANGUAGES, "language"),
        (Category::Models, _, "provider") => Picker::fixed(preferences::PROVIDERS, "provider"),
        (Category::Models, "claude", "model") => Picker {
            options: hub.claude_models.iter().map(|m| m.name.clone()).collect(),
            noun: "Claude model",
            fixed: false,
            notes: hub
                .claude_models
                .iter()
                .map(|m| (m.name.clone(), m.note.clone()))
                .collect(),
        },
        (Category::Agents, _, "adapter") => Picker::fixed(preferences::ADAPTERS, "adapter"),
        (Category::Agents, _, "confinement") => {
            Picker::fixed(preferences::CONFINEMENTS, "confinement")
        }
        (Category::Branches, "promotions", "strategy") => {
            Picker::fixed(preferences::STRATEGIES, "strategy")
        }
        (Category::Branches, "promotions", "from") => Picker {
            options: std::iter::once("feature".to_string())
                .chain(environment_ids(hub))
                .collect(),
            noun: "source",
            fixed: true,
            notes: Vec::new(),
        },
        (Category::Branches, "promotions", "to") => Picker {
            options: environment_ids(hub),
            noun: "environment",
            fixed: true,
            notes: Vec::new(),
        },
        (Category::Tools, _, "editor") => Picker::suggested(&hub.editors, "editor"),
        (Category::Tools, _, "terminal") => Picker::suggested(&hub.shells, "shell"),
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
        hub.input.text.to_lowercase()
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
        .text
        .trim()
        .parse::<i64>()
        .ok()
        .or_else(|| fields(hub).get(hub.selected).and_then(|f| f.value.as_i64()));
    let next = current.unwrap_or_default().saturating_add(delta).max(0);
    hub.input.set(next.to_string());
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
    hub.input.set(display(&f.value));
    hub.choice = None;
    hub.typed = false;
    if let Some(p) = options(hub, &f) {
        hub.choice = p.options.iter().position(|b| *b == hub.input.text);
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
/// The provider whose settings a Models field belongs to, by the word
/// `provider` takes for it; `None` for the fields every provider shares.
fn provider_of(path: &str) -> Option<LlmProvider> {
    match path {
        "/endpoint" | "/model" => Some(LlmProvider::Mtplx),
        "/claude_model" => Some(LlmProvider::Claude),
        _ => None,
    }
}
/// Puts the Models fields under a heading per provider, after the shared
/// ones, so the settings only one provider reads are seen to be that
/// provider's. The paths stay as the file has them.
fn group_by_provider(fields: &mut [Field]) {
    for field in fields.iter_mut() {
        let Some(provider) = provider_of(&field.path) else {
            continue;
        };
        let group = provider.config_value().to_string();
        if field.key == "claude_model" {
            field.key = "model".into();
        }
        field.label = format!("{group} \u{b7} {}", humanize(&field.key));
        field.groups = vec![group];
    }
    fields.sort_by_key(|f| provider_of(&f.path).map(|p| p as usize + 1).unwrap_or(0));
}
/// The provider `provider` names in the draft, if it is one.
pub(super) fn chosen_provider(hub: &Settings) -> Option<LlmProvider> {
    let word = hub.draft.get("provider")?.as_str()?;
    LlmProvider::ALL
        .into_iter()
        .find(|p| p.config_value() == word)
}
/// Whether the field belongs to a provider other than the chosen one, and so
/// is not read while that choice stands.
pub(super) fn unused(hub: &Settings, field: &Field) -> bool {
    hub.current_category() == Category::Models
        && provider_of(&field.path).is_some_and(|p| chosen_provider(hub).is_some_and(|c| c != p))
}
/// What a provider heading in Models says after its name: what the
/// provider is, and whether it is the one in use. `None` for any other
/// heading.
pub(super) fn provider_heading(hub: &Settings, label: &str) -> Option<(&'static str, bool)> {
    if hub.current_category() != Category::Models {
        return None;
    }
    let provider = LlmProvider::ALL
        .into_iter()
        .find(|p| p.config_value() == label)?;
    let what = match provider {
        LlmProvider::Mtplx => "an OpenAI-compatible chat endpoint",
        LlmProvider::Claude => "the claude CLI",
    };
    Some((what, chosen_provider(hub).is_none_or(|c| c == provider)))
}
pub(super) fn fields(hub: &Settings) -> Vec<Field> {
    let mut fields = Vec::new();
    flatten(&hub.draft, "", &[], &mut fields);
    if hub.current_category() == Category::Models {
        group_by_provider(&mut fields);
    }
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
pub(super) fn describe(category: Category, field: &Field) -> Option<&'static str> {
    let group = field.groups.first().map(String::as_str).unwrap_or_default();
    Some(match (category, group, field.key.as_str()) {
        (Category::Identity, _, "scope") => {
            "Repository writes the author into this checkout's .git/config. Folder writes a Git includeIf for every repository below the folder."
        }
        (Category::Identity, _, "folder") => {
            "Where the Folder scope applies. Every repository under this path gets the author below."
        }
        (Category::Identity, _, "name") => {
            "Author name for commits made from lg. Signing keys stay in Git's own configuration."
        }
        (Category::Identity, _, "email") => {
            "Author email for commits made from lg. Matches the quiet author emails list in Interface & Tools, which hides your own commits from activity."
        }
        (Category::Writing, _, "language") => {
            "Language commit messages and pull request text are written in."
        }
        (Category::Writing, _, "comment_style") => {
            "The shape of a commit message: Conventional Commits, plain imperative, and so on. Press L to derive it from this checkout's history."
        }
        (Category::Writing, _, "subject_max") => {
            "Longest subject line the commit writer may produce, in characters."
        }
        (Category::Writing, _, "body_lines") => "Most lines the commit body may run to.",
        (Category::Writing, _, "commit_prompt") => {
            "The instructions given to the model before the staged diff. Enter opens the whole text in the editor; v previews the assembled prompt."
        }
        (Category::Writing, _, "review_style") => {
            "House rules the reviewer checks changes against."
        }
        (Category::Models, _, "enabled") => {
            "Whether lg asks the model at all. Off, commit messages are typed by hand and conflicts and reviews are left alone; use it when no model server is running."
        }
        (Category::Models, _, "provider") => {
            "Who answers every AI request — commit messages, review assists, the review chat, guided reviews and conflict resolutions. local sends them to the chat endpoint under local; claude runs the claude CLI (tools off, your Claude Code login) with the settings under claude. Only the chosen provider's settings are read. Enter chooses from the list."
        }
        (Category::Models, "claude", "model") => {
            "Model the claude CLI is asked for. Enter picks from the models the CLI offers; any other model id may be typed. Empty uses the Claude agent profile's model, or the CLI default."
        }
        (Category::Models, "local", "model") => {
            "Model the chat endpoint is asked for. Enter picks from the models the endpoint serves; L opens the model modal with connectivity checks."
        }
        (Category::Models, "local", "endpoint") => {
            "OpenAI-compatible chat completions URL the requests are sent to."
        }
        (Category::Agents, _, "name") => "How the agent is listed in the session picker.",
        (Category::Agents, _, "adapter") => {
            "Which integration drives it: claude, codex, pi, or terminal for a plain shell with no agent protocol. Enter chooses from the list."
        }
        (Category::Agents, _, "executable") => {
            "Program to launch, found on PATH or given as a full path."
        }
        (Category::Agents, _, "args") => {
            "Extra arguments, as a JSON list: [\"--flag\", \"value\"]. Passed directly, without a shell."
        }
        (Category::Agents, _, "model") => {
            "Model the agent is asked to use. Empty leaves the agent's own default."
        }
        (Category::Agents, _, "confinement") => {
            "terrarium runs it in the sandbox profile, agent trusts the agent's own harness (the default for claude and codex), direct runs it unconfined. A terminal is never sandboxed. Enter chooses from the list."
        }
        (Category::Agents, _, "default") => {
            "The agent a new session starts with when none is chosen. One agent should be true."
        }
        (Category::Sandbox, _, "preset") => {
            "Terrarium preset the private sandbox profile is generated from; i creates it, v validates it, e opens it."
        }
        (Category::Sandbox, _, "profile") => "The Seatbelt profile itself. Ctrl-S writes it back.",
        (Category::Branches, _, "base") => {
            "The trunk: feature branches start here and merge back here. The deployment block measures every environment against it."
        }
        (Category::Branches, _, "protected") => {
            "Branches lg will not delete or force-push, as a JSON list."
        }
        (Category::Branches, "environments", "id") => {
            "Key the rest of lg refers to the environment by. Release actions and the deployment block are wired for dev and test; other ids are stored but not shown there yet."
        }
        (Category::Branches, "environments", "name") => {
            "Label shown in the deployment block and the branch action menu."
        }
        (Category::Branches, "environments", "branch") => {
            "The branch that deploys this environment, for example develop or test. Empty means lg cannot tell what is deployed and hides the environment."
        }
        (Category::Branches, "environments", "remote") => {
            "Remote whose copy of the branch is the deployed one."
        }
        (Category::Branches, _, "remote") => {
            "Remote whose copies of the deploy branches count as released, so a stale local checkout does not report a release that never landed."
        }
        (Category::Branches, "environments", "url") => "Where the environment runs. Informational.",
        (Category::Branches, "promotions", "from") => {
            "Source of the promotion: an environment id, or the word feature for the branch you are on."
        }
        (Category::Branches, "promotions", "to") => "Environment id the source is promoted into.",
        (Category::Branches, "promotions", "strategy") => {
            "How the promotion lands: merge (a merge commit), squash (one commit), or ff-only (refuse unless fast-forward). Enter chooses from the list."
        }
        (Category::Branches, "promotions", "push") => {
            "Push the environment branch to its remote after promoting."
        }
        (Category::Tools, _, "decorative_animations") => {
            "Pulsing frames and travelling light. false holds every frame still."
        }
        (Category::Tools, _, "quiet_author_emails") => {
            "Authors whose commits are not announced in activity, as a JSON list; * matches any prefix: [\"*@client.com\"]."
        }
        (Category::Tools, _, "editor") => {
            "Program opened for a file with e. Enter offers the editors found on this machine; any other program may be typed. Empty uses $EDITOR."
        }
        (Category::Tools, _, "editor_args") => {
            "Arguments for the editor, as a JSON list. The file path is appended."
        }
        (Category::Tools, _, "terminal") => {
            "Shell a terminal session runs. Enter offers the shells found on this machine; any other program may be typed. Empty uses $SHELL."
        }
        (Category::Tools, _, "terminal_args") => "Arguments for that shell, as a JSON list.",
        (Category::Sessions, _, "label") => {
            "Name shown for the session in the workspace tree. Ctrl-S renames it."
        }
        (Category::Sessions, _, "path") => "Working directory the session runs in.",
        (Category::Sessions, _, "kind") => "What is running: an agent or a plain terminal.",
        (Category::Sessions, _, "id") => "Internal identifier.",
        _ => return None,
    })
}

pub(super) fn commit_edit(hub: &mut Settings) -> Result<()> {
    if hub.editing_folder {
        hub.folder = hub.input.text.clone();
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
                .find(|o| **o == hub.input.text)
                .cloned()
                .or_else(|| (matching.len() == 1).then(|| matching[0].clone()));
        }
        if picked.is_none() {
            anyhow::bail!("{} must be one of: {}", field.label, p.options.join(", "));
        }
    }
    let value = match field.value {
        Value::String(_) => Value::String(picked.clone().unwrap_or_else(|| hub.input.text.clone())),
        Value::Bool(_) => Value::Bool(hub.input.text.parse().context("use true or false")?),
        Value::Number(_) => Value::from(
            hub.input
                .text
                .parse::<u64>()
                .context("enter a nonnegative integer")?,
        ),
        _ => serde_json::from_str(&hub.input.text)
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
    if !(hub.current_category() == Category::Branches
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
        hub.notify(NoticeKind::Info, format!("Added {}.", added.join(" and ")));
    }
}
