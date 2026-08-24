//! The repository every git command runs against.
//!
//! Each command resolves its own directory here rather than relying on the
//! process working directory, which is never changed: the calling thread's pin
//! if it has one, otherwise the process-wide selection, and the process working
//! directory when neither is set. Jobs snapshot the selection when they are
//! spawned (`spawn_pinned`), so switching repositories mid-job cannot retarget
//! a job already in flight.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;
use std::thread::JoinHandle;

static ACTIVE_REPO: RwLock<Option<PathBuf>> = RwLock::new(None);

thread_local! {
    static PINNED_REPO: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Point later git commands at `dir`. Threads that pinned a directory of their
/// own keep it.
pub fn set_active_repo(dir: impl Into<PathBuf>) {
    let dir = dir.into();
    if let Ok(mut active) = ACTIVE_REPO.write() {
        *active = Some(dir);
    }
}

/// The process-wide selection, ignoring any pin on this thread.
pub fn active_repo() -> Option<PathBuf> {
    ACTIVE_REPO.read().ok().and_then(|active| active.clone())
}

/// Directory this thread's git commands run in.
fn repo_dir() -> Option<PathBuf> {
    PINNED_REPO
        .with(|pinned| pinned.borrow().clone())
        .or_else(active_repo)
}

/// Restores the previous pin even if the body panics.
struct PinGuard(Option<PathBuf>);

impl Drop for PinGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        PINNED_REPO.with(|pinned| *pinned.borrow_mut() = previous);
    }
}

/// Run `f` with this thread's git commands pointed at `dir`.
pub fn with_repo<T>(dir: impl Into<PathBuf>, f: impl FnOnce() -> T) -> T {
    let previous = PINNED_REPO.with(|pinned| pinned.replace(Some(dir.into())));
    let _guard = PinGuard(previous);
    f()
}

/// Spawn a thread that keeps running against the repository selected now, so a
/// switch while it works cannot move it to a different checkout.
pub fn spawn_pinned<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match repo_dir() {
        Some(dir) => std::thread::spawn(move || with_repo(dir, f)),
        None => std::thread::spawn(f),
    }
}

/// A `git` invocation aimed at this thread's repository.
pub(super) fn git_command(args: &[&str]) -> Command {
    let mut command = Command::new("git");
    if let Some(dir) = repo_dir() {
        command.arg("-C").arg(dir);
    }
    command.args(args);
    command
}

/// A `git` invocation aimed at `dir`. An explicit directory wins over the
/// selection, so callers that already know which checkout they mean — nested
/// repositories, worktrees — are unaffected by it.
pub(super) fn git_command_in_dir(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command.args(args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `active_repo` is process-wide, so tests that set it would collide; these
    /// only exercise the thread-local pin, which is per-test by construction.
    #[test]
    fn pin_applies_to_this_thread_only() {
        with_repo("/tmp/pinned", || {
            assert_eq!(repo_dir(), Some(PathBuf::from("/tmp/pinned")));
            let other = std::thread::spawn(repo_dir).join().unwrap();
            assert_eq!(other, active_repo());
        });
    }

    #[test]
    fn pin_is_restored_after_the_body() {
        let before = repo_dir();
        with_repo("/tmp/pinned", || {});
        assert_eq!(repo_dir(), before);
    }

    #[test]
    fn nested_pins_restore_the_outer_one() {
        with_repo("/tmp/outer", || {
            with_repo("/tmp/inner", || {
                assert_eq!(repo_dir(), Some(PathBuf::from("/tmp/inner")));
            });
            assert_eq!(repo_dir(), Some(PathBuf::from("/tmp/outer")));
        });
    }

    #[test]
    fn pin_survives_a_panicking_body() {
        let before = repo_dir();
        let panicked = std::panic::catch_unwind(|| {
            with_repo("/tmp/pinned", || panic!("boom"));
        });
        assert!(panicked.is_err());
        assert_eq!(repo_dir(), before);
    }

    #[test]
    fn spawned_threads_inherit_the_pin() {
        let inherited = with_repo("/tmp/pinned", || spawn_pinned(repo_dir).join().unwrap());
        assert_eq!(inherited, Some(PathBuf::from("/tmp/pinned")));
    }

    #[test]
    fn explicit_dir_wins_over_the_pin() {
        with_repo("/tmp/pinned", || {
            let command = git_command_in_dir(Path::new("/tmp/explicit"), &["status"]);
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args, ["-C", "/tmp/explicit", "status"]);
        });
    }

    #[test]
    fn pinned_dir_is_passed_to_git() {
        with_repo("/tmp/pinned", || {
            let command = git_command(&["status", "--porcelain"]);
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args, ["-C", "/tmp/pinned", "status", "--porcelain"]);
        });
    }
}
