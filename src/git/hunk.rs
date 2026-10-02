//! The hunks the diff pane shows, and staging, unstaging or discarding one of
//! them on its own.
//!
//! The pane's text for a file, a folder or every change is two diffs one after
//! the other: what is staged, under `== staged (--cached) ==`, and what is in
//! the working tree, under `== worktree ==`. A hunk's section says what can be
//! done with it: a staged hunk can be unstaged, a worktree hunk staged or
//! thrown away. A commit's hunks are history and can be neither.
//!
//! The text on screen is only what the hunk *was* when the pane was last read.
//! Applying one reads the file's diff again, finds the same hunk in it and
//! applies that, so a file edited since is refused with a reason rather than
//! patched with a hunk that no longer describes it.

use anyhow::Result;

use super::patch::{self, FilePatch, Hunk};
use super::run;

/// Which of the pane's diffs a hunk was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HunkSide {
    /// In the index: `git diff --cached`.
    Staged,
    /// In the working tree, not yet staged: `git diff`, and untracked files.
    #[default]
    Worktree,
    /// Part of a commit, which is history.
    Committed,
}

impl HunkSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Worktree => "unstaged",
            Self::Committed => "committed",
        }
    }
}

/// One hunk as the pane shows it, with enough of its file to find it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownHunk {
    pub side: HunkSide,
    /// Where the file was before the change; the same as `path` unless renamed.
    pub old_path: String,
    pub path: String,
    /// The hunk adds the whole file: an untracked file, or a new one staged.
    pub is_new: bool,
    pub hunk: Hunk,
}

/// What to do with one hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkOp {
    /// Add a worktree hunk to the index.
    Stage,
    /// Take a staged hunk back out of the index; the working tree keeps it.
    Unstage,
    /// Undo a worktree hunk in the file itself.
    Discard,
}

impl HunkOp {
    /// Whether this can be done to a hunk read from `side`.
    pub fn applies_to(self, side: HunkSide) -> bool {
        matches!(
            (self, side),
            (Self::Stage | Self::Discard, HunkSide::Worktree) | (Self::Unstage, HunkSide::Staged)
        )
    }
}

/// Which diff a line of the pane's text opens, when it is one of the section
/// headings lg writes between them.
fn section_heading(line: &str) -> Option<HunkSide> {
    let line = line.trim_end();
    if !line.ends_with("==") {
        return None;
    }
    if line.starts_with("== staged (--cached)") {
        Some(HunkSide::Staged)
    } else if line.starts_with("== worktree") {
        Some(HunkSide::Worktree)
    } else {
        None
    }
}

/// Every hunk in the pane's text, in the order it shows them.
///
/// `sections` is whether the text is lg's staged-then-worktree view; without
/// it every hunk is [`HunkSide::Committed`], as in a commit, whose message may
/// say anything at all.
pub fn shown_hunks(text: &str, sections: bool) -> Vec<ShownHunk> {
    if !sections {
        return hunks_of(text, HunkSide::Committed);
    }
    let mut out = Vec::new();
    let mut side = None;
    let mut segment = String::new();
    for line in text.split('\n') {
        if let Some(next) = section_heading(line) {
            if let Some(side) = side {
                out.extend(hunks_of(&segment, side));
            }
            segment.clear();
            side = Some(next);
            continue;
        }
        segment.push_str(line);
        segment.push('\n');
    }
    if let Some(side) = side {
        out.extend(hunks_of(&segment, side));
    }
    out
}

fn hunks_of(text: &str, side: HunkSide) -> Vec<ShownHunk> {
    patch::parse_patch(text)
        .into_iter()
        .filter(|file| !file.is_binary)
        .flat_map(|file| {
            let path = file.path().to_string();
            let old_path = if file.old_path.is_empty() {
                path.clone()
            } else {
                file.old_path.clone()
            };
            let is_new = file.is_new;
            file.hunks.into_iter().map(move |hunk| ShownHunk {
                side,
                old_path: old_path.clone(),
                path: path.clone(),
                is_new,
                hunk,
            })
        })
        .collect()
}

/// Do `op` to `shown`, against the file as it is now. Returns what to tell
/// the user.
pub fn apply_hunk(op: HunkOp, shown: &ShownHunk) -> Result<String> {
    if !op.applies_to(shown.side) {
        anyhow::bail!(
            "a {} hunk cannot be {}",
            shown.side.label(),
            match op {
                HunkOp::Stage => "staged",
                HunkOp::Unstage => "unstaged",
                HunkOp::Discard => "discarded",
            }
        );
    }
    let path = shown.path.as_str();
    if shown.side == HunkSide::Worktree && is_untracked(path)? {
        // An untracked file is one hunk that adds all of it, and git has no
        // index entry to apply that hunk to; staging it is adding the file.
        return match op {
            HunkOp::Stage => {
                run(&["add", "--", path])?;
                Ok(format!("staged new file {path}"))
            }
            _ => anyhow::bail!(
                "{path} is untracked: delete it from the Files pane (d) to throw it away"
            ),
        };
    }
    let current = current_file_patch(shown)?;
    let index = find_hunk(&current, &shown.hunk).ok_or_else(|| {
        anyhow::anyhow!(
            "{path} changed since the diff was read; the hunk is no longer there as shown \u{2014} look again and retry"
        )
    })?;
    let patch = current
        .hunk_patch(index)
        .ok_or_else(|| anyhow::anyhow!("hunk {index} of {path} has no patch"))?;
    let args: &[&str] = match op {
        HunkOp::Stage => &["apply", "--cached", "--whitespace=nowarn", "-"],
        HunkOp::Unstage => &["apply", "--cached", "--reverse", "--whitespace=nowarn", "-"],
        HunkOp::Discard => &["apply", "--reverse", "--whitespace=nowarn", "-"],
    };
    super::run_with_input(args, &patch).map_err(|err| {
        anyhow::anyhow!("the hunk in {path} no longer applies (the file changed since): {err}")
    })?;
    let at = shown.hunk.new_start.max(shown.hunk.old_start);
    Ok(match op {
        HunkOp::Stage => format!("staged hunk at {path}:{at}"),
        HunkOp::Unstage => format!("unstaged hunk at {path}:{at}"),
        HunkOp::Discard => format!("discarded hunk at {path}:{at}"),
    })
}

fn is_untracked(path: &str) -> Result<bool> {
    let out = run(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
        path,
    ])?;
    Ok(out
        .stdout
        .split(|byte| *byte == 0)
        .any(|entry| entry == path.as_bytes()))
}

/// The file's diff on the hunk's side as git reads it now, written with the
/// plain `a/` and `b/` prefixes `git apply` expects whatever the user's diff
/// settings are.
fn current_file_patch(shown: &ShownHunk) -> Result<FilePatch> {
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    if shown.side == HunkSide::Staged {
        args.push("--cached");
    }
    args.push("--");
    args.push(&shown.path);
    if shown.old_path != shown.path {
        args.push(&shown.old_path);
    }
    let out = run(&args)?;
    let text = String::from_utf8_lossy(&out.stdout);
    patch::parse_patch(&text)
        .into_iter()
        .find(|file| file.path() == shown.path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} has no {} changes any more",
                shown.path,
                shown.side.label()
            )
        })
}

/// The hunk of `file` that is `wanted`: the one with the same lines, at the
/// same place when several have them.
fn find_hunk(file: &FilePatch, wanted: &Hunk) -> Option<usize> {
    let same_lines = |hunk: &Hunk| {
        hunk.lines.len() == wanted.lines.len()
            && hunk
                .lines
                .iter()
                .zip(&wanted.lines)
                .all(|(a, b)| a.kind == b.kind && a.text == b.text)
    };
    let candidates: Vec<usize> = file
        .hunks
        .iter()
        .enumerate()
        .filter(|(_, hunk)| same_lines(hunk))
        .map(|(index, _)| index)
        .collect();
    candidates
        .iter()
        .copied()
        .find(|index| {
            let hunk = &file.hunks[*index];
            hunk.old_start == wanted.old_start && hunk.new_start == wanted.new_start
        })
        .or_else(|| (candidates.len() == 1).then(|| candidates[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMBINED: &str = r#"== staged (--cached) ==
diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
-one
+ONE
 two

== worktree ==
diff --git a/a.txt b/a.txt
index 2222222..3333333 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,3 @@
 ONE
 two
+three
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..4444444
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+fresh
"#;

    #[test]
    fn hunks_are_told_apart_by_the_section_they_sit_under() {
        let hunks = shown_hunks(COMBINED, true);
        let sides: Vec<_> = hunks.iter().map(|h| (h.side, h.path.as_str())).collect();
        assert_eq!(
            sides,
            [
                (HunkSide::Staged, "a.txt"),
                (HunkSide::Worktree, "a.txt"),
                (HunkSide::Worktree, "new.txt"),
            ]
        );
        assert!(hunks[2].is_new);
    }

    #[test]
    fn a_commit_has_no_sections_and_nothing_to_stage() {
        let hunks = shown_hunks(COMBINED, false);
        assert!(hunks.iter().all(|h| h.side == HunkSide::Committed));
        assert!(!HunkOp::Stage.applies_to(HunkSide::Committed));
        assert!(!HunkOp::Unstage.applies_to(HunkSide::Worktree));
        assert!(!HunkOp::Discard.applies_to(HunkSide::Staged));
    }
}
