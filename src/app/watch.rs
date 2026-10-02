//! Turning the file watcher's events into refreshes: which events matter,
//! how much each one has to read again, and when a burst of them has settled
//! enough to act on.

use notify::{
    EventKind,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::{FILE_EVENT_DEBOUNCE_MS, FILE_EVENT_MAX_WAIT_MS};
use crate::state::RefreshScope;

use super::refresh::{git_metadata_path_should_refresh, should_refresh_for_fs_event};

/// Directories whose contents are build output or tool state in practice
/// everywhere, so events from inside them are dropped without asking git.
/// Anything less certain is left for `git check-ignore` to judge.
const NEVER_TRACKED_DIRS: &[&str] = &["target", "node_modules", "__pycache__", ".venv", ".gradle"];

/// How deep below the checkout a directory appearing or going away is taken
/// as a possible repository arriving or leaving, which the workspace scan is
/// for. Deeper than this it is only files.
const WORKSPACE_DIR_DEPTH: usize = 3;

/// Above this many changed files, asking git which of them are ignored costs
/// more than just refreshing.
const MAX_CHECKED_PATHS: usize = 4096;

/// Where the watcher looks: the checkout, as spelled and as resolved, and the
/// git directories it uses outside it.
#[derive(Debug, Clone, Default)]
pub(super) struct WatchRoots {
    pub(super) root: PathBuf,
    pub(super) canonical_root: PathBuf,
    pub(super) git_dirs: Vec<PathBuf>,
}

impl WatchRoots {
    pub(super) fn new(root: &Path, git_dirs: Vec<PathBuf>) -> Self {
        Self {
            root: root.to_path_buf(),
            canonical_root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            git_dirs,
        }
    }

    /// `path` relative to the checkout, under either spelling of it.
    fn relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.canonical_root)
            .or_else(|_| path.strip_prefix(&self.root))
            .ok()
    }

    /// `path` relative to the git directory it is in, preferring the most
    /// specific one: a linked worktree's own directory sits inside the
    /// repository's shared one.
    fn git_relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        self.git_dirs
            .iter()
            .filter_map(|dir| path.strip_prefix(dir).ok())
            .min_by_key(|rest| rest.components().count())
    }
}

/// What one event asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    Nothing,
    /// Files in the checkout changed: the status, unless git ignores them all.
    Files(Vec<PathBuf>),
    Scope(RefreshScope),
}

/// How much `event` has to read again. `nested` are the repositories found
/// inside the checkout, relative to it: their state is only read by the
/// workspace scan.
fn classify(event: &notify::Event, roots: &WatchRoots, nested: &[PathBuf]) -> Change {
    if !should_refresh_for_fs_event(event) {
        return Change::Nothing;
    }
    if event.paths.is_empty() {
        return Change::Scope(RefreshScope::Workspace);
    }
    let mut files = Vec::new();
    let mut scope: Option<RefreshScope> = None;
    for path in &event.paths {
        match classify_path(event.kind, path, roots, nested) {
            Change::Nothing => {}
            Change::Files(mut paths) => files.append(&mut paths),
            Change::Scope(wanted) => scope = scope.max(Some(wanted)),
        }
    }
    match scope {
        Some(scope) => Change::Scope(scope),
        None if files.is_empty() => Change::Nothing,
        None => Change::Files(files),
    }
}

fn classify_path(kind: EventKind, path: &Path, roots: &WatchRoots, nested: &[PathBuf]) -> Change {
    if let Some(rest) = roots.git_relative(path) {
        let parts = normal_parts(rest);
        if !git_metadata_path_should_refresh(&parts) {
            return Change::Nothing;
        }
        // The index alone changes what is staged, and nothing about branches.
        return Change::Scope(if parts.first() == Some(&"index") {
            RefreshScope::Files
        } else {
            RefreshScope::Repository
        });
    }
    let Some(rel) = roots.relative(path) else {
        // Somewhere the watcher was not asked about; take no chances.
        return Change::Scope(RefreshScope::Repository);
    };
    let parts = normal_parts(rel);
    if let Some(git_at) = parts.iter().position(|part| *part == ".git") {
        if !git_metadata_path_should_refresh(&parts[git_at + 1..]) {
            return Change::Nothing;
        }
        // The checkout's own git directory, or a repository nested in it.
        return Change::Scope(match git_at {
            0 if parts.get(1) == Some(&"index") => RefreshScope::Files,
            0 => RefreshScope::Repository,
            _ => RefreshScope::Workspace,
        });
    }
    if parts.iter().any(|part| NEVER_TRACKED_DIRS.contains(part)) {
        return Change::Nothing;
    }
    if nested.iter().any(|repo| rel.starts_with(repo)) {
        return Change::Scope(RefreshScope::Workspace);
    }
    if parts.len() <= WORKSPACE_DIR_DEPTH && is_directory_coming_or_going(kind, path) {
        return Change::Scope(RefreshScope::Workspace);
    }
    if parts.is_empty() {
        return Change::Scope(RefreshScope::Workspace);
    }
    Change::Files(vec![rel.to_path_buf()])
}

/// Whether `kind` is a directory being created, removed or renamed. A removal
/// that does not say what was removed may have been a directory.
fn is_directory_coming_or_going(kind: EventKind, path: &Path) -> bool {
    match kind {
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => true,
        EventKind::Remove(RemoveKind::Any | RemoveKind::Other) => true,
        EventKind::Create(CreateKind::Any | CreateKind::Other)
        | EventKind::Modify(ModifyKind::Name(_)) => path.is_dir() || !path.exists(),
        _ => false,
    }
}

fn normal_parts(path: &Path) -> Vec<&str> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect()
}

/// A refresh the settled events ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DueRefresh {
    pub(super) scope: RefreshScope,
    /// For a [`RefreshScope::Files`] refresh, the files that changed, so it
    /// can be skipped when git ignores them all. Empty: not worth checking.
    pub(super) changed: Vec<PathBuf>,
}

/// File events gathered until they settle.
///
/// A save is several events, a build or a checkout thousands, and each one
/// used to start a refresh of its own. They are now held until none has
/// arrived for [`FILE_EVENT_DEBOUNCE_MS`], and acted on together — or after
/// [`FILE_EVENT_MAX_WAIT_MS`], so a file written to continuously still shows
/// its changes now and then rather than never.
#[derive(Debug, Default)]
pub(super) struct FileBatch {
    first_at: Option<Instant>,
    last_at: Option<Instant>,
    scope: Option<RefreshScope>,
    changed: Vec<PathBuf>,
    /// More files changed than are worth checking one by one.
    overflowed: bool,
}

impl FileBatch {
    /// Take in one event that arrived at `now`.
    pub(super) fn add(
        &mut self,
        event: &notify::Event,
        roots: &WatchRoots,
        nested: &[PathBuf],
        now: Instant,
    ) {
        let wanted = match classify(event, roots, nested) {
            Change::Nothing => return,
            Change::Files(paths) => {
                if !self.overflowed {
                    self.changed.extend(paths);
                    if self.changed.len() > MAX_CHECKED_PATHS {
                        self.overflowed = true;
                        self.changed.clear();
                    }
                }
                RefreshScope::Files
            }
            Change::Scope(scope) => scope,
        };
        self.scope = self.scope.max(Some(wanted));
        self.first_at.get_or_insert(now);
        self.last_at = Some(now);
    }

    /// When the gathered events are due to be acted on, if any are waiting.
    pub(super) fn deadline(&self) -> Option<Instant> {
        let first = self.first_at?;
        let last = self.last_at.unwrap_or(first);
        Some(
            (last + Duration::from_millis(FILE_EVENT_DEBOUNCE_MS))
                .min(first + Duration::from_millis(FILE_EVENT_MAX_WAIT_MS)),
        )
    }

    /// The refresh the gathered events ask for, once they are due at `now`.
    pub(super) fn take_due(&mut self, now: Instant) -> Option<DueRefresh> {
        if self.deadline()? > now {
            return None;
        }
        let batch = std::mem::take(self);
        let scope = batch.scope?;
        let checkable = scope == RefreshScope::Files && !batch.overflowed;
        Some(DueRefresh {
            scope,
            changed: if checkable { batch.changed } else { Vec::new() },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::{Event, event::DataChange};

    fn roots() -> WatchRoots {
        WatchRoots {
            root: PathBuf::from("/repo"),
            canonical_root: PathBuf::from("/private/repo"),
            git_dirs: vec![PathBuf::from("/repo/.git")],
        }
    }

    fn write(path: &str) -> Event {
        Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
            .add_path(PathBuf::from(path))
    }

    fn settled(batch: &mut FileBatch, at: Instant) -> Option<DueRefresh> {
        batch.take_due(at + Duration::from_millis(FILE_EVENT_DEBOUNCE_MS))
    }

    #[test]
    fn a_burst_of_events_starts_one_refresh_once_it_settles() {
        let mut batch = FileBatch::default();
        let start = Instant::now();
        for step in 0..20u64 {
            let at = start + Duration::from_millis(step * 10);
            batch.add(&write("/repo/src/main.rs"), &roots(), &[], at);
            assert_eq!(batch.take_due(at), None, "still arriving at step {step}");
        }
        let last = start + Duration::from_millis(190);
        let due = settled(&mut batch, last).expect("a refresh once it is quiet");
        assert_eq!(due.scope, RefreshScope::Files);
        assert_eq!(settled(&mut batch, last), None, "and only the one");
    }

    #[test]
    fn a_file_written_continuously_still_refreshes_now_and_then() {
        let mut batch = FileBatch::default();
        let start = Instant::now();
        let mut refreshed = false;
        let mut at = start;
        while at < start + Duration::from_millis(FILE_EVENT_MAX_WAIT_MS + 100) {
            batch.add(&write("/repo/log.txt"), &roots(), &[], at);
            refreshed |= batch.take_due(at).is_some();
            at += Duration::from_millis(50);
        }
        assert!(
            refreshed,
            "the cap must cut through a stream that never pauses"
        );
    }

    #[test]
    fn editing_a_file_reads_only_the_status_again() {
        let mut batch = FileBatch::default();
        let now = Instant::now();
        batch.add(&write("/private/repo/src/lib.rs"), &roots(), &[], now);
        let due = settled(&mut batch, now).unwrap();
        assert_eq!(due.scope, RefreshScope::Files);
        assert_eq!(due.changed, vec![PathBuf::from("src/lib.rs")]);
    }

    #[test]
    fn a_commit_or_branch_change_reads_the_repository_again() {
        for path in [
            "/repo/.git/HEAD",
            "/repo/.git/refs/heads/main",
            "/repo/.git/packed-refs",
        ] {
            let mut batch = FileBatch::default();
            let now = Instant::now();
            batch.add(&write(path), &roots(), &[], now);
            assert_eq!(
                settled(&mut batch, now).map(|due| due.scope),
                Some(RefreshScope::Repository),
                "{path}"
            );
        }
    }

    #[test]
    fn staging_from_outside_reads_the_status_again() {
        let mut batch = FileBatch::default();
        let now = Instant::now();
        batch.add(&write("/repo/.git/index"), &roots(), &[], now);
        assert_eq!(
            settled(&mut batch, now).map(|due| due.scope),
            Some(RefreshScope::Files)
        );
    }

    #[test]
    fn the_widest_scope_in_a_batch_wins() {
        let mut batch = FileBatch::default();
        let now = Instant::now();
        batch.add(&write("/repo/src/lib.rs"), &roots(), &[], now);
        batch.add(&write("/repo/.git/HEAD"), &roots(), &[], now);
        let due = settled(&mut batch, now).unwrap();
        assert_eq!(due.scope, RefreshScope::Repository);
        assert!(due.changed.is_empty(), "nothing to skip a full refresh for");
    }

    #[test]
    fn build_output_and_git_internals_are_dropped() {
        let mut batch = FileBatch::default();
        let now = Instant::now();
        for path in [
            "/repo/target/debug/lg",
            "/repo/web/node_modules/x/index.js",
            "/repo/.git/objects/ab/cdef",
        ] {
            batch.add(&write(path), &roots(), &[], now);
        }
        assert_eq!(batch.deadline(), None);
        assert_eq!(settled(&mut batch, now), None);
    }

    #[test]
    fn a_repository_nested_in_the_checkout_is_read_by_the_workspace_scan() {
        let nested = [PathBuf::from("libs/inner")];
        for path in ["/repo/libs/inner/src/a.rs", "/repo/libs/inner/.git/HEAD"] {
            let mut batch = FileBatch::default();
            let now = Instant::now();
            batch.add(&write(path), &roots(), &nested, now);
            assert_eq!(
                settled(&mut batch, now).map(|due| due.scope),
                Some(RefreshScope::Workspace),
                "{path}"
            );
        }
    }

    #[test]
    fn a_directory_appearing_near_the_top_rescans_the_workspace() {
        let mut batch = FileBatch::default();
        let now = Instant::now();
        let event = Event::new(EventKind::Create(CreateKind::Folder))
            .add_path(PathBuf::from("/repo/clone"));
        batch.add(&event, &roots(), &[], now);
        assert_eq!(
            settled(&mut batch, now).map(|due| due.scope),
            Some(RefreshScope::Workspace)
        );
    }

    #[test]
    fn an_event_without_paths_refreshes_everything() {
        let mut batch = FileBatch::default();
        let now = Instant::now();
        batch.add(&Event::new(EventKind::Any), &roots(), &[], now);
        assert_eq!(
            settled(&mut batch, now).map(|due| due.scope),
            Some(RefreshScope::Workspace)
        );
    }
}
