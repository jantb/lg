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
pub(crate) fn repo_dir() -> Option<PathBuf> {
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
    let mut command = base_command();
    let dir = repo_dir();
    if let Some(dir) = &dir {
        command.arg("-C").arg(dir);
    }
    command.args(args);
    batch_ssh_for_remotes(&mut command, dir.as_deref(), args);
    command
}

/// A `git` invocation aimed at `dir`. An explicit directory wins over the
/// selection, so callers that already know which checkout they mean — nested
/// repositories, worktrees — are unaffected by it.
pub(crate) fn git_command_in_dir(dir: &Path, args: &[&str]) -> Command {
    let mut command = base_command();
    command.arg("-C").arg(dir);
    command.args(args);
    batch_ssh_for_remotes(&mut command, Some(dir), args);
    command
}

/// Every git command lg runs, with the optional index lock turned off.
///
/// `git status` rewrites the index whenever the stat data it cached has gone
/// stale, and a refresh runs one in the checkout and in every worktree. The
/// watcher sees that write and asks for another refresh, which writes the index
/// again — and in a worktree something else keeps touching, a claude session or
/// a build, the stat data is never fresh, so the two chase each other for as
/// long as that worktree is busy. Reading a repository must not change it.
/// Commands that write take locks that are not optional and are unaffected.
///
/// None of them can ask for anything either. They run with their output
/// captured behind the TUI, so a credential prompt git opens on the terminal
/// is drawn over by the next frame and then waits for an answer nobody knows
/// is wanted — and the job that asked holds up every git job after it. With
/// terminal prompts off a missing credential fails the command instead, and
/// says so. Credential helpers and askpass programs are still consulted.
fn base_command() -> Command {
    let mut command = Command::new("git");
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command
}

/// The subcommands lg runs that reach a remote, and with it ssh. `remote` is
/// not among them: lg only lists remotes and reads their URLs, and asking the
/// configuration about ssh for each of those would cost a refresh a process.
const REMOTE_SUBCOMMANDS: [&str; 5] = ["fetch", "pull", "push", "ls-remote", "clone"];

/// What ssh runs as when nobody configured it: the same ssh, refusing to stop
/// for a passphrase or an unknown host key, for the reason git's own prompts
/// are off.
const BATCH_SSH_COMMAND: &str = "ssh -o BatchMode=yes";

/// Keep ssh from prompting too, for a command that may talk to a remote.
///
/// Only when ssh is otherwise left at git's default: `GIT_SSH_COMMAND` beats
/// both `GIT_SSH` and `core.sshCommand`, so setting it over either would throw
/// away a choice the user made.
fn batch_ssh_for_remotes(command: &mut Command, dir: Option<&Path>, args: &[&str]) {
    if subcommand(args).is_some_and(|sub| REMOTE_SUBCOMMANDS.contains(&sub)) && !ssh_configured(dir)
    {
        command.env("GIT_SSH_COMMAND", BATCH_SSH_COMMAND);
    }
}

/// The git subcommand among `args`, past any `-c key=value` or `-C dir`.
pub(super) fn subcommand<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "-c" | "-C" => {
                args.next();
            }
            arg if arg.starts_with('-') => {}
            arg => return Some(arg),
        }
    }
    None
}

/// Whether the user chose how git runs ssh, in the environment or in the
/// configuration `dir` sees.
fn ssh_configured(dir: Option<&Path>) -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if set("GIT_SSH_COMMAND") || set("GIT_SSH") {
        return true;
    }
    let mut config = Command::new("git");
    if let Some(dir) = dir {
        config.arg("-C").arg(dir);
    }
    config
        .args(["config", "--get", "core.sshCommand"])
        .output()
        .is_ok_and(|out| out.status.success() && !out.stdout.trim_ascii().is_empty())
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

    /// A status that rewrites the index wakes the watcher that asked for it,
    /// and the refresh it triggers runs another status.
    #[test]
    fn commands_leave_the_index_alone() {
        for command in [
            git_command(&["status"]),
            git_command_in_dir(Path::new("/tmp/explicit"), &["status"]),
        ] {
            assert!(
                command.get_envs().any(|(key, value)| key
                    == std::ffi::OsStr::new("GIT_OPTIONAL_LOCKS")
                    && value == Some(std::ffi::OsStr::new("0"))),
                "reading a repository must not write to it"
            );
        }
    }

    fn env_of(command: &Command, name: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new(name))
            .and_then(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
    }

    /// lg's git runs behind the TUI with its output captured, so a prompt
    /// opened on the terminal would wait for an answer nobody can see.
    #[test]
    fn commands_never_wait_on_a_terminal_prompt() {
        for command in [
            git_command(&["fetch", "--all"]),
            git_command(&["status"]),
            git_command_in_dir(Path::new("/tmp/explicit"), &["push", "origin", "main"]),
        ] {
            assert_eq!(
                env_of(&command, "GIT_TERMINAL_PROMPT").as_deref(),
                Some("0")
            );
        }
    }

    #[test]
    fn ssh_runs_in_batch_mode_unless_the_user_chose_how_it_runs() {
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        if set("GIT_SSH_COMMAND") || set("GIT_SSH") {
            // The environment already decides; there is nothing of lg's to see.
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).status.success());
        let inherited = git(&["config", "--get", "core.sshCommand"]);
        if inherited.status.success() {
            // A global core.sshCommand is the user's choice for this repo too.
            return;
        }

        let fetch = git_command_in_dir(repo.path(), &["fetch"]);
        assert_eq!(
            env_of(&fetch, "GIT_SSH_COMMAND").as_deref(),
            Some(BATCH_SSH_COMMAND)
        );

        assert!(
            git(&["config", "core.sshCommand", "ssh -i /tmp/key"])
                .status
                .success()
        );
        let fetch = git_command_in_dir(repo.path(), &["fetch"]);
        assert_eq!(
            env_of(&fetch, "GIT_SSH_COMMAND"),
            None,
            "core.sshCommand would lose to GIT_SSH_COMMAND, so it must not be set"
        );
    }
}
