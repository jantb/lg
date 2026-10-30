//! Where the preference files live and how they stack: the scopes, the
//! checkout they are read for, and one load of all of them merged.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::cache::common_dir;
use super::{Preferences, Writing, detect_branches};

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
pub(super) struct Checkout {
    pub(super) root: String,
    pub(super) common_dir: PathBuf,
}

impl Checkout {
    pub(super) fn repository_file(&self) -> PathBuf {
        base_dir()
            .join("repositories")
            .join(slug(&self.common_dir.to_string_lossy()))
            .join("preferences.toml")
    }
    pub(super) fn worktree_file(&self) -> PathBuf {
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
pub(super) fn context_dir() -> PathBuf {
    crate::git::configuration_context()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

pub(super) fn checkout() -> Result<Checkout> {
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
pub(super) fn folder_files() -> Vec<PathBuf> {
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

/// The files a checkout's preferences are read from, least specific first: the
/// user's, each folder holding the checkout from the outermost in, then the
/// repository's and the worktree's. A later one wins wherever both say.
pub(super) fn layers(checkout: Option<&Checkout>) -> Vec<(Scope, PathBuf)> {
    let mut paths = vec![(Scope::User, base_dir().join("preferences.toml"))];
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
    paths
}

/// Missing configuration is fine. Invalid files remain visible and are never overwritten on load.
pub(super) fn load_uncached(checkout: Option<&Checkout>) -> Loaded {
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
        serde_json::to_value(Writing::from(legacy)),
    );
    if checkout.is_some_and(|c| crate::settings::is_configured_at(&c.root)) {
        sources.insert(
            "writing".into(),
            "Legacy checkout settings (preserved)".into(),
        );
    }
    for (scope, path) in layers(checkout) {
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

pub(super) fn read_patch(path: &Path) -> Result<serde_json::Value> {
    parse_patch(&std::fs::read_to_string(path)?)
}

/// Settings lg once had and has dropped. A file that still sets them reads as
/// if it did not, and the next save writes it back without them.
const RETIRED: [(&str, &str); 2] = [("writing", "subject_max"), ("writing", "body_lines")];

pub(super) fn parse_patch(text: &str) -> Result<serde_json::Value> {
    let value: toml::Value = toml::from_str(text).context("invalid TOML")?;
    let mut value = serde_json::to_value(value)?;
    for (category, key) in RETIRED {
        if let Some(table) = value.get_mut(category).and_then(|v| v.as_object_mut()) {
            table.remove(key);
        }
    }
    Ok(value)
}

pub fn default_folder() -> PathBuf {
    let root = checkout()
        .map(|c| PathBuf::from(c.root))
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    root.parent().unwrap_or(&root).to_path_buf()
}

pub(super) fn folder_path(folder: &Path) -> PathBuf {
    base_dir()
        .join("folders")
        .join(slug(&folder.to_string_lossy()))
        .join("preferences.toml")
}
