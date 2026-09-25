//! Versioned local configuration, shared by linked worktrees with explicit overrides.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub writing: Writing,
    pub models: Models,
    pub agents: Vec<Agent>,
    pub sandbox: Sandbox,
    pub branches: Branches,
    pub tools: Tools,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Writing {
    pub language: String,
    pub comment_style: String,
    pub subject_max: usize,
    pub body_lines: usize,
    pub commit_prompt: String,
    pub review_style: String,
}
impl Default for Writing {
    fn default() -> Self {
        Self {
            language: "English".into(),
            comment_style: String::new(),
            subject_max: 72,
            body_lines: 8,
            commit_prompt: crate::config::COMMIT_PROMPT_PREFIX.into(),
            review_style: crate::config::REVIEW_STYLE_GUIDE.into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Models {
    /// Whether lg talks to the model at all. Off, the commit message is typed,
    /// conflicts are left to the person, and the review assists stay quiet,
    /// so nothing waits on a server that is not running.
    pub enabled: bool,
    pub model: String,
    pub endpoint: String,
}
impl Default for Models {
    fn default() -> Self {
        Self {
            enabled: true,
            model: crate::config::LLM_MODEL.into(),
            endpoint: crate::config::MTPLX_CHAT_ENDPOINT.into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    pub name: String,
    pub adapter: String,
    pub executable: String,
    pub args: Vec<String>,
    pub model: String,
    pub confinement: String,
    pub default: bool,
}
impl Default for Agent {
    fn default() -> Self {
        Self {
            name: "Custom agent".into(),
            adapter: "terminal".into(),
            executable: String::new(),
            args: vec![],
            model: String::new(),
            confinement: default_confinement("terminal").into(),
            default: false,
        }
    }
}
impl Agent {
    /// Whether lg wraps the process in the Terrarium sandbox. A terminal is
    /// never wrapped: it is the user's own shell, and confining it only gets
    /// in the way of the work done there by hand.
    pub fn sandboxed(&self) -> bool {
        self.confinement == "terrarium" && self.adapter != "terminal"
    }
}
/// The integrations an agent may be driven through. The settings picker offers
/// exactly these, so a typo cannot reach the file.
pub const ADAPTERS: &[&str] = &["claude", "codex", "pi", "terminal"];
/// The languages generated prose may be written in.
pub const LANGUAGES: &[&str] = &["English", "Norwegian"];
/// How tightly an agent is confined: the terrarium sandbox, the agent's own
/// permission harness, or nothing.
pub const CONFINEMENTS: &[&str] = &["terrarium", "agent", "direct"];
/// The ways a promotion may land on an environment branch.
pub const STRATEGIES: &[&str] = &["merge", "squash", "ff-only"];
/// The confinement an adapter starts out with: coding agents bring their own
/// permission harness and run under it, anything else runs unconfined.
pub fn default_confinement(adapter: &str) -> &'static str {
    if ["claude", "codex"].contains(&adapter) {
        "agent"
    } else {
        "direct"
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Sandbox {
    pub preset: String,
}
impl Default for Sandbox {
    fn default() -> Self {
        Self {
            preset: "rust".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Branches {
    pub base: String,
    pub remote: String,
    pub protected: Vec<String>,
    pub environments: Vec<Environment>,
    pub promotions: Vec<Promotion>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub remote: String,
    pub branch: String,
    pub url: String,
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            id: "new".into(),
            name: "New environment".into(),
            remote: "origin".into(),
            branch: String::new(),
            url: String::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Promotion {
    pub from: String,
    pub to: String,
    pub strategy: String,
    pub push: bool,
}
impl Default for Promotion {
    fn default() -> Self {
        Self {
            from: "feature".into(),
            to: "dev".into(),
            strategy: "merge".into(),
            push: true,
        }
    }
}
/// A checkout with nothing saved has no environments of its own; what it
/// deploys is read off its branches by [`detect_branches`].
impl Default for Branches {
    fn default() -> Self {
        Self {
            base: crate::config::BRANCH_MAIN.into(),
            remote: crate::config::DEFAULT_PUSH_REMOTE.into(),
            protected: vec![crate::config::BRANCH_MAIN.into()],
            environments: vec![],
            promotions: vec![],
        }
    }
}
/// The id of the environment the integration branch deploys.
pub const PROD_ENV_ID: &str = "prod";
/// Where the detected configuration is said to come from.
pub const DETECTED_SOURCE: &str = "Detected from local branches";
/// The branch configuration a checkout implies. `develop` or `dev` is the
/// development environment and `test` the test environment, when those
/// branches exist; the integration branch is production unless a `prod`
/// branch exists to take that place.
pub fn detect_branches(local: &[String]) -> Branches {
    let has = |name: &str| local.iter().any(|b| b == name);
    let base = if !has(crate::config::BRANCH_MAIN) && has("master") {
        "master".to_string()
    } else {
        crate::config::BRANCH_MAIN.to_string()
    };
    let remote = crate::config::DEFAULT_PUSH_REMOTE.to_string();
    let environment = |id: &str, name: &str, branch: &str| Environment {
        id: id.into(),
        name: name.into(),
        remote: remote.clone(),
        branch: branch.into(),
        url: String::new(),
    };
    let mut environments = Vec::new();
    if let Some(dev) = crate::config::DEV_BRANCH_NAMES
        .into_iter()
        .find(|name| has(name))
    {
        environments.push(environment("dev", "Development", dev));
    }
    if has(crate::config::BRANCH_TEST) {
        environments.push(environment("test", "Test", crate::config::BRANCH_TEST));
    }
    let prod = if has(PROD_ENV_ID) { PROD_ENV_ID } else { &base };
    environments.push(environment(PROD_ENV_ID, "Production", prod));
    let promotions = environments
        .iter()
        .filter(|e| e.id != PROD_ENV_ID)
        .map(|e| Promotion {
            to: e.id.clone(),
            ..Promotion::default()
        })
        .collect();
    let mut protected = vec![base.clone()];
    for e in &environments {
        if !protected.contains(&e.branch) {
            protected.push(e.branch.clone());
        }
    }
    Branches {
        base,
        remote,
        protected,
        environments,
        promotions,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Tools {
    pub decorative_animations: bool,
    pub quiet_author_emails: Vec<String>,
    pub editor: String,
    pub editor_args: Vec<String>,
    pub terminal: String,
    pub terminal_args: Vec<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            writing: Writing::default(),
            models: Models::default(),
            agents: ["claude", "codex", "pi"]
                .into_iter()
                .map(|name| Agent {
                    name: name.into(),
                    adapter: name.into(),
                    executable: name.into(),
                    default: name == "claude",
                    confinement: default_confinement(name).into(),
                    ..Agent::default()
                })
                .collect(),
            sandbox: Sandbox::default(),
            branches: Branches::default(),
            tools: Tools::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    Folder,
    Repository,
    Worktree,
}
impl Scope {
    pub const ALL: [Self; 3] = [Self::User, Self::Repository, Self::Worktree];
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Folder => "Folder",
            Self::Repository => "Repository",
            Self::Worktree => "Worktree",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Preferences,
    pub sources: BTreeMap<String, String>,
    pub errors: Vec<String>,
}

pub fn base_dir() -> PathBuf {
    std::env::var_os("LG_PREFERENCES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/lg")
        })
}
fn slug(path: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for b in path.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
pub fn scope_path(scope: Scope) -> Result<PathBuf> {
    Ok(match scope {
        Scope::User => base_dir().join("preferences.toml"),
        Scope::Folder => folder_path(&default_folder()),
        Scope::Repository => checkout()?.repository_file(),
        Scope::Worktree => checkout()?.worktree_file(),
    })
}
/// Where a directory's checkout is: the top of its working tree and the
/// repository's shared Git directory. Repository and worktree preferences are
/// found through these, and neither moves while lg runs.
#[derive(Debug, Clone)]
struct Checkout {
    root: String,
    common_dir: PathBuf,
}
impl Checkout {
    fn repository_file(&self) -> PathBuf {
        base_dir()
            .join("repositories")
            .join(slug(&self.common_dir.to_string_lossy()))
            .join("preferences.toml")
    }
    fn worktree_file(&self) -> PathBuf {
        base_dir()
            .join("worktrees")
            .join(slug(&self.root))
            .join("preferences.toml")
    }
}
/// Checkouts already located, by the directory git was asked in. Each answer
/// costs two git launches, and preferences are read on the thread that draws.
/// A failure is not kept, since a folder that is no checkout may become one.
static CHECKOUTS: std::sync::Mutex<BTreeMap<PathBuf, Checkout>> =
    std::sync::Mutex::new(BTreeMap::new());
/// The directory preferences are read for: the one git commands run in.
fn context_dir() -> PathBuf {
    crate::git::configuration_context()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}
fn checkout() -> Result<Checkout> {
    let dir = context_dir();
    let known = CHECKOUTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&dir)
        .cloned();
    if let Some(found) = known {
        return Ok(found);
    }
    let root = crate::git::repo_root()?;
    if root.is_empty() {
        bail!("not inside a git checkout");
    }
    let found = Checkout {
        root,
        common_dir: common_dir()?,
    };
    CHECKOUTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(dir, found.clone());
    Ok(found)
}
/// The per-folder preference files, in a stable order.
fn folder_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(base_dir().join("folders"))
        .into_iter()
        .flatten()
        .filter_map(|e| Some(e.ok()?.path().join("preferences.toml")))
        .collect();
    files.sort();
    files
}
pub(crate) fn merge(base: &mut serde_json::Value, patch: serde_json::Value) {
    if let (Some(base), Some(patch)) = (base.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            if let Some(old) = base.get_mut(key) {
                merge(old, value.clone());
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = patch;
    }
}
/// Missing configuration is fine. Invalid files remain visible and are never overwritten on load.
fn load_uncached(checkout: Option<&Checkout>) -> Loaded {
    let mut sources = BTreeMap::new();
    let mut errors = Vec::new();
    let mut config = serialized(
        &mut errors,
        "built-in defaults",
        serde_json::to_value(Preferences::default()),
    );
    if let Some(model) = crate::llm::legacy_model() {
        config["models"]["model"] = model.into();
        sources.insert(
            "models".into(),
            "Legacy model configuration (preserved)".into(),
        );
    }
    // Existing files are an explicit compatibility layer; leave originals in place.
    let legacy = checkout
        .map(|c| crate::settings::load_legacy(&c.root))
        .unwrap_or_default();
    config["writing"] = serialized(
        &mut errors,
        "legacy writing settings",
        serde_json::to_value(Writing {
            language: legacy.pr_language,
            comment_style: legacy.comment_style,
            subject_max: legacy.commit_subject_max_chars,
            body_lines: legacy.commit_body_max_lines,
            commit_prompt: legacy.commit_prompt,
            review_style: legacy.review_style,
        }),
    );
    if checkout.is_some_and(|c| crate::settings::is_configured_at(&c.root)) {
        sources.insert(
            "writing".into(),
            "Legacy checkout settings (preserved)".into(),
        );
    }
    let mut paths = Vec::new();
    match scope_path(Scope::User) {
        Ok(path) => paths.push((Scope::User, path)),
        Err(e) => errors.push(format!("user preferences: {e}")),
    }
    let root = checkout.map(|c| PathBuf::from(&c.root));
    let mut folders: Vec<(PathBuf, PathBuf)> = folder_files()
        .into_iter()
        .filter_map(|path| {
            let patch = read_patch(&path).ok()?;
            let folder = PathBuf::from(patch.get("folder")?.as_str()?);
            root.as_ref().filter(|root| root.starts_with(&folder))?;
            Some((folder, path))
        })
        .collect();
    folders.sort_by_key(|(folder, _)| folder.components().count());
    paths.extend(folders.into_iter().map(|(_, p)| (Scope::Folder, p)));
    if let Some(c) = checkout {
        paths.push((Scope::Repository, c.repository_file()));
        paths.push((Scope::Worktree, c.worktree_file()));
    }
    for (scope, path) in paths {
        if !path.exists() {
            continue;
        }
        match read_patch(&path) {
            Ok(mut patch) => {
                if let Some(object) = patch.as_object_mut() {
                    object.remove("folder");
                }
                let mut candidate = config.clone();
                merge(&mut candidate, patch.clone());
                match serde_json::from_value::<Preferences>(candidate.clone())
                    .and_then(|v| v.validate().map(|()| v).map_err(serde::de::Error::custom))
                {
                    Ok(_) => {
                        config = candidate;
                        for key in patch
                            .as_object()
                            .into_iter()
                            .flat_map(|m| m.keys())
                            .filter(|k| k.as_str() != "version")
                        {
                            sources.insert(
                                key.clone(),
                                format!("{}: {}", scope.label(), path.display()),
                            );
                        }
                    }
                    Err(e) => errors.push(format!("{}: {e}", path.display())),
                }
            }
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    if !sources.contains_key("branches") {
        config["branches"] = serialized(
            &mut errors,
            "detected branches",
            serde_json::to_value(detect_branches(&crate::git::local_branch_names())),
        );
    }
    if let Ok(v) = std::env::var("LG_LLM_ENABLED") {
        let on = !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        );
        config["models"]["enabled"] = on.into();
        sources.insert(
            "models.enabled".into(),
            "LG_LLM_ENABLED environment override".into(),
        );
    }
    if let Ok(v) = std::env::var("LG_LLM_MODEL") {
        config["models"]["model"] = v.into();
        sources.insert(
            "models.model".into(),
            "LG_LLM_MODEL environment override".into(),
        );
    }
    if let Ok(v) =
        std::env::var("LG_MTPLX_CHAT_ENDPOINT").or_else(|_| std::env::var("LG_MTPLX_URL"))
    {
        config["models"]["endpoint"] = v.into();
        sources.insert("models.endpoint".into(), "Environment override".into());
    }
    let config = serde_json::from_value(config).unwrap_or_else(|e| {
        errors.push(format!("effective configuration: {e}"));
        Preferences::default()
    });
    Loaded {
        config,
        sources,
        errors,
    }
}
/// Serializing lg's own types cannot fail in practice; if it ever does, the
/// configuration screen shows why instead of the app refusing to start.
fn serialized(
    errors: &mut Vec<String>,
    what: &str,
    value: serde_json::Result<serde_json::Value>,
) -> serde_json::Value {
    match value {
        Ok(value) => value,
        Err(e) => {
            errors.push(format!("{what}: {e}"));
            serde_json::Value::Object(Default::default())
        }
    }
}

fn read_patch(path: &Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(path)?;
    let value: toml::Value = toml::from_str(&text).context("invalid TOML")?;
    Ok(serde_json::to_value(value)?)
}
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("missing configuration directory")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(content)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub fn save_category(scope: Scope, category: &str, value: serde_json::Value) -> Result<()> {
    save_at(scope_path(scope)?, None, category, value)
}
pub fn save_folder(folder: &Path, category: &str, value: serde_json::Value) -> Result<()> {
    let folder = folder.canonicalize().context("folder must exist")?;
    save_at(folder_path(&folder), Some(&folder), category, value)
}
fn save_at(
    path: PathBuf,
    folder: Option<&Path>,
    category: &str,
    value: serde_json::Value,
) -> Result<()> {
    let mut patch = if path.exists() {
        read_patch(&path)?
    } else {
        serde_json::json!({"version":1})
    };
    patch[category] = value;
    let saved_folder = patch
        .as_object_mut()
        .with_context(|| format!("{}: configuration is not a table", path.display()))?
        .remove("folder");
    let mut effective = serde_json::to_value(load().config)?;
    merge(&mut effective, patch.clone());
    serde_json::from_value::<Preferences>(effective)?.validate()?;
    if let Some(folder) = folder {
        patch["folder"] = folder.display().to_string().into();
    } else if let Some(folder) = saved_folder {
        patch["folder"] = folder;
    }
    if path.exists() {
        std::fs::copy(&path, path.with_extension("toml.bak"))?;
    }
    invalidate();
    atomic_write(&path, toml::to_string_pretty(&patch)?.as_bytes())
}
pub fn reset_category(scope: Scope, category: &str) -> Result<()> {
    reset_at(scope_path(scope)?, category)
}
fn reset_at(path: PathBuf, category: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut patch = read_patch(&path)?;
    patch
        .as_object_mut()
        .context("expected table")?
        .remove(category);
    std::fs::copy(&path, path.with_extension("toml.bak"))?;
    invalidate();
    atomic_write(&path, toml::to_string_pretty(&patch)?.as_bytes())
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported configuration version {}", self.version);
        }
        if self.writing.language.trim().is_empty() {
            bail!("writing.language must not be empty");
        }
        if !self.models.endpoint.starts_with("http://")
            && !self.models.endpoint.starts_with("https://")
        {
            bail!("models.endpoint must be an HTTP(S) URL");
        }
        validate_ref(&self.branches.base)?;
        validate_remote(&self.branches.remote)?;
        let mut ids = HashSet::new();
        let mut refs = HashSet::new();
        for e in &self.branches.environments {
            if e.id.is_empty() || e.id == "feature" || !ids.insert(e.id.as_str()) {
                bail!("environment IDs must be unique, nonempty, and not 'feature'");
            }
            validate_remote(&e.remote)?;
            if !e.branch.is_empty() {
                validate_ref(&e.branch)?;
                if !refs.insert((&e.remote, &e.branch)) {
                    bail!("multiple environments map to {}/{}", e.remote, e.branch);
                }
            }
        }
        for b in &self.branches.protected {
            validate_ref(b)?;
        }
        for p in &self.branches.promotions {
            if !ids.contains(p.to.as_str())
                || (p.from != "feature" && !ids.contains(p.from.as_str()))
            {
                bail!("promotion references an unknown environment");
            }
            if !STRATEGIES.contains(&p.strategy.as_str()) {
                bail!("promotion strategy must be merge, squash or ff-only");
            }
            let mut seen = HashSet::new();
            if cycle(
                &p.from,
                &self.branches.promotions,
                &mut seen,
                &mut HashSet::new(),
            ) {
                bail!("promotion paths contain a cycle");
            }
        }
        let mut names = HashSet::new();
        if self.agents.iter().filter(|a| a.default).count() > 1 {
            bail!("choose only one default agent");
        }
        for a in &self.agents {
            if a.name.trim().is_empty() || !names.insert(&a.name) {
                bail!("agent names must be unique and nonempty");
            }
            if a.executable.is_empty() || a.executable.contains('\0') {
                bail!("agent executable must not be empty");
            }
            if !ADAPTERS.contains(&a.adapter.as_str()) {
                bail!("unknown agent adapter {}", a.adapter);
            }
            if !CONFINEMENTS.contains(&a.confinement.as_str()) {
                bail!("confinement must be terrarium, agent or direct");
            }
            if a.confinement == "agent" && !["claude", "codex"].contains(&a.adapter.as_str()) {
                bail!(
                    "{} does not provide an agent-managed permission adapter",
                    a.name
                );
            }
        }
        Ok(())
    }
}
fn cycle<'a>(
    node: &'a str,
    edges: &'a [Promotion],
    stack: &mut HashSet<&'a str>,
    done: &mut HashSet<&'a str>,
) -> bool {
    if done.contains(node) {
        return false;
    }
    if !stack.insert(node) {
        return true;
    }
    for e in edges.iter().filter(|e| e.from == node) {
        if cycle(&e.to, edges, stack, done) {
            return true;
        }
    }
    stack.remove(node);
    done.insert(node);
    false
}
fn validate_ref(s: &str) -> Result<()> {
    if s.is_empty()
        || s.starts_with('-')
        || s.contains([' ', '\n', '\r', '\0', ':', '~', '^', '?', '*', '[', '\\'])
        || s.contains("..")
        || s.contains("@{")
        || s.ends_with('/')
        || s.ends_with('.')
        || s.ends_with(".lock")
    {
        bail!("invalid branch name: {s}");
    }
    Ok(())
}
fn validate_remote(s: &str) -> Result<()> {
    if s.is_empty() || s.starts_with('-') || s.contains(['/', ' ', '\n', '\0']) {
        bail!("invalid remote name: {s}");
    }
    Ok(())
}
pub fn base_branch() -> String {
    load().config.branches.base
}
pub fn remote() -> String {
    load().config.branches.remote
}
pub fn configured_category(category: &str) -> bool {
    load().sources.contains_key(category)
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// How long the files behind a load are trusted before being looked at again.
/// Readers come many times a frame, and an edit made outside lg need only show
/// up about as soon as someone could look for it.
const FILES_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);
/// One loaded configuration, kept until something it was read from changes, so
/// the many readers in a frame share a single read and an idle lg asks git
/// nothing.
struct Cached {
    root: PathBuf,
    generation: u64,
    checkout: Option<Checkout>,
    refs: RefsFingerprint,
    files: FilesFingerprint,
    files_checked: std::time::Instant,
    loaded: Loaded,
}
impl Cached {
    fn common_dir(&self) -> Option<&Path> {
        self.checkout.as_ref().map(|c| c.common_dir.as_path())
    }
    fn files_unchanged(&mut self) -> bool {
        if self.files_checked.elapsed() < FILES_RECHECK {
            return true;
        }
        self.files_checked = std::time::Instant::now();
        files_fingerprint(self.checkout.as_ref()) == self.files
    }
}
/// When the top-level branches last changed: the modification times of the
/// loose-refs directory and the packed-refs file. Branches are detected from
/// the checkout, so a branch made or deleted a moment ago must show up at once
/// rather than after the cache expires.
type RefsFingerprint = (Option<std::time::SystemTime>, Option<std::time::SystemTime>);
fn refs_fingerprint(common_dir: Option<&Path>) -> RefsFingerprint {
    let modified = |name: &str| {
        common_dir
            .and_then(|dir| std::fs::metadata(dir.join(name)).ok())
            .and_then(|meta| meta.modified().ok())
    };
    (modified("refs/heads"), modified("packed-refs"))
}
/// Every file a load reads, and when each last changed: the user's, each
/// folder's, the checkout's, and the legacy settings and model files.
type FilesFingerprint = Vec<(PathBuf, Option<(std::time::SystemTime, u64)>)>;
fn files_fingerprint(checkout: Option<&Checkout>) -> FilesFingerprint {
    let mut files = vec![base_dir().join("preferences.toml")];
    files.extend(folder_files());
    if let Some(c) = checkout {
        files.push(c.repository_file());
        files.push(c.worktree_file());
        files.extend(crate::settings::legacy_files(&c.root));
    }
    files.extend(crate::llm::legacy_model_file());
    files
        .into_iter()
        .map(|path| {
            let stamp = std::fs::metadata(&path)
                .ok()
                .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
            (path, stamp)
        })
        .collect()
}
fn common_dir() -> Result<PathBuf> {
    let out = crate::git::run(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        bail!("git reported no repository directory");
    }
    Ok(PathBuf::from(path))
}
thread_local! { static CACHE: std::cell::RefCell<Option<Cached>> = const { std::cell::RefCell::new(None) }; }
pub fn invalidate() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}
pub fn load() -> Loaded {
    let root = context_dir();
    let generation = GENERATION.load(std::sync::atomic::Ordering::Relaxed);
    CACHE.with(|cache| {
        if let Some(cached) = cache.borrow_mut().as_mut()
            && cached.root == root
            && cached.generation == generation
            && cached.refs == refs_fingerprint(cached.common_dir())
            && cached.files_unchanged()
        {
            return cached.loaded.clone();
        }
        let checkout = checkout().ok();
        // Taken before reading, so a change that lands mid-read is seen as one
        // on the next look rather than lost.
        let refs = refs_fingerprint(checkout.as_ref().map(|c| c.common_dir.as_path()));
        let files = files_fingerprint(checkout.as_ref());
        let loaded = load_uncached(checkout.as_ref());
        *cache.borrow_mut() = Some(Cached {
            root,
            generation,
            checkout,
            refs,
            files,
            files_checked: std::time::Instant::now(),
            loaded: loaded.clone(),
        });
        loaded
    })
}
pub fn default_folder() -> PathBuf {
    let root = checkout()
        .map(|c| PathBuf::from(c.root))
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    root.parent().unwrap_or(&root).to_path_buf()
}
fn folder_path(folder: &Path) -> PathBuf {
    base_dir()
        .join("folders")
        .join(slug(&folder.to_string_lossy()))
        .join("preferences.toml")
}
/// Whether the model is asked for anything: commit messages, conflict
/// resolutions and review assists. `LG_LLM_ENABLED=0` turns it off for one run.
pub fn ai_enabled() -> bool {
    load().config.models.enabled
}
pub fn animations_enabled() -> bool {
    load().config.tools.decorative_animations
        && animations_enabled_for(crate::git::author_config().ok().as_ref())
}
/// [`animations_enabled`] for a caller that has already read who commits here.
pub fn animations_enabled_for(author: Option<&crate::git::AuthorConfig>) -> bool {
    let tools = load().config.tools;
    if !tools.decorative_animations {
        return false;
    }
    let email = author
        .and_then(|a| a.email.as_deref())
        .unwrap_or_default()
        .to_lowercase();
    !tools.quiet_author_emails.iter().any(|rule| {
        let rule = rule.to_lowercase();
        if let Some(domain) = rule.strip_prefix("*@") {
            email.rsplit_once('@').is_some_and(|(_, d)| d == domain)
        } else {
            email == rule
        }
    })
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            decorative_animations: true,
            quiet_author_emails: vec![],
            editor: String::new(),
            editor_args: vec![],
            terminal: String::new(),
            terminal_args: vec![],
        }
    }
}
pub fn reset_folder(folder: &Path, category: &str) -> Result<()> {
    reset_at(folder_path(&folder.canonicalize()?), category)
}

pub fn import_document(scope: Scope, patch: serde_json::Value) -> Result<()> {
    let path = scope_path(scope)?;
    let mut document = if path.exists() {
        read_patch(&path)?
    } else {
        serde_json::json!({"version":1})
    };
    merge(&mut document, patch);
    let mut effective = serde_json::to_value(load().config)?;
    merge(&mut effective, document.clone());
    serde_json::from_value::<Preferences>(effective)?.validate()?;
    if path.exists() {
        std::fs::copy(&path, path.with_extension("toml.bak"))?;
    }
    atomic_write(&path, toml::to_string_pretty(&document)?.as_bytes())?;
    invalidate();
    Ok(())
}

#[cfg(test)]
mod detect_tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }
    fn valid(b: &Branches) {
        Preferences {
            branches: b.clone(),
            ..Preferences::default()
        }
        .validate()
        .unwrap();
    }
    fn branch_of<'a>(b: &'a Branches, id: &str) -> Option<&'a str> {
        b.environments
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.branch.as_str())
    }

    #[test]
    fn a_checkout_without_deploy_branches_only_has_production_on_main() {
        let b = detect_branches(&names(&["main", "feature/x"]));
        assert_eq!(branch_of(&b, "prod"), Some("main"));
        assert_eq!(branch_of(&b, "dev"), None);
        assert_eq!(branch_of(&b, "test"), None);
        assert!(b.promotions.is_empty());
        valid(&b);
    }

    #[test]
    fn dev_and_test_branches_become_environments_with_promotions() {
        let b = detect_branches(&names(&["main", "dev", "test"]));
        assert_eq!(branch_of(&b, "dev"), Some("dev"));
        assert_eq!(branch_of(&b, "test"), Some("test"));
        assert_eq!(branch_of(&b, "prod"), Some("main"));
        let targets: Vec<_> = b.promotions.iter().map(|p| p.to.as_str()).collect();
        assert!(targets.contains(&"dev") && targets.contains(&"test"));
        assert!(!targets.contains(&"prod"));
        for e in &b.environments {
            assert!(b.protected.contains(&e.branch), "{} unprotected", e.branch);
        }
        valid(&b);
    }

    #[test]
    fn a_local_prod_branch_is_production_instead_of_main() {
        let b = detect_branches(&names(&["main", "prod", "develop"]));
        assert_eq!(branch_of(&b, "prod"), Some("prod"));
        assert_eq!(branch_of(&b, "dev"), Some("develop"));
        assert_eq!(b.base, "main");
    }

    #[test]
    fn master_is_the_integration_branch_when_there_is_no_main() {
        let b = detect_branches(&names(&["master", "test"]));
        assert_eq!(b.base, "master");
        assert_eq!(branch_of(&b, "prod"), Some("master"));
    }
}
