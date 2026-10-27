//! Collecting the branch diff from git, untracked files included.

use anyhow::Result;
use std::path::Path;

use crate::git::attrs::suppress_generated_diff;
use crate::git::{git_command, run};

use super::truncate_review_text;

pub(super) fn branch_review_commits(base_ref: &str) -> Result<Vec<String>> {
    let range = format!("{base_ref}..HEAD");
    let out = run(&[
        "log",
        "--format=%h%x1f%s%x1f%b%x1e",
        "--decorate=no",
        &range,
    ])?;
    Ok(parse_review_commits(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_review_commits(text: &str) -> Vec<String> {
    text.split('\x1e')
        .filter_map(|record| {
            let record = record.trim_matches(|ch| ch == '\n' || ch == '\r');
            if record.is_empty() {
                return None;
            }
            let mut parts = record.splitn(3, '\x1f');
            let hash = parts.next()?.trim();
            let subject = parts.next()?.trim();
            let body = compact_commit_body(parts.next().unwrap_or_default());
            let line = if body.is_empty() {
                format!("{hash} {subject}")
            } else {
                format!("{hash} {subject} - {body}")
            };
            Some(truncate_review_text(&line, 240))
        })
        .collect()
}

fn compact_commit_body(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn worktree_review_diff(base: &str) -> Result<String> {
    let out = run(&["diff", "--ignore-all-space", "--find-renames", base])?;
    let mut diff = String::from_utf8_lossy(&out.stdout).into_owned();
    for path in untracked_paths(".")? {
        let untracked = untracked_file_diff(&path)?;
        append_review_diff_part(&mut diff, &untracked);
    }
    Ok(suppress_generated_diff(&diff))
}

fn append_review_diff_part(out: &mut String, part: &str) {
    if part.trim().is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(part);
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

fn untracked_paths(pathspec: &str) -> Result<Vec<String>> {
    let out = run(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
        pathspec,
    ])?;
    Ok(out
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect())
}

fn untracked_file_diff(path: &str) -> Result<String> {
    untracked_no_index_diff(path, &["diff", "--no-index", "--", "/dev/null", path])
}

fn untracked_no_index_diff(path: &str, args: &[&str]) -> Result<String> {
    let out = git_command(args).output()?;
    if out.status.success() || out.status.code() == Some(1) {
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        normalize_no_index_path(&mut text, path);
        Ok(text)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(anyhow::anyhow!(
            "git {} failed for {path}: {}",
            args.join(" "),
            stderr.trim()
        ))
    }
}

fn normalize_no_index_path(diff: &mut String, path: &str) {
    let Some(file_name) = Path::new(path).file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let from = format!("b/{file_name}");
    let to = format!("b/{path}");
    *diff = diff.replace(&from, &to);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_review_commits_includes_compact_body_intent() {
        let commits = parse_review_commits(
            "1b7004a\x1ffix(balance): convert pending points\x1fAggregate pending transaction points into the household's base currency.\n\nSet pending balances for non-household currencies to zero.\x1e",
        );

        assert_eq!(commits.len(), 1);
        assert!(
            commits[0].contains(
                "fix(balance): convert pending points - Aggregate pending transaction points"
            ),
            "{commits:?}"
        );
        assert!(
            commits[0].contains("Set pending balances for non-household currencies to zero."),
            "{commits:?}"
        );
    }
}
