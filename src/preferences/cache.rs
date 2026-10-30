//! The preferences in effect, loaded once and read again only when a file or
//! the branches they depend on change, and the shortcuts that read them.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

use super::layers::{
    Checkout, Loaded, base_dir, checkout, context_dir, folder_files, load_uncached,
};

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

pub(super) fn common_dir() -> Result<PathBuf> {
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
