use anyhow::Result;
use std::path::Path;

use std::process::{Command, Output};

use super::attrs::suppress_generated_diff;
use super::{git_command, git_command_in_dir, run};

/// `git diff` as every diff lg parses is read: plain text with `a/` and `b/`
/// prefixes, whatever colour, prefix or external diff tool the user set up for
/// their own terminal. Hunk staging finds the file by those prefixes.
pub(super) const PLAIN_DIFF: [&str; 5] = [
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

pub(super) fn plain_diff(args: &[&str]) -> Result<Output> {
    run(&[&PLAIN_DIFF[..], args].concat())
}

pub fn staged_diff() -> Result<String> {
    let out = plain_diff(&["--cached"])?;
    Ok(suppress_generated_diff(&String::from_utf8_lossy(
        &out.stdout,
    )))
}

pub fn repo_root() -> Result<String> {
    let out = run(&["rev-parse", "--show-toplevel"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The checkout `dir` belongs to, or `None` when it belongs to none. Asked
/// about an explicit directory because startup has to answer this before any
/// repository has been picked to run git in.
pub fn repo_root_at(dir: &Path) -> Option<String> {
    let out = super::run_in_dir(dir, &["rev-parse", "--show-toplevel"]).ok()?;
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!root.is_empty()).then_some(root)
}

pub fn all_diffs() -> Result<String> {
    let cached_out = plain_diff(&["--cached"])?;
    let worktree_out = plain_diff(&[])?;
    let cached = String::from_utf8_lossy(&cached_out.stdout).into_owned();
    let worktree = with_untracked_diffs(
        String::from_utf8_lossy(&worktree_out.stdout).into_owned(),
        ".",
    )?;
    Ok(suppress_generated_diff(&format!(
        "== staged (--cached) ==\n{}\n== worktree ==\n{}",
        cached, worktree
    )))
}

pub fn file_diff(path: &str) -> Result<String> {
    let cached_out = plain_diff(&["--cached", "--", path])?;
    let worktree_out = plain_diff(&["--", path])?;
    let cached = String::from_utf8_lossy(&cached_out.stdout).into_owned();
    let worktree = with_untracked_diffs(
        String::from_utf8_lossy(&worktree_out.stdout).into_owned(),
        path,
    )?;
    Ok(suppress_generated_diff(&format!(
        "== staged (--cached) ==\n{}\n== worktree ==\n{}",
        cached, worktree
    )))
}

/// Aggregate staged + worktree diff for everything under a folder prefix.
pub fn folder_diff(prefix: &str) -> Result<String> {
    let spec = if prefix.is_empty() {
        ".".to_string()
    } else {
        format!("{prefix}/")
    };
    let cached_out = plain_diff(&["--cached", "--", &spec])?;
    let worktree_out = plain_diff(&["--", &spec])?;
    let cached = String::from_utf8_lossy(&cached_out.stdout).into_owned();
    let worktree = with_untracked_diffs(
        String::from_utf8_lossy(&worktree_out.stdout).into_owned(),
        &spec,
    )?;
    Ok(suppress_generated_diff(&format!(
        "== staged (--cached) {prefix}/ ==\n{}\n== worktree {prefix}/ ==\n{}",
        cached, worktree
    )))
}

pub fn show_commit(sha: &str) -> Result<String> {
    let format = "format:commit %H%nAuthor: %an <%ae>%nDate:   %ad%n%nMessage:%n%B%nFiles changed:";
    let out = run(&[
        "show",
        "--date=short",
        "--patch-with-stat",
        "--find-renames",
        "--root",
        &format!("--format={format}"),
        sha,
    ])?;
    Ok(label_commit_patch(&suppress_generated_diff(
        &String::from_utf8_lossy(&out.stdout),
    )))
}

pub fn branch_log(reference: &str, limit: usize) -> Result<String> {
    let n = limit.to_string();
    let out = run(&[
        "log",
        "--graph",
        "--decorate",
        "--date=relative",
        "--abbrev-commit",
        "-n",
        &n,
        reference,
        "--",
    ])?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub(super) fn label_commit_patch(text: &str) -> String {
    if let Some(pos) = text.find("\ndiff --git ") {
        let mut out = String::with_capacity(text.len() + "\nPatch:\n".len());
        out.push_str(text[..pos].trim_end());
        out.push_str("\n\nPatch:\n");
        out.push_str(&text[pos + 1..]);
        out
    } else {
        text.to_owned()
    }
}

fn with_untracked_diffs(mut diff: String, pathspec: &str) -> Result<String> {
    push_added_file_diffs(&mut diff, &untracked_paths(pathspec)?, untracked_file_diff)?;
    Ok(diff)
}

/// What the pane will take of new files before the rest is left out. Past
/// this much text there is nothing more to see, and every file read is a git
/// process of its own.
const MAX_NEW_DIFF_BYTES: usize = 256 * 1024;

/// Append the diff adding each of `paths` would be, until they come to
/// `MAX_NEW_DIFF_BYTES`, with a line saying how many files were left out.
pub(super) fn push_added_file_diffs(
    text: &mut String,
    paths: &[String],
    added_file_diff: impl Fn(&str) -> Result<String>,
) -> Result<()> {
    let start = text.len();
    let mut shown = 0usize;
    for path in paths {
        if text.len() - start >= MAX_NEW_DIFF_BYTES {
            break;
        }
        let diff = added_file_diff(path)?;
        shown += 1;
        if diff.trim().is_empty() {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&diff);
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    if shown < paths.len() {
        text.push_str(&format!(
            "\n... {} more file(s) not shown\n",
            paths.len() - shown
        ));
    }
    Ok(())
}

pub(super) fn untracked_paths(pathspec: &str) -> Result<Vec<String>> {
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

pub(super) fn untracked_file_diff(path: &str) -> Result<String> {
    no_index_diff(git_command(&added_file_diff_args(path)), path)
}

/// A file with nothing to compare against, shown as the diff that adding it
/// would be. `git diff --no-index` asks nothing of a repository, which is what
/// lets a plain directory be shown this way too — so the directory is named
/// outright rather than taken from whichever checkout is selected.
pub(super) fn added_file_diff(dir: &Path, path: &str) -> Result<String> {
    no_index_diff(git_command_in_dir(dir, &added_file_diff_args(path)), path)
}

fn added_file_diff_args(path: &str) -> Vec<&str> {
    [&PLAIN_DIFF[..], &["--no-index", "--", "/dev/null", path]].concat()
}

/// A `--no-index` diff reports a difference with exit code 1, which for this
/// is the expected answer rather than a failure.
fn no_index_diff(mut command: Command, path: &str) -> Result<String> {
    let out = command.output()?;
    if out.status.success() || out.status.code() == Some(1) {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(anyhow::anyhow!(
            "git diff --no-index failed for {path}: {}",
            stderr.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_commit_patch_inserts_patch_heading() {
        let text = "commit abc\n\nFiles changed:\n a.rs | 1 +\n\ndiff --git a/a.rs b/a.rs\n";
        assert_eq!(
            label_commit_patch(text),
            "commit abc\n\nFiles changed:\n a.rs | 1 +\n\nPatch:\ndiff --git a/a.rs b/a.rs\n"
        );
    }
}
