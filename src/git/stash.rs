//! The stash as the user sees it: listing it, adding to it, and applying,
//! popping or dropping one entry.
//!
//! Entries are named by their commit rather than by `stash@{n}`: lg stashes on
//! its own before a pull or a flow, and an index read a moment ago can point at
//! a different entry by the time it is acted on.

use anyhow::Result;

use super::{run, run_combined};

/// What lg's own stashes start with, so the list can tell them apart from the
/// user's. A flow that stopped on a conflict, or a pull that failed, can leave
/// one behind.
const LG_STASH_TAGS: [&str; 2] = ["lg flow: auto-stash", "lg: auto-stash"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// Where it sits in `git stash list` right now.
    pub index: usize,
    pub sha: String,
    /// The reflog subject: `On main: message`, or `WIP on main: ...`.
    pub subject: String,
    /// How long ago it was made, as git words it.
    pub age: String,
    /// lg took this one itself, on the way into a pull or a flow.
    pub auto: bool,
}

impl StashEntry {
    pub fn short_sha(&self) -> &str {
        self.sha.get(..7).unwrap_or(&self.sha)
    }
}

pub fn stash_list() -> Result<Vec<StashEntry>> {
    let out = run(&["stash", "list", "--format=%H%x1f%gs%x1f%cr"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .filter_map(|(index, line)| {
            let mut parts = line.splitn(3, '\x1f');
            let sha = parts.next()?.trim().to_string();
            let subject = parts.next().unwrap_or_default().trim().to_string();
            let age = parts.next().unwrap_or_default().trim().to_string();
            let auto = LG_STASH_TAGS.iter().any(|tag| subject.contains(tag));
            Some(StashEntry {
                index,
                sha,
                subject,
                age,
                auto,
            })
        })
        .collect())
}

/// Stash every change, untracked files included, under `message` when one is
/// given.
pub fn stash_push(message: &str) -> Result<String> {
    let message = message.trim();
    let mut args = vec!["stash", "push", "--include-untracked"];
    if !message.is_empty() {
        args.extend(["-m", message]);
    }
    let out = run_combined(&args)?;
    Ok(last_line(&out, "stashed"))
}

/// `stash@{n}` for the entry whose commit is `sha`, as the list stands now.
fn stash_ref(sha: &str) -> Result<String> {
    stash_list()?
        .into_iter()
        .find(|entry| entry.sha == sha)
        .map(|entry| format!("stash@{{{}}}", entry.index))
        .ok_or_else(|| anyhow::anyhow!("that stash is gone; the list was read again"))
}

/// Put an entry's changes back in the working tree and keep the entry.
pub fn stash_apply(sha: &str) -> Result<String> {
    let reference = stash_ref(sha)?;
    let out = run_combined(&["stash", "apply", &reference])?;
    Ok(format!(
        "applied {reference}{}",
        conflict_note(&out).unwrap_or_default()
    ))
}

/// Put an entry's changes back and drop it. Git keeps the entry when applying
/// it conflicts, and so does this.
pub fn stash_pop(sha: &str) -> Result<String> {
    let reference = stash_ref(sha)?;
    let out = run_combined(&["stash", "pop", &reference])?;
    Ok(format!(
        "popped {reference}{}",
        conflict_note(&out).unwrap_or_default()
    ))
}

/// Throw an entry away.
pub fn stash_drop(sha: &str) -> Result<String> {
    let reference = stash_ref(sha)?;
    run_combined(&["stash", "drop", &reference])?;
    Ok(format!("dropped {reference}"))
}

fn conflict_note(out: &str) -> Option<&'static str> {
    out.contains("CONFLICT").then_some(" with conflicts")
}

fn last_line(out: &str, fallback: &str) -> String {
    out.lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_string()
}
