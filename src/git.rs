//! Running git, and the reads and writes lg builds on it.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

mod attrs;
mod branches;
mod commits;
mod config;
mod context;
mod diff;
pub mod environments;
mod flow;
pub mod guided;
mod history;
pub mod hunk;
mod index;
mod merge_editor;
mod nested;
pub mod patch;
mod plain;
mod release;
mod remote;
mod review;
mod stash;
mod status;
mod worktree;

pub use attrs::{
    FileAttrs, SUPPRESSED_DIFF_MARKER, file_attrs, is_suppressed_diff_body, suppress_generated_diff,
};
pub use branches::{
    Branch, RemoteBranch, list_branches, list_remote_branches, local_branch_names,
    nested_repo_branches, nested_repo_branches_at, nested_repo_remote_branches,
    nested_repo_remote_branches_at,
};
use commits::preferred_commit_ref;
pub use commits::{
    Commit, commit_messages_since, counts_ahead_behind, list_commits, list_commits_for_ref,
    recent_commit_messages, unpushed_shas,
};
pub use config::{
    AuthorConfig, IdeOpenCommand, add_to_gitignore, author_config, clear_local_author,
    clear_subtree_author, ide_open_command, open_file_in_ide, open_project_in_ide,
    open_project_path_in_ide, project_open_command, set_local_author, set_subtree_author,
    subtree_author_rule_exists,
};
use context::git_command;
pub(crate) use context::git_command_in_dir;
pub(crate) use context::repo_dir;
pub use context::{active_repo, set_active_repo, spawn_pinned, with_repo};
pub use diff::{
    all_diffs, branch_log, file_diff, folder_diff, repo_root, repo_root_at, show_commit,
    staged_diff,
};
pub use flow::{
    ConflictHunk, ConflictSideCommit, ConflictSides, ConflictedFile, FilePart, Followup,
    OrphanBranch, abort_in_progress_operation, abort_in_progress_operation_with_cleanup,
    abort_in_progress_operation_with_return, checkout_branch, checkout_remote_branch,
    conflict_sides, conflicted_files, delete_local_branch, delete_remote_branch,
    flow_clean_orphan_branches, flow_create_feature_branch, flow_discard_checkout_from_remote,
    flow_discard_checkout_from_remote_with_progress, flow_merge_main_into_all_local_branches,
    flow_merge_main_into_current, flow_merge_main_into_current_with_progress, flow_release_current,
    flow_release_current_with_progress, flow_reset_branch_from_main,
    flow_reset_branch_from_main_with_progress, flow_transfer_diff_to_feature_branch,
    flow_transfer_diff_to_feature_branch_with_progress, holds_conflict_marker, is_safety_ref,
    marker_line, orphan_branches, stage_resolved_conflicts, validate_conflict_resolution,
};
pub use history::{
    amend_commit, cherry_pick_commit, full_sha, head_commit_message, head_published_on,
    revert_commit,
};
pub use index::{
    commit, delete_worktree_path, rollback_worktree_path, stage, stage_all, unstage, unstage_all,
};
pub use merge_editor::MergeSnapshot;
pub use nested::{
    NestedRepo, checkout_nested_branch, checkout_nested_branch_at, checkout_nested_remote_branch,
    checkout_nested_remote_branch_at, nested_repositories, nested_repositories_at,
};
pub use plain::{init_repository, new_file_diff, new_file_entries, new_files_diff};
pub use release::{
    BranchReleaseStatus, ReleaseBranches, ReleaseEnv, ReleaseTargetStatus, branch_release_status,
    configured_release_branches, release_branches,
};
pub use remote::{
    FetchOutcome, ForcePushPlan, fetch_updates, force_push_plan, merge_upstream, pull, push,
    push_force_with_lease, push_with_upstream, remote_url, set_branch_upstream,
};
pub use review::{
    AssistedReview, BranchDiff, Language, REVIEW_AGENT_NODE_ID, REVIEW_PR_TEXT_NODE_ID, ReviewNode,
    assisted_review_against_main, branch_diff_against_main, build_assisted_review_against_main,
    is_test_path, uncommitted_diff,
};
pub use stash::{StashEntry, stash_apply, stash_drop, stash_list, stash_pop, stash_push};
pub use status::{
    FileEntry, all_ignored, parse_porcelain, parse_porcelain_xy, status_entries, status_porcelain,
};
pub use worktree::{
    Worktree, common_git_dir, default_worktree_path, forget_resolved_dirs, main_worktree,
    parse_worktree_list, preferred_base_ref, same_dir, worktree_add, worktree_add_detached,
    worktree_bring_home, worktree_land, worktree_land_with_progress, worktree_prune,
    worktree_remove, worktree_slug, worktree_sync_main, worktree_sync_main_with_progress,
    worktrees, worktrees_knowing,
};

/// The lock file git refused over, when that is why a command failed.
///
/// Git says so across four lines and puts the part worth acting on last, so a
/// status bar with room for the first line says only that some file exists.
/// A lock left behind by a git process that died — lg crashing takes its own
/// git children with it — then makes every command that writes the index fail
/// while `git status` carries on as if nothing were wrong.
fn refused_lock_path(text: &str) -> Option<&str> {
    let (_, rest) = text.split_once("Unable to create '")?;
    let (path, _) = rest.split_once("': File exists")?;
    Some(path)
}

/// What a failed git command should say: the command, then what git printed.
/// The status bar shows the start of it on one line and `!` opens all of it,
/// so what matters most goes first.
fn failure_message(command: &str, text: &str) -> String {
    match refused_lock_path(text) {
        Some(path) => format!(
            "{command} failed: {path} exists, so another git process may be running here; delete it if none is"
        ),
        None => format!("{command} failed: {}", text.trim()),
    }
}

/// Same, keeping git's own output for the callers that show all of it.
fn combined_failure_message(command: &str, text: &str) -> String {
    match refused_lock_path(text) {
        Some(_) => failure_message(command, text),
        None => format!("{command} failed:\n{text}"),
    }
}

/// Longest a command that reads from a remote may take before it is killed.
///
/// Prompts are already off (see `context::base_command`), so what this catches
/// is a remote that takes the connection and then never answers. Left alone,
/// that fetch holds the git job slot, and commit, push and pull all wait on it
/// for as long as the connection stays open.
const REMOTE_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// The subcommands cut off at [`REMOTE_READ_TIMEOUT`]. A push is not among
/// them: a large one over a slow link can honestly take longer than any fixed
/// bound, and killing it part way through tells nobody anything.
const TIMED_SUBCOMMANDS: [&str; 2] = ["fetch", "ls-remote"];

/// Run a prepared git command to the end and collect what it printed.
/// `label` names it in errors.
fn command_output(command: Command, args: &[&str], label: &str) -> Result<Output> {
    if !context::subcommand(args).is_some_and(|sub| TIMED_SUBCOMMANDS.contains(&sub)) {
        let mut command = command;
        return command
            .output()
            .with_context(|| format!("failed to spawn {label}"));
    }
    match output_with_timeout(command, REMOTE_READ_TIMEOUT)
        .with_context(|| format!("failed to spawn {label}"))?
    {
        Some(out) => Ok(out),
        None => anyhow::bail!("{label} timed out after {}s", REMOTE_READ_TIMEOUT.as_secs()),
    }
}

/// [`Command::output`], giving up after `timeout`: `None` when the command had
/// to be killed.
///
/// The command runs in a process group of its own and the whole group is
/// killed, because the part that hangs is usually the ssh git started rather
/// than git itself — and an ssh left running holds the output pipes open. The
/// pipes are drained on threads of their own for the same reason: a command
/// that fills one would otherwise never exit.
fn output_with_timeout(mut command: Command, timeout: Duration) -> std::io::Result<Option<Output>> {
    use std::process::Stdio;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let stdout = drain_pipe(child.stdout.take());
    let stderr = drain_pipe(child.stderr.take());

    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(Output {
                status,
                stdout: stdout.join().unwrap_or_default(),
                stderr: stderr.join().unwrap_or_default(),
            }));
        }
        if std::time::Instant::now() >= deadline {
            kill_process_group(&mut child);
            let _ = child.wait();
            // The readers finish once the last writer is gone; nothing waits
            // for them, in case something outside the group still holds a pipe.
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn drain_pipe<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    })
}

fn kill_process_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: killpg only sends a signal. The group is the one this child
        // was made the leader of when it was spawned, and the child has not
        // been waited for, so its id cannot have been reused.
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    let _ = child.kill();
}

/// Run git with `args`, in `dir` when given and in the selected repository
/// otherwise, and collect what it printed along with the label errors use.
fn execute(dir: Option<&Path>, args: &[&str]) -> Result<(Output, String)> {
    let label = command_label(dir, args);
    let command = match dir {
        Some(dir) => git_command_in_dir(dir, args),
        None => git_command(args),
    };
    let out = command_output(command, args, &label)?;
    Ok((out, label))
}

/// The output of a command that succeeded, or git's complaint about it.
fn checked_output(out: Output, label: &str) -> Result<Output> {
    if out.status.success() {
        Ok(out)
    } else {
        Err(anyhow::anyhow!(
            "{}",
            failure_message(label, &failure_text(&out))
        ))
    }
}

/// What git said about a command that failed. Git complains on stderr, but a
/// hook prints wherever it likes and several commands report on stdout
/// (`nothing to commit`), so stdout is kept when stderr has nothing, and both
/// are when both have something.
fn failure_text(out: &Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (stderr, stdout) = (stderr.trim(), stdout.trim());
    match (stderr.is_empty(), stdout.is_empty()) {
        (false, true) => stderr.to_string(),
        (true, false) => stdout.to_string(),
        (false, false) => format!("{stderr}\n{stdout}"),
        (true, true) => format!("exited with {}", out.status),
    }
}

/// How errors name a git command: the subcommand, and for the commands that
/// talk to a remote the remote they talked to. Never the rest of the
/// arguments: a commit's carry its whole message, which buried whatever the
/// hook that refused it had to say.
fn command_label(dir: Option<&Path>, args: &[&str]) -> String {
    let mut label = String::from("git");
    let mut rest = args.iter().copied();
    let mut sub = None;
    while let Some(arg) = rest.next() {
        match arg {
            "-c" | "-C" => {
                rest.next();
            }
            arg if arg.starts_with('-') => {}
            arg => {
                sub = Some(arg);
                break;
            }
        }
    }
    if let Some(sub) = sub {
        label.push(' ');
        label.push_str(sub);
        if matches!(sub, "push" | "fetch" | "pull")
            && let Some(remote) = rest.find(|arg| !arg.starts_with('-'))
        {
            label.push(' ');
            label.push_str(remote);
        }
    }
    if let Some(name) = dir.and_then(|dir| dir.file_name()) {
        label.push_str(" in ");
        label.push_str(&name.to_string_lossy());
    }
    label
}

pub(crate) fn run(args: &[&str]) -> Result<Output> {
    let (out, label) = execute(None, args)?;
    checked_output(out, &label)
}

/// Run git with `input` on its standard input, as `git apply -` reads a patch.
pub(crate) fn run_with_input(args: &[&str], input: &str) -> Result<Output> {
    use std::io::Write;
    use std::process::Stdio;

    let label = command_label(None, args);
    let mut child = git_command(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {label}"))?;
    let writer = child.stdin.take().map(|mut stdin| {
        let input = input.to_owned();
        // Written from a thread of its own: a patch bigger than the pipe
        // would otherwise wait on git reading it while git waits on us
        // reading its output.
        std::thread::spawn(move || stdin.write_all(input.as_bytes()))
    });
    let out = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for {label}"))?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    checked_output(out, &label)
}

fn run_in_dir(dir: &Path, args: &[&str]) -> Result<Output> {
    let (out, label) = execute(Some(dir), args)?;
    checked_output(out, &label)
}

fn run_combined(args: &[&str]) -> Result<String> {
    let (out, label) = execute(None, args)?;
    combined_output(out, &label)
}

fn run_combined_in_dir(dir: &Path, args: &[&str]) -> Result<String> {
    let (out, label) = execute(Some(dir), args)?;
    combined_output(out, &label)
}

/// Stdout and stderr together, the way the callers that show all of git's
/// output want it.
fn combined_output(out: Output, label: &str) -> Result<String> {
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(anyhow::anyhow!(
            "{}",
            combined_failure_message(label, &text)
        ))
    }
}

pub fn is_repo() -> bool {
    git_command(&["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn head_branch() -> Result<String> {
    let out = run(&["branch", "--show-current"])?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !branch.is_empty() {
        return Ok(branch);
    }

    let out = run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if branch == "HEAD" {
        anyhow::bail!("detached HEAD");
    }
    Ok(branch)
}

pub(crate) use context::repo_dir as configuration_context;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_command_that_finishes_in_time_hands_back_its_output() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo out; echo err >&2; exit 3"]);

        let out = output_with_timeout(command, Duration::from_secs(10))
            .unwrap()
            .expect("finished well inside the timeout");

        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
        assert_eq!(out.status.code(), Some(3));
    }

    /// A commit's arguments carry its whole message; an error naming them
    /// pushed what the hook said off the end of the status bar.
    #[test]
    fn errors_name_the_subcommand_and_never_the_message() {
        assert_eq!(
            command_label(None, &["commit", "-m", "a long message\nwith a body"]),
            "git commit"
        );
        assert_eq!(
            command_label(None, &["-c", "core.x=y", "push", "-u", "origin", "feature"]),
            "git push origin"
        );
        assert_eq!(command_label(None, &["fetch", "--prune"]), "git fetch");
        assert_eq!(
            command_label(Some(Path::new("/work/nested")), &["pull", "origin"]),
            "git pull origin in nested"
        );
    }

    #[test]
    fn a_failure_keeps_stdout_when_stderr_has_nothing_and_both_when_both_do() {
        let output = |script: &str| {
            Command::new("sh")
                .args(["-c", script])
                .output()
                .expect("sh runs")
        };

        assert_eq!(
            failure_text(&output("echo 'nothing to commit'; exit 1")),
            "nothing to commit"
        );
        assert_eq!(
            failure_text(&output("echo hook said; echo git said >&2; exit 1")),
            "git said\nhook said"
        );
        assert_eq!(
            failure_text(&output("echo only err >&2; exit 1")),
            "only err"
        );
    }

    /// What hangs is usually the ssh git started, which keeps the output
    /// pipes open; giving up on git alone would leave the caller waiting on
    /// those pipes anyway.
    #[test]
    fn a_command_that_hangs_is_killed_with_everything_it_started() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & sleep 30"]);
        let started = Instant::now();

        let out = output_with_timeout(command, Duration::from_millis(200)).unwrap();

        assert!(
            out.is_none(),
            "a command past its timeout is reported as such"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "gave up after {:?}",
            started.elapsed()
        );
    }
}
