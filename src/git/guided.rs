//! A diff cut into the steps a guided review walks: one hunk at a time, every
//! line numbered on the side it belongs to, so a note can be pinned to a line
//! that GitHub will accept a comment on.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// What one line of a hunk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file` and the like: shown, never commented on.
    Meta,
}

/// One line of a hunk, with its number in the old file, the new one, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkLine {
    pub kind: LineKind,
    /// The line as the diff has it, sign included.
    pub text: String,
    pub old: Option<usize>,
    pub new: Option<usize>,
}

impl HunkLine {
    /// Where a note on this line is pinned: the new file's line for anything
    /// that is still there, the old file's for a line the change removed.
    pub fn anchor(&self) -> Option<(usize, Side)> {
        match self.kind {
            LineKind::Removed => self.old.map(|line| (line, Side::Left)),
            LineKind::Added | LineKind::Context => self.new.map(|line| (line, Side::Right)),
            LineKind::Meta => None,
        }
    }
}

/// Which file of the diff a line number counts in, in GitHub's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Side {
    /// The file before the change.
    Left,
    /// The file after it.
    #[default]
    Right,
}

impl Side {
    pub fn github(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }
}

/// One step of the walk: a single hunk of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidedHunk {
    pub path: String,
    /// The `@@ … @@` line, function context included.
    pub header: String,
    pub lines: Vec<HunkLine>,
    /// Which file of the diff this is, counting from zero.
    pub file_index: usize,
    /// Which hunk of its file this is, counting from zero.
    pub hunk_index: usize,
    /// How many hunks its file has.
    pub hunks_in_file: usize,
    /// The file is new, deleted or binary: said once, on its first hunk.
    pub file_note: Option<String>,
}

impl GuidedHunk {
    pub fn added(&self) -> usize {
        self.count(LineKind::Added)
    }

    pub fn removed(&self) -> usize {
        self.count(LineKind::Removed)
    }

    fn count(&self, kind: LineKind) -> usize {
        self.lines.iter().filter(|line| line.kind == kind).count()
    }

    /// The first line the change itself touches, which is where the cursor
    /// starts and where an editor opens.
    pub fn first_changed(&self) -> usize {
        self.lines
            .iter()
            .position(|line| matches!(line.kind, LineKind::Added | LineKind::Removed))
            .unwrap_or(0)
    }

    /// The new-file line an editor should open on for line `index` of the hunk:
    /// the line itself when it survives, else the nearest surviving one before
    /// it, else the hunk's start.
    pub fn editor_line(&self, index: usize) -> usize {
        self.lines[..=index.min(self.lines.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|line| line.new)
            .or_else(|| self.lines.iter().find_map(|line| line.new))
            .unwrap_or(1)
    }

    /// The hunk as diff text, header first, for a prompt or a clipboard.
    pub fn patch(&self) -> String {
        let mut out = String::with_capacity(self.header.len() + 1);
        out.push_str(&self.header);
        out.push('\n');
        for line in &self.lines {
            out.push_str(&line.text);
            out.push('\n');
        }
        out
    }

    /// What identifies this hunk across rebuilds: the file and the text of
    /// what changed, not its position, so editing one hunk does not make the
    /// others look new.
    pub fn key(&self) -> String {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for line in self
            .lines
            .iter()
            .filter(|line| matches!(line.kind, LineKind::Added | LineKind::Removed))
        {
            for byte in line.text.bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            hash ^= 0x0a;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{}#{hash:016x}", self.path)
    }
}

/// Cut a unified diff into hunks, numbering every line.
///
/// A file that changed with no hunk to show — a binary, a pure rename, a mode
/// change — still becomes one step, so the walk passes over every file the
/// diff names.
pub fn parse_hunks(diff: &str) -> Vec<GuidedHunk> {
    let files = super::patch::parse_patch(diff).into_iter().filter(|file| {
        file.header
            .first()
            .is_some_and(|line| line.starts_with("diff --git "))
    });
    let mut steps = Vec::new();
    for (file_index, file) in files.enumerate() {
        let note = file_note(&file);
        let path = file.path().to_string();
        let mut hunks: Vec<(String, Vec<HunkLine>)> = file
            .hunks
            .into_iter()
            .map(|hunk| (hunk.header, hunk.lines.iter().map(hunk_line).collect()))
            .collect();
        if hunks.is_empty() {
            hunks.push((String::new(), Vec::new()));
        }
        let count = hunks.len();
        for (hunk_index, (header, lines)) in hunks.into_iter().enumerate() {
            steps.push(GuidedHunk {
                path: path.clone(),
                header,
                lines,
                file_index,
                hunk_index,
                hunks_in_file: count,
                file_note: if hunk_index == 0 { note.clone() } else { None },
            });
        }
    }
    steps
}

/// What a file's header says about it that its hunks cannot: the last thing
/// git reports, so a binary file reads as binary however else it changed.
fn file_note(file: &super::patch::FilePatch) -> Option<String> {
    if file.is_binary {
        Some("binary file".to_string())
    } else if file.is_rename {
        Some(format!("renamed from {}", file.old_path))
    } else if file.is_deleted {
        Some("deleted file".to_string())
    } else if file.is_new {
        Some("new file".to_string())
    } else {
        None
    }
}

fn hunk_line(line: &super::patch::PatchLine) -> HunkLine {
    use super::patch::LineKind as Patch;
    let kind = match line.kind {
        Patch::Context => LineKind::Context,
        Patch::Added => LineKind::Added,
        Patch::Removed => LineKind::Removed,
        Patch::NoNewline => LineKind::Meta,
    };
    let text = line.to_patch_line();
    HunkLine {
        kind,
        text: text.strip_suffix('\r').map(str::to_string).unwrap_or(text),
        old: line.old_no,
        new: line.new_no,
    }
}

/// A note written during a guided review, pinned to one line of one hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidedNote {
    pub path: String,
    pub line: usize,
    #[serde(default)]
    pub side: Side,
    pub body: String,
    /// The hunk the note was written on, by [`GuidedHunk::key`].
    pub step: String,
    /// The line it is pinned to, as the diff showed it, so a note whose code
    /// has since moved still says what it was about.
    #[serde(default)]
    pub quote: String,
}

/// What a guided review remembers between visits: which steps were looked at,
/// and the notes written along the way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidedProgress {
    #[serde(default)]
    pub reviewed: Vec<String>,
    #[serde(default)]
    pub notes: Vec<GuidedNote>,
}

/// Where the progress of the review named `key` is kept: inside the git
/// directory, so it is never committed and every worktree of the repository
/// sees the same notes on the same pull request.
pub fn progress_path(key: &str) -> Result<PathBuf> {
    let dir = super::common_git_dir().context("locate the git directory")?;
    let slug: String = key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    Ok(dir.join("lg").join("guided").join(format!("{slug}.json")))
}

pub fn load_progress(key: &str) -> GuidedProgress {
    progress_path(key)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_progress(key: &str, progress: &GuidedProgress) -> Result<()> {
    let path = progress_path(key)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(progress)?)
        .with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,4 +10,5 @@ fn a() {
 keep
-old line
+new line
+another
 tail
@@ -40,2 +41,2 @@ fn b() {
-x
+y
 z
diff --git a/img.png b/img.png
new file mode 100644
Binary files /dev/null and b/img.png differ
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,2 @@
+fn main() {}
+// end
";

    #[test]
    fn every_hunk_and_every_file_becomes_a_step() {
        let steps = parse_hunks(DIFF);

        let paths: Vec<&str> = steps.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "src/a.rs", "img.png", "src/new.rs"]);
        assert_eq!(steps[1].hunk_index, 1);
        assert_eq!(steps[1].hunks_in_file, 2);
        assert_eq!(steps[2].file_note.as_deref(), Some("binary file"));
        assert_eq!(steps[3].file_note.as_deref(), Some("new file"));
    }

    #[test]
    fn lines_are_numbered_on_the_side_they_belong_to() {
        let steps = parse_hunks(DIFF);
        let hunk = &steps[0];
        let find = |text: &str| hunk.lines.iter().find(|l| l.text == text).unwrap();

        assert_eq!(find(" keep").anchor(), Some((10, Side::Right)));
        assert_eq!(find("-old line").anchor(), Some((11, Side::Left)));
        assert_eq!(find("+new line").anchor(), Some((11, Side::Right)));
        assert_eq!(find("+another").anchor(), Some((12, Side::Right)));
        assert_eq!(find(" tail").anchor(), Some((13, Side::Right)));
        assert_eq!(hunk.added(), 2);
        assert_eq!(hunk.removed(), 1);
    }

    #[test]
    fn an_editor_opens_on_a_line_that_still_exists() {
        let steps = parse_hunks(DIFF);
        let hunk = &steps[0];
        let removed = hunk
            .lines
            .iter()
            .position(|l| l.text == "-old line")
            .unwrap();

        assert_eq!(hunk.editor_line(removed), 10);
        assert_eq!(hunk.editor_line(hunk.first_changed()), 10);
    }

    #[test]
    fn a_hunk_keeps_its_key_when_another_hunk_changes() {
        let before = parse_hunks(DIFF);
        let edited = DIFF.replace("+y", "+y2");
        let after = parse_hunks(&edited);

        assert_eq!(before[0].key(), after[0].key());
        assert_ne!(before[1].key(), after[1].key());
    }

    #[test]
    fn progress_round_trips_through_json() {
        let progress = GuidedProgress {
            reviewed: vec!["src/a.rs#1".into()],
            notes: vec![GuidedNote {
                path: "src/a.rs".into(),
                line: 11,
                side: Side::Left,
                body: "why remove this?".into(),
                step: "src/a.rs#1".into(),
                quote: "-old line".into(),
            }],
        };
        let text = serde_json::to_string(&progress).unwrap();

        assert_eq!(
            serde_json::from_str::<GuidedProgress>(&text).unwrap(),
            progress
        );
    }
}
