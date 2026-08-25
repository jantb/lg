//! The local and remote branch lists, and how far each is behind main.

use anyhow::Result;
use std::path::Path;

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE};

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

pub fn list_branches() -> Result<Vec<Branch>> {
    let main_ref =
        preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"), BRANCH_MAIN);
    let out = run(&[
        "branch",
        "--format=%(refname:short)\x1f%(HEAD)\x1f%(upstream:short)\x1f%(upstream:track)\x1f%(committerdate:unix)",
    ])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut branches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\x1f');
            let name = parts.next()?.trim().to_owned();
            let head = parts.next()?.trim();
            let upstream = parts.next().unwrap_or("").trim();
            let track = parts.next().unwrap_or("").trim();
            let (ahead, behind) = parse_upstream_track(track);
            let last_commit_unix = parse_unix_timestamp(parts.next().unwrap_or("").trim());
            if name.is_empty() {
                return None;
            }
            let behind_main = branch_behind_main(&name, main_ref.as_deref());
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

pub fn nested_repo_branches(repo_path: &str) -> Result<Vec<Branch>> {
    let dir = nested_repo_dir(repo_path)?;
    list_branches_in_dir(&dir)
}

pub fn nested_repo_branches_at(root: &Path, repo_path: &str) -> Result<Vec<Branch>> {
    let dir = nested_repo_dir_at(root, repo_path)?;
    list_branches_in_dir(&dir)
}

fn list_branches_in_dir(dir: &Path) -> Result<Vec<Branch>> {
    let main_ref = preferred_commit_ref_in_dir(
        dir,
        &format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"),
        BRANCH_MAIN,
    );
    let out = run_in_dir(
        dir,
        &[
            "branch",
            "--format=%(refname:short)\x1f%(HEAD)\x1f%(upstream:short)\x1f%(upstream:track)\x1f%(committerdate:unix)",
        ],
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut branches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\x1f');
            let name = parts.next()?.trim().to_owned();
            let head = parts.next()?.trim();
            let upstream = parts.next().unwrap_or("").trim();
            let track = parts.next().unwrap_or("").trim();
            let (ahead, behind) = parse_upstream_track(track);
            let last_commit_unix = parse_unix_timestamp(parts.next().unwrap_or("").trim());
            if name.is_empty() {
                return None;
            }
            let behind_main = branch_behind_main_in_dir(dir, &name, main_ref.as_deref());
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

fn branch_behind_main(branch: &str, main_ref: Option<&str>) -> u32 {
    let Some(main_ref) = main_ref else {
        return 0;
    };
    if branch == BRANCH_MAIN || branch == main_ref {
        return 0;
    }
    let Ok(out) = run(&["rev-list", "--count", main_ref, "--not", branch]) else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn branch_behind_main_in_dir(dir: &Path, branch: &str, main_ref: Option<&str>) -> u32 {
    let Some(main_ref) = main_ref else {
        return 0;
    };
    if branch == BRANCH_MAIN || branch == main_ref {
        return 0;
    }
    let Ok(out) = run_in_dir(dir, &["rev-list", "--count", main_ref, "--not", branch]) else {
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
}
