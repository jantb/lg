//! Reading commits: the log, the graph rows, and what is unpushed.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;

use super::{head_branch, run, run_in_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub author: String,
    pub author_short: String,
    pub parents: Vec<String>,
    pub is_first_parent: bool,
    pub subject: String,
}

impl Commit {
    pub fn parent_count(&self) -> usize {
        self.parents.len()
    }

    pub fn is_graph_row(&self) -> bool {
        false
    }
}

impl crate::graph::CommitNode for Commit {
    fn sha(&self) -> &str {
        &self.sha
    }
    fn parents(&self) -> &[String] {
        &self.parents
    }
    fn is_first_parent(&self) -> bool {
        self.is_first_parent
    }
}

/// Recent commit messages, subject and body, as one blob. Used to derive this
/// checkout's writing conventions rather than assuming the defaults.
pub fn recent_commit_messages(limit: usize) -> Result<String> {
    let n = limit.to_string();
    let out = run(&["log", "-n", &n, "--no-merges", "--format=%B%x1e"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let messages: Vec<String> = text
        .split('\x1e')
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .map(|message| message.to_string())
        .collect();
    Ok(messages.join("\n---\n"))
}

pub fn list_commits(limit: usize) -> Result<Vec<Commit>> {
    list_commits_for_ref("HEAD", limit)
}

pub fn list_commits_for_ref(reference: &str, limit: usize) -> Result<Vec<Commit>> {
    let n = limit.to_string();
    let first_parent = first_parent_shas(reference, limit).unwrap_or_default();
    let fmt = "--format=%x1f%h%x1f%an%x1f%p%x1f%s";
    let result = run(&["log", fmt, "-n", &n, reference]);
    match result {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut commits = Vec::new();
            for line in text.lines() {
                let Some(marker) = line.find('\x1f') else {
                    continue;
                };
                let mut parts = line[marker + 1..].splitn(4, '\x1f');
                let Some(sha) = parts.next().map(str::trim).map(str::to_owned) else {
                    continue;
                };
                if sha.is_empty() {
                    continue;
                }
                let author = parts.next().unwrap_or("").trim().to_owned();
                let parents_str = parts.next().unwrap_or("").trim();
                let subject = parts.next().unwrap_or("").trim().to_owned();
                let parents: Vec<String> =
                    parents_str.split_whitespace().map(str::to_owned).collect();
                let is_first_parent = first_parent.contains(&sha);
                commits.push(Commit {
                    sha,
                    author_short: short_author_name(&author),
                    author,
                    parents,
                    is_first_parent,
                    subject,
                });
            }
            Ok(commits)
        }
        Err(e) => {
            let msg = e.to_string();
            if is_empty_commit_history_error(reference, &msg) {
                Ok(vec![])
            } else {
                Err(e)
            }
        }
    }
}

fn is_empty_commit_history_error(reference: &str, msg: &str) -> bool {
    if msg.contains("does not have any commits") || msg.contains("no commits yet") {
        return true;
    }

    let looks_like_unborn_ref = msg.contains("unknown revision")
        || msg.contains("ambiguous argument")
        || msg.contains("bad default revision");
    if !looks_like_unborn_ref {
        return false;
    }

    reference == "HEAD" || current_unborn_branch().as_deref() == Some(reference)
}

fn current_unborn_branch() -> Option<String> {
    head_branch().ok()
}

fn first_parent_shas(reference: &str, limit: usize) -> Result<HashSet<String>> {
    let n = limit.to_string();
    let out = run(&[
        "rev-list",
        "--first-parent",
        "--abbrev-commit",
        "-n",
        &n,
        reference,
    ])?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn short_author_name(author: &str) -> String {
    let trimmed = author.trim();
    let parts: Vec<&str> = trimmed
        .split_whitespace()
        .map(|part| part.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() >= 2 {
        return parts
            .iter()
            .take(2)
            .filter_map(|part| part.chars().next())
            .flat_map(char::to_uppercase)
            .take(2)
            .collect();
    }
    parts
        .first()
        .copied()
        .unwrap_or(trimmed)
        .chars()
        .take(2)
        .collect()
}

pub(super) fn preferred_commit_ref(remote_ref: &str, local_ref: &str) -> Option<String> {
    if commit_ref_exists(remote_ref) {
        Some(remote_ref.to_string())
    } else if commit_ref_exists(local_ref) {
        Some(local_ref.to_string())
    } else {
        None
    }
}

pub(super) fn preferred_commit_ref_in_dir(
    dir: &Path,
    remote_ref: &str,
    local_ref: &str,
) -> Option<String> {
    if commit_ref_exists_in_dir(dir, remote_ref) {
        Some(remote_ref.to_string())
    } else if commit_ref_exists_in_dir(dir, local_ref) {
        Some(local_ref.to_string())
    } else {
        None
    }
}

fn commit_ref_exists(reference: &str) -> bool {
    run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{reference}^{{commit}}"),
    ])
    .is_ok()
}

fn commit_ref_exists_in_dir(dir: &Path, reference: &str) -> bool {
    run_in_dir(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{reference}^{{commit}}"),
        ],
    )
    .is_ok()
}

pub(super) fn commit_oid(reference: &str) -> Option<String> {
    let out = run(&["rev-parse", "--verify", &format!("{reference}^{{commit}}")]).ok()?;
    let oid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!oid.is_empty()).then_some(oid)
}

pub(super) fn rev_list(args: &[&str]) -> Result<Vec<String>> {
    let mut cmd = vec!["rev-list"];
    cmd.extend_from_slice(args);
    let out = run(&cmd)?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn unpushed_shas() -> Result<std::collections::HashSet<String>> {
    match run(&["rev-list", "--abbrev-commit", "@{u}..HEAD"]) {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            Ok(text
                .lines()
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect())
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("no upstream")
                || msg.contains("unknown revision")
                || msg.contains("ambiguous argument '@{u}'")
            {
                Ok(std::collections::HashSet::new())
            } else {
                Err(e)
            }
        }
    }
}

pub fn counts_ahead_behind() -> Result<(u32, u32)> {
    let out = run(&["rev-list", "--left-right", "--count", "@{u}...HEAD"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    let mut parts = text.splitn(2, '\t');
    let behind: u32 = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("unexpected rev-list output: {text}"))?
        .trim()
        .parse()
        .context("parsing behind count")?;
    let ahead: u32 = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("unexpected rev-list output: {text}"))?
        .trim()
        .parse()
        .context("parsing ahead count")?;
    Ok((ahead, behind))
}
