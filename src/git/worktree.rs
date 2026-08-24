//! Linked worktrees: the other checkouts of this repository.
//!
//! Every worktree shares one git directory, so `git worktree list` returns the
//! same set no matter which of them it is asked from — including the main
//! worktree, which git always lists first.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE};

use super::{run, run_combined, run_combined_in_dir, run_in_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Absolute path to the checkout.
    pub path: String,
    /// Short branch name, or `None` when the worktree has a detached HEAD.
    pub branch: Option<String>,
    /// Commit HEAD points at. Empty for a bare repository.
    pub head: String,
    /// The worktree the repository was created in, which cannot be removed.
    pub is_main: bool,
    pub bare: bool,
    /// Lock reason when locked; an empty string when locked without one.
    pub locked: Option<String>,
    /// Why git considers the worktree removable, when it does — usually a
    /// checkout whose directory is gone.
    pub prunable: Option<String>,
    pub has_changes: bool,
}

impl Worktree {
    /// What to call this worktree: its branch, or the directory it lives in
    /// when HEAD is detached.
    pub fn label(&self) -> String {
        match &self.branch {
            Some(branch) => branch.clone(),
            None => self.dir_name(),
        }
    }

    /// Last path component, which is how worktrees are told apart on disk.
    pub fn dir_name(&self) -> String {
        Path::new(&self.path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.clone())
    }

    /// A worktree whose directory git can no longer find. Switching to one
    /// would fail, so callers offer to prune it instead.
    pub fn is_missing(&self) -> bool {
        self.prunable.is_some() && !Path::new(&self.path).is_dir()
    }
}

/// Worktrees of the repository git commands currently point at, with their
/// dirty state filled in. Worktrees whose directory is gone are reported as
/// clean rather than as an error.
pub fn worktrees() -> Result<Vec<Worktree>> {
    let out = run(&["worktree", "list", "--porcelain"])?;
    let mut worktrees = parse_worktree_list(&String::from_utf8_lossy(&out.stdout));
    for worktree in &mut worktrees {
        worktree.has_changes = worktree_has_changes(Path::new(&worktree.path)).unwrap_or(false);
    }
    Ok(worktrees)
}

/// The git directory every worktree of this repository shares. A linked
/// worktree keeps its own state under it, and commits land in it.
pub fn common_git_dir() -> Result<PathBuf> {
    let out = run(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if path.is_empty() {
        anyhow::bail!("git did not report a common git directory");
    }
    Ok(PathBuf::from(path))
}

/// The checkout the repository was created in — where the git directory lives.
pub fn main_worktree() -> Result<PathBuf> {
    let out = run(&["worktree", "list", "--porcelain"])?;
    parse_worktree_list(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .find(|worktree| worktree.is_main)
        .map(|worktree| PathBuf::from(worktree.path))
        .context("no main worktree reported")
}

fn worktree_has_changes(dir: &Path) -> Result<bool> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let out = run_in_dir(dir, &["status", "--porcelain"])?;
    Ok(!out.stdout.is_empty())
}

/// Add a worktree at `path`. A branch that already exists locally is checked
/// out there; any other name is created from `base`. Git creates the leaf and
/// any missing parent directories itself.
pub fn worktree_add(path: &Path, branch: &str, base: &str) -> Result<String> {
    let branch = branch.trim();
    if branch.is_empty() {
        anyhow::bail!("branch name cannot be empty");
    }
    if !is_valid_branch_name(branch) {
        anyhow::bail!("invalid branch name: {branch}");
    }
    let path = path.to_string_lossy().into_owned();
    if local_branch_exists(branch) {
        return run_combined(&["worktree", "add", &path, branch]);
    }

    let base = base.trim();
    if base.is_empty() {
        anyhow::bail!("a new branch needs a base to start from");
    }
    if !ref_exists(base) {
        anyhow::bail!("unknown base ref: {base}");
    }
    run_combined(&["worktree", "add", "--no-track", "-b", branch, &path, base])
}

/// Remove a worktree. Git refuses while it holds uncommitted work unless
/// `force` is set, and that refusal is worth surfacing rather than overriding.
pub fn worktree_remove(path: &Path, force: bool) -> Result<String> {
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    let out = run_combined(&args)?;
    Ok(if out.trim().is_empty() {
        format!("removed worktree {path}")
    } else {
        out
    })
}

/// Merge a worktree's branch into `main` and clean up after it: push `main`,
/// remove the worktree, and delete the branch here and on the remote.
///
/// The merge runs in whichever checkout already holds `main` — git never lets
/// two worktrees share a branch, so that is never the worktree being landed.
/// Every checkout involved has to be clean before anything moves, and a
/// conflicting merge is aborted rather than left half-done, so a failure never
/// strands work. Running it again after fixing what failed carries on from
/// where it stopped.
pub fn worktree_land(path: &Path, branch: &str) -> Result<String> {
    let worktrees = worktrees()?;
    let branch = movable_branch(&worktrees, path, branch)?;
    let host = main_branch_host(&worktrees)?;
    let mut steps = Vec::new();

    // Being offline is no reason to refuse a local merge, so a failed fetch
    // only means `main` is compared against what the last fetch left behind.
    let _ = run_in_dir(&host, &["fetch", DEFAULT_PUSH_REMOTE, "--prune"]);
    let remote_main = format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    if ref_exists_in(&host, &remote_main) && behind_count(&host, &remote_main)? > 0 {
        run_in_dir(&host, &["merge", "--ff-only", &remote_main]).with_context(|| {
            format!("{BRANCH_MAIN} has diverged from {remote_main}; reconcile those first")
        })?;
        steps.push(format!("updated {BRANCH_MAIN} from {remote_main}"));
    }

    match run_combined_in_dir(&host, &["merge", "--no-edit", &branch]) {
        Ok(out) => steps.push(last_line(&out, &format!("merged {branch}"))),
        Err(err) => {
            // A conflicted `main` sitting in a checkout nobody is looking at is
            // worse than not having merged at all.
            let _ = run_in_dir(&host, &["merge", "--abort"]);
            anyhow::bail!(
                "{err}\nmerge {branch} into {BRANCH_MAIN} by hand in {}",
                host.display()
            );
        }
    }

    if ref_exists_in(&host, &format!("{BRANCH_MAIN}@{{u}}")) {
        run_combined_in_dir(&host, &["push", DEFAULT_PUSH_REMOTE, BRANCH_MAIN])
            .context("merged, but the push failed; push it and run this again to clean up")?;
        steps.push(format!("pushed {BRANCH_MAIN}"));
    }

    // The worktree has to let go of the branch before git will delete it.
    worktree_remove(path, false)?;
    steps.push(format!("removed {}", path.display()));
    run_in_dir(&host, &["branch", "-d", &branch])?;
    steps.push(format!("deleted {branch}"));

    if ref_exists_in(
        &host,
        &format!("refs/remotes/{DEFAULT_PUSH_REMOTE}/{branch}"),
    ) {
        // The branch is merged and pushed by this point, so a remote that
        // refuses the delete is worth a note rather than failing the whole run.
        match run_combined_in_dir(&host, &["push", DEFAULT_PUSH_REMOTE, "--delete", &branch]) {
            Ok(_) => steps.push(format!("deleted {DEFAULT_PUSH_REMOTE}/{branch}")),
            Err(err) => steps.push(format!("kept {DEFAULT_PUSH_REMOTE}/{branch}: {err}")),
        }
    }

    Ok(steps.join("; "))
}

/// Move a worktree's branch back to the main checkout: remove the worktree and
/// check the branch out where the repository was cloned. Nothing is merged and
/// the branch keeps living — this is for carrying on with the work in one
/// place, not for finishing it.
pub fn worktree_bring_home(path: &Path, branch: &str) -> Result<String> {
    let worktrees = worktrees()?;
    let branch = movable_branch(&worktrees, path, branch)?;
    let main = worktrees
        .iter()
        .find(|worktree| worktree.is_main)
        .context("no main worktree reported")?;
    if main.has_changes {
        anyhow::bail!("commit or stash the changes in {} first", main.path);
    }
    let home = PathBuf::from(&main.path);

    // Git refuses to check out a branch another worktree still claims, so the
    // worktree goes first. It is clean, so a checkout that then fails costs
    // nothing beyond leaving the branch where it already was.
    worktree_remove(path, false)?;
    run_in_dir(&home, &["checkout", &branch]).with_context(|| {
        format!(
            "removed the worktree, but checking {branch} out in {} failed",
            main.path
        )
    })?;
    Ok(format!(
        "removed {}; checked out {branch} in {}",
        path.display(),
        main.path
    ))
}

/// The branch a worktree can hand over: it has to be a linked checkout, still
/// on the `expected` branch, unlocked, and holding nothing uncommitted. The
/// branch is checked because it is what the user was asked to confirm, and a
/// worktree can be switched to another one in between.
fn movable_branch(worktrees: &[Worktree], path: &Path, expected: &str) -> Result<String> {
    let source = worktrees
        .iter()
        .find(|worktree| same_dir(Path::new(&worktree.path), path))
        .with_context(|| format!("{} is not a worktree of this repository", path.display()))?;
    if source.is_main {
        anyhow::bail!("the main checkout has no branch to hand over");
    }
    if source.locked.is_some() {
        anyhow::bail!("{} is locked", source.label());
    }
    let branch = source
        .branch
        .clone()
        .context("a detached worktree has no branch to hand over")?;
    if branch == BRANCH_MAIN {
        anyhow::bail!("{BRANCH_MAIN} is not a branch to move off a worktree");
    }
    if branch != expected {
        anyhow::bail!("{} is on {branch} now, not {expected}", source.dir_name());
    }
    if source.has_changes {
        anyhow::bail!("commit or discard the changes in {branch} first");
    }
    Ok(branch)
}

/// The checkout a merge into `main` runs in: whichever worktree already holds
/// it, or the main worktree with `main` checked out into it when none does.
fn main_branch_host(worktrees: &[Worktree]) -> Result<PathBuf> {
    if let Some(host) = worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(BRANCH_MAIN))
    {
        if host.has_changes {
            anyhow::bail!("commit or stash the changes in {} first", host.path);
        }
        return Ok(PathBuf::from(&host.path));
    }

    let main = worktrees
        .iter()
        .find(|worktree| worktree.is_main)
        .context("no main worktree reported")?;
    if main.has_changes {
        anyhow::bail!("commit or stash the changes in {} first", main.path);
    }
    let dir = PathBuf::from(&main.path);
    run_in_dir(&dir, &["checkout", BRANCH_MAIN])
        .with_context(|| format!("no checkout holds {BRANCH_MAIN}"))?;
    Ok(dir)
}

/// Forget worktrees whose directories are gone.
pub fn worktree_prune() -> Result<String> {
    let out = run_combined(&["worktree", "prune", "-v"])?;
    Ok(if out.trim().is_empty() {
        "nothing to prune".to_string()
    } else {
        out
    })
}

/// Where a worktree for `branch` goes by default: a sibling of the main
/// worktree, named after it, so the repository never contains a checkout of
/// itself and one terrarium profile per worktree stays possible.
pub fn default_worktree_path(main_worktree: &Path, branch: &str) -> PathBuf {
    let name = main_worktree
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    let parent = main_worktree
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    parent
        .join(format!("{name}.worktrees"))
        .join(worktree_slug(branch))
}

/// Directory name for a branch: `feat/x` becomes `feat-x`, and anything a path
/// component should not carry is folded into a dash.
pub fn worktree_slug(branch: &str) -> String {
    let mut slug = String::with_capacity(branch.len());
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches(['-', '.']).to_string();
    if slug.is_empty() {
        "worktree".to_string()
    } else {
        slug
    }
}

/// The ref a new branch should start from: the remote's main branch when it is
/// known, which is what the branch flows already assume.
pub fn preferred_base_ref() -> String {
    let remote_main = format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}");
    if ref_exists(&remote_main) {
        remote_main
    } else {
        BRANCH_MAIN.to_string()
    }
}

/// Compare two directories, allowing for one side being reached through a
/// symlink — a workspace of symlinked repositories is the normal case.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn ref_exists_in(dir: &Path, name: &str) -> bool {
    run_in_dir(dir, &["rev-parse", "--verify", "--quiet", name]).is_ok()
}

/// How many commits `ahead_of` carries that this checkout's HEAD does not.
fn behind_count(dir: &Path, ahead_of: &str) -> Result<u32> {
    let out = run_in_dir(dir, &["rev-list", "--count", ahead_of, "--not", "HEAD"])?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .context("parsing how far behind its remote main is")
}

/// Git's own last word on what it did, for a status line with room for one
/// short phrase per step.
fn last_line(output: &str, fallback: &str) -> String {
    output
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_string()
}

fn local_branch_exists(branch: &str) -> bool {
    ref_exists(&format!("refs/heads/{branch}"))
}

fn ref_exists(name: &str) -> bool {
    run(&["rev-parse", "--verify", "--quiet", name]).is_ok()
}

fn is_valid_branch_name(branch: &str) -> bool {
    run(&["check-ref-format", "--branch", branch]).is_ok()
}

/// Parse `git worktree list --porcelain`: blank-line separated records, each
/// starting with `worktree <path>`. The first record is the main worktree.
pub fn parse_worktree_list(porcelain: &str) -> Vec<Worktree> {
    let mut worktrees: Vec<Worktree> = Vec::new();
    for line in porcelain.lines() {
        let line = line.trim_end();
        let (key, value) = match line.split_once(' ') {
            Some((key, value)) => (key, value.trim()),
            None => (line, ""),
        };
        match key {
            "worktree" => {
                if value.is_empty() {
                    continue;
                }
                worktrees.push(Worktree {
                    path: value.to_string(),
                    branch: None,
                    head: String::new(),
                    is_main: worktrees.is_empty(),
                    bare: false,
                    locked: None,
                    prunable: None,
                    has_changes: false,
                });
            }
            "HEAD" => {
                if let Some(current) = worktrees.last_mut() {
                    current.head = value.to_string();
                }
            }
            "branch" => {
                if let Some(current) = worktrees.last_mut() {
                    current.branch = Some(short_branch_name(value));
                }
            }
            "detached" => {
                if let Some(current) = worktrees.last_mut() {
                    current.branch = None;
                }
            }
            "bare" => {
                if let Some(current) = worktrees.last_mut() {
                    current.bare = true;
                }
            }
            "locked" => {
                if let Some(current) = worktrees.last_mut() {
                    current.locked = Some(value.to_string());
                }
            }
            "prunable" => {
                if let Some(current) = worktrees.last_mut() {
                    current.prunable = Some(value.to_string());
                }
            }
            _ => {}
        }
    }
    worktrees
}

fn short_branch_name(refname: &str) -> String {
    refname
        .strip_prefix("refs/heads/")
        .unwrap_or(refname)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
worktree /repo
HEAD e6b8e0af7c208f06fafa41ec712a0bd4dd8a8045
branch refs/heads/main

worktree /repo.worktrees/detached
HEAD e6b8e0af7c208f06fafa41ec712a0bd4dd8a8045
detached

worktree /repo.worktrees/feature
HEAD abc1230000000000000000000000000000000000
branch refs/heads/feat/x

worktree /repo.worktrees/held
HEAD abc1230000000000000000000000000000000000
branch refs/heads/held
locked busy building

worktree /repo.worktrees/gone
HEAD abc1230000000000000000000000000000000000
branch refs/heads/gone
prunable gitdir file points to non-existent location
";

    #[test]
    fn parses_every_record_in_order() {
        let worktrees = parse_worktree_list(SAMPLE);
        let paths: Vec<_> = worktrees.iter().map(|w| w.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/repo",
                "/repo.worktrees/detached",
                "/repo.worktrees/feature",
                "/repo.worktrees/held",
                "/repo.worktrees/gone",
            ]
        );
    }

    #[test]
    fn only_the_first_record_is_the_main_worktree() {
        let worktrees = parse_worktree_list(SAMPLE);
        assert!(worktrees[0].is_main);
        assert!(worktrees[1..].iter().all(|worktree| !worktree.is_main));
    }

    #[test]
    fn branch_names_lose_their_ref_prefix() {
        let worktrees = parse_worktree_list(SAMPLE);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(worktrees[2].branch.as_deref(), Some("feat/x"));
    }

    #[test]
    fn a_detached_worktree_has_no_branch_and_falls_back_to_its_directory() {
        let worktrees = parse_worktree_list(SAMPLE);
        assert_eq!(worktrees[1].branch, None);
        assert_eq!(worktrees[1].label(), "detached");
    }

    #[test]
    fn lock_and_prune_reasons_are_kept() {
        let worktrees = parse_worktree_list(SAMPLE);
        assert_eq!(worktrees[3].locked.as_deref(), Some("busy building"));
        assert_eq!(
            worktrees[4].prunable.as_deref(),
            Some("gitdir file points to non-existent location")
        );
        assert!(worktrees[2].locked.is_none());
        assert!(worktrees[2].prunable.is_none());
    }

    #[test]
    fn a_lock_without_a_reason_still_reads_as_locked() {
        let worktrees =
            parse_worktree_list("worktree /repo\nHEAD abc\nbranch refs/heads/x\nlocked");
        assert_eq!(worktrees[0].locked.as_deref(), Some(""));
    }

    #[test]
    fn a_bare_repository_record_is_marked_bare() {
        let worktrees = parse_worktree_list("worktree /repo.git\nbare");
        assert!(worktrees[0].bare);
        assert!(worktrees[0].head.is_empty());
        assert_eq!(worktrees[0].branch, None);
    }

    #[test]
    fn empty_output_is_no_worktrees() {
        assert!(parse_worktree_list("").is_empty());
        assert!(parse_worktree_list("\n\n").is_empty());
    }

    #[test]
    fn slugs_are_path_safe_and_readable() {
        assert_eq!(worktree_slug("feat/x"), "feat-x");
        assert_eq!(
            worktree_slug("feature/JIRA-42_thing"),
            "feature-JIRA-42_thing"
        );
        assert_eq!(worktree_slug("release/1.2.3"), "release-1.2.3");
        assert_eq!(worktree_slug("a//b"), "a-b", "runs collapse to one dash");
        assert_eq!(worktree_slug("/leading/"), "leading");
        assert_eq!(worktree_slug("..."), "worktree", "never an empty component");
        assert_eq!(worktree_slug("with space"), "with-space");
    }

    #[test]
    fn the_default_path_is_a_sibling_of_the_main_worktree() {
        assert_eq!(
            default_worktree_path(Path::new("/Users/me/dev/lg"), "feat/x"),
            Path::new("/Users/me/dev/lg.worktrees/feat-x")
        );
    }

    #[test]
    fn the_default_path_survives_a_repository_at_the_filesystem_root() {
        assert_eq!(
            default_worktree_path(Path::new("/repo"), "main"),
            Path::new("/repo.worktrees/main")
        );
    }

    #[test]
    fn a_missing_directory_is_only_reported_when_git_calls_it_prunable() {
        let present = Worktree {
            path: "/repo".into(),
            branch: Some("main".into()),
            head: "abc".into(),
            is_main: true,
            bare: false,
            locked: None,
            prunable: None,
            has_changes: false,
        };
        assert!(!present.is_missing());

        let gone = Worktree {
            path: "/repo.worktrees/gone".into(),
            prunable: Some("gitdir file points to non-existent location".into()),
            ..present
        };
        assert!(gone.is_missing());
    }
}
