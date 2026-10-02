//! The local and remote branch lists, and how far each is behind main.

use anyhow::Result;
use std::path::Path;

use super::commits::{preferred_commit_ref, preferred_commit_ref_in_dir};
use super::nested::{nested_repo_dir, nested_repo_dir_at};
use super::{run, run_in_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub upstream_gone: bool,
    pub ahead: u32,
    pub behind: u32,
    pub behind_main: u32,
    pub last_commit_unix: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranch {
    pub name: String,
    pub remote: String,
    pub local_name: String,
    pub last_commit_unix: Option<i64>,
}

/// The names of the local branches, without any status. Empty outside a
/// repository, so callers that only need to know what exists never fail.
pub fn local_branch_names() -> Vec<String> {
    run(&["branch", "--format=%(refname:short)"])
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// What `git branch` is asked to print for every branch, one field per
/// branch separated by unit separators.
const BRANCH_FORMAT: &str = "%(refname:short)\x1f%(HEAD)\x1f%(upstream:short)\x1f%(upstream:track)\x1f%(committerdate:unix)";

pub fn list_branches() -> Result<Vec<Branch>> {
    list_branches_at(None)
}

pub fn nested_repo_branches(repo_path: &str) -> Result<Vec<Branch>> {
    let dir = nested_repo_dir(repo_path)?;
    list_branches_in_dir(&dir)
}

pub fn nested_repo_branches_at(root: &Path, repo_path: &str) -> Result<Vec<Branch>> {
    let dir = nested_repo_dir_at(root, repo_path)?;
    list_branches_in_dir(&dir)
}

fn list_branches_in_dir(dir: &Path) -> Result<Vec<Branch>> {
    list_branches_at(Some(dir))
}

/// Git in `dir`, or in the checkout lg is pointed at.
fn git_at(dir: Option<&Path>, args: &[&str]) -> Result<std::process::Output> {
    match dir {
        Some(dir) => run_in_dir(dir, args),
        None => run(args),
    }
}

/// The local branches in `dir` (or the active checkout), newest first, each
/// with how far it trails main.
///
/// How far behind main every branch is comes from the same `git branch` call
/// through `%(ahead-behind:...)`, rather than one `rev-list` per branch. Git
/// older than 2.41 does not know that atom and refuses the whole format, and
/// then the counts are taken the old way, one branch at a time.
fn list_branches_at(dir: Option<&Path>) -> Result<Vec<Branch>> {
    let configured_base = crate::preferences::base_branch();
    let configured_remote = crate::preferences::remote();
    let remote_main = format!("{configured_remote}/{configured_base}");
    let main_ref = match dir {
        Some(dir) => preferred_commit_ref_in_dir(dir, &remote_main, configured_base.as_str()),
        None => preferred_commit_ref(&remote_main, configured_base.as_str()),
    };
    let counted = main_ref
        .as_deref()
        // A ref name may hold a `)`, which would end the atom early.
        .filter(|main| !main.contains(')'))
        .and_then(|main| {
            let format = format!("--format={BRANCH_FORMAT}\x1f%(ahead-behind:{main})");
            git_at(dir, &["branch", &format]).ok()
        });
    let (out, counted) = match counted {
        Some(out) => (out, true),
        None => (
            git_at(dir, &["branch", &format!("--format={BRANCH_FORMAT}")])?,
            false,
        ),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut branches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(6, '\x1f');
            let name = parts.next()?.trim().to_owned();
            let head = parts.next()?.trim();
            let upstream = parts.next().unwrap_or("").trim();
            let track = parts.next().unwrap_or("").trim();
            let (ahead, behind) = parse_upstream_track(track);
            let last_commit_unix = parse_unix_timestamp(parts.next().unwrap_or("").trim());
            if name.is_empty() {
                return None;
            }
            let counts = parts.next().unwrap_or("");
            let behind_main = if !counts_against_main(&name, main_ref.as_deref()) {
                0
            } else if counted {
                parse_ahead_behind(counts).map_or(0, |(_, behind)| behind)
            } else {
                branch_behind_main(dir, &name, main_ref.as_deref())
            };
            Some(Branch {
                name,
                is_current: head == "*",
                upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
                upstream_gone: track.contains("gone"),
                ahead,
                behind,
                behind_main,
                last_commit_unix,
            })
        })
        .collect();
    sort_refs_by_recent_commit(
        &mut branches,
        |branch| branch.last_commit_unix,
        |branch| branch.name.as_str(),
    );
    Ok(branches)
}

/// How far every local branch is ahead of and behind `base`, by branch name,
/// from one `for-each-ref` call. `None` when git is too old to say that way,
/// or `base` does not resolve.
pub(super) fn ahead_behind_all(
    base: &str,
) -> Option<std::collections::HashMap<String, (u32, u32)>> {
    if base.contains(')') {
        return None;
    }
    let format = format!("--format=%(refname:short)\x1f%(ahead-behind:{base})");
    let out = run(&["for-each-ref", "refs/heads", &format]).ok()?;
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let (name, counts) = line.split_once('\x1f')?;
                Some((name.trim().to_owned(), parse_ahead_behind(counts)?))
            })
            .collect(),
    )
}

/// `%(ahead-behind:...)`'s answer: commits ahead, then commits behind.
fn parse_ahead_behind(value: &str) -> Option<(u32, u32)> {
    let mut counts = value.split_whitespace().map(str::parse::<u32>);
    match (counts.next(), counts.next(), counts.next()) {
        (Some(Ok(ahead)), Some(Ok(behind)), None) => Some((ahead, behind)),
        _ => None,
    }
}

/// Whether `branch` is one whose distance from main means anything: not main
/// itself, and not the detached-HEAD row `git branch` lists in brackets.
fn counts_against_main(branch: &str, main_ref: Option<&str>) -> bool {
    let configured_base = crate::preferences::base_branch();
    main_ref.is_some_and(|main_ref| {
        branch != configured_base.as_str() && branch != main_ref && !branch.starts_with('(')
    })
}

pub fn list_remote_branches() -> Result<Vec<RemoteBranch>> {
    let out = run(&[
        "for-each-ref",
        "refs/remotes",
        "--format=%(refname:short)\x1f%(committerdate:unix)",
    ])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut branches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\x1f');
            let name = parts.next()?.trim().to_owned();
            if name.is_empty() || name.ends_with("/HEAD") {
                return None;
            }
            let (remote, local_name) = name.split_once('/')?;
            let remote = remote.to_owned();
            let local_name = local_name.to_owned();
            Some(RemoteBranch {
                name,
                remote,
                local_name,
                last_commit_unix: parse_unix_timestamp(parts.next().unwrap_or("").trim()),
            })
        })
        .collect();
    sort_refs_by_recent_commit(
        &mut branches,
        |branch| branch.last_commit_unix,
        |branch| branch.name.as_str(),
    );
    Ok(branches)
}

pub fn nested_repo_remote_branches(repo_path: &str) -> Result<Vec<RemoteBranch>> {
    let dir = nested_repo_dir(repo_path)?;
    list_remote_branches_in_dir(&dir)
}

pub fn nested_repo_remote_branches_at(root: &Path, repo_path: &str) -> Result<Vec<RemoteBranch>> {
    let dir = nested_repo_dir_at(root, repo_path)?;
    list_remote_branches_in_dir(&dir)
}

fn list_remote_branches_in_dir(dir: &Path) -> Result<Vec<RemoteBranch>> {
    let out = run_in_dir(
        dir,
        &[
            "for-each-ref",
            "refs/remotes",
            "--format=%(refname:short)\x1f%(committerdate:unix)",
        ],
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut branches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\x1f');
            let name = parts.next()?.trim().to_owned();
            if name.is_empty() || name.ends_with("/HEAD") {
                return None;
            }
            let (remote, local_name) = name.split_once('/')?;
            let remote = remote.to_owned();
            let local_name = local_name.to_owned();
            Some(RemoteBranch {
                name,
                remote,
                local_name,
                last_commit_unix: parse_unix_timestamp(parts.next().unwrap_or("").trim()),
            })
        })
        .collect();
    sort_refs_by_recent_commit(
        &mut branches,
        |branch| branch.last_commit_unix,
        |branch| branch.name.as_str(),
    );
    Ok(branches)
}

fn parse_unix_timestamp(value: &str) -> Option<i64> {
    value.parse::<i64>().ok().filter(|ts| *ts > 0)
}

fn parse_upstream_track(value: &str) -> (u32, u32) {
    let text = value.trim().trim_start_matches('[').trim_end_matches(']');
    let mut ahead = 0;
    let mut behind = 0;
    for part in text.split(',').map(str::trim) {
        if let Some(count) = part.strip_prefix("ahead ") {
            ahead = count.trim().parse().unwrap_or(0);
        } else if let Some(count) = part.strip_prefix("behind ") {
            behind = count.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

/// How many commits `main_ref` has that `branch` does not, asked on its own:
/// the way it was done before git could answer for every branch at once.
fn branch_behind_main(dir: Option<&Path>, branch: &str, main_ref: Option<&str>) -> u32 {
    let Some(main_ref) = main_ref else {
        return 0;
    };
    let Ok(out) = git_at(dir, &["rev-list", "--count", main_ref, "--not", branch]) else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn sort_refs_by_recent_commit<T, F, N>(refs: &mut [T], timestamp: F, name: N)
where
    F: Fn(&T) -> Option<i64>,
    N: Fn(&T) -> &str,
{
    refs.sort_by(|a, b| {
        timestamp(b)
            .cmp(&timestamp(a))
            .then_with(|| name(a).cmp(name(b)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_upstream_track_counts() {
        assert_eq!(parse_upstream_track("[ahead 1]"), (1, 0));
        assert_eq!(parse_upstream_track("[behind 78]"), (0, 78));
        assert_eq!(parse_upstream_track("[ahead 1, behind 6]"), (1, 6));
        assert_eq!(parse_upstream_track("[gone]"), (0, 0));
        assert_eq!(parse_upstream_track(""), (0, 0));
    }

    #[test]
    fn parses_ahead_behind_counts() {
        assert_eq!(parse_ahead_behind("3 12"), Some((3, 12)));
        assert_eq!(parse_ahead_behind("0 0\n"), Some((0, 0)));
        assert_eq!(
            parse_ahead_behind("%(ahead-behind:main)"),
            None,
            "a git that printed the atom back did not count anything"
        );
        assert_eq!(parse_ahead_behind(""), None);
    }
}
