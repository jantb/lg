//! Linked worktrees: the other checkouts of this repository.
//!
//! Every worktree shares one git directory, so `git worktree list` returns the
//! same set no matter which of them it is asked from — including the main
//! worktree, which git always lists first.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE};

use super::{run, run_combined, run_in_dir};

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
