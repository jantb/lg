//! A diff cut into the steps a guided review walks: one file at a time, every
//! line numbered on the side it belongs to, so a note can be pinned to a line
//! that GitHub will accept a comment on.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// What one line of a file's diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file` and the like: shown, never commented on.
    Meta,
    /// The `@@ … @@` line a hunk opens on, function context included.
    Header,
}

/// One line of a file's diff, with its number in the old file, the new one, or both.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
            LineKind::Meta | LineKind::Header => None,
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

/// One step of the walk: every hunk of one file, each opened by its header line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuidedFile {
    pub path: String,
    pub lines: Vec<HunkLine>,
    /// Which file of the diff this is, counting from zero.
    pub file_index: usize,
    /// The file is new, deleted, renamed or binary.
    pub file_note: Option<String>,
    key_memo: KeyMemo,
}

/// The last [`GuidedFile::key`] worked out, with exactly what it was worked
/// out from. The fields it reads are public and are changed after parsing, so
/// a key is only reused when what it hashed is still there byte for byte:
/// comparing is far cheaper than hashing again.
#[derive(Default)]
struct KeyMemo(std::sync::Mutex<Option<KeyMemoEntry>>);

struct KeyMemoEntry {
    path: String,
    /// Every changed line's text followed by a newline: what the hash reads.
    hashed: Vec<u8>,
    key: String,
}

impl KeyMemo {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<KeyMemoEntry>> {
        self.0.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl Clone for KeyMemo {
    fn clone(&self) -> Self {
        let copy = self.lock().as_ref().map(|entry| KeyMemoEntry {
            path: entry.path.clone(),
            hashed: entry.hashed.clone(),
            key: entry.key.clone(),
        });
        Self(std::sync::Mutex::new(copy))
    }
}

/// A memo is not part of what a step is.
impl PartialEq for KeyMemo {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for KeyMemo {}

impl std::hash::Hash for KeyMemo {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl std::fmt::Debug for KeyMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyMemo")
    }
}

impl GuidedFile {
    pub fn added(&self) -> usize {
        self.count(LineKind::Added)
    }

    pub fn removed(&self) -> usize {
        self.count(LineKind::Removed)
    }

    pub fn hunks(&self) -> usize {
        self.count(LineKind::Header)
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

    /// The new-file line an editor should open on for line `index`: the line
    /// itself when it survives, else the nearest surviving one before it, else
    /// the first hunk's start.
    pub fn editor_line(&self, index: usize) -> usize {
        self.lines[..=index.min(self.lines.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|line| line.new)
            .or_else(|| self.lines.iter().find_map(|line| line.new))
            .unwrap_or(1)
    }

    /// What identifies this file's change across rebuilds: the file and the
    /// text of what changed, not its position, so editing one file does not
    /// make the others look new.
    pub fn key(&self) -> String {
        let mut memo = self.key_memo.lock();
        if let Some(entry) = memo.as_ref()
            && entry.path == self.path
            && self.changed_bytes_are(&entry.hashed)
        {
            return entry.key.clone();
        }
        let mut hashed = Vec::new();
        for line in self.changed_lines() {
            hashed.extend_from_slice(line.text.as_bytes());
            hashed.push(b'\n');
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for &byte in &hashed {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        let key = format!("{}#{hash:016x}", self.path);
        *memo = Some(KeyMemoEntry {
            path: self.path.clone(),
            hashed,
            key: key.clone(),
        });
        key
    }

    fn changed_lines(&self) -> impl Iterator<Item = &HunkLine> {
        self.lines
            .iter()
            .filter(|line| matches!(line.kind, LineKind::Added | LineKind::Removed))
    }

    /// Whether the changed lines, each followed by a newline, are `bytes`.
    fn changed_bytes_are(&self, bytes: &[u8]) -> bool {
        let mut rest = bytes;
        for line in self.changed_lines() {
            let text = line.text.as_bytes();
            match rest.strip_prefix(text).and_then(|r| r.strip_prefix(b"\n")) {
                Some(after) => rest = after,
                None => return false,
            }
        }
        rest.is_empty()
    }
}

/// Cut a unified diff into one step per file, in path order, numbering every line.
///
/// A file that changed with no hunk to show — a binary, a pure rename, a mode
/// change — still becomes a step, so the walk passes over every file the diff
/// names.
pub fn parse_files(diff: &str) -> Vec<GuidedFile> {
    let files = super::patch::parse_patch(diff).into_iter().filter(|file| {
        file.header
            .first()
            .is_some_and(|line| line.starts_with("diff --git "))
    });
    let mut steps: Vec<GuidedFile> = files
        .enumerate()
        .map(|(file_index, file)| {
            let file_note = file_note(&file);
            let path = file.path().to_string();
            let lines = file
                .hunks
                .into_iter()
                .flat_map(|hunk| {
                    let header = HunkLine {
                        kind: LineKind::Header,
                        text: hunk.header,
                        old: None,
                        new: None,
                    };
                    std::iter::once(header)
                        .chain(hunk.lines.iter().map(hunk_line).collect::<Vec<_>>())
                })
                .collect();
            GuidedFile {
                path,
                lines,
                file_index,
                file_note,
                key_memo: KeyMemo::default(),
            }
        })
        .collect();
    // Sorted by path, every directory's files are next to each other, so the
    // walk goes through the tree the step list shows one directory at a time.
    steps.sort_by(|a, b| a.path.cmp(&b.path));
    for (file_index, step) in steps.iter_mut().enumerate() {
        step.file_index = file_index;
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

/// A note written during a guided review, pinned to one line of one file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GuidedNote {
    pub path: String,
    pub line: usize,
    #[serde(default)]
    pub side: Side,
    pub body: String,
    /// The step the note was written on, by [`GuidedFile::key`].
    pub step: String,
    /// The line it is pinned to, as the diff showed it, so a note whose code
    /// has since moved still says what it was about.
    #[serde(default)]
    pub quote: String,
}

/// A finding of the model's the reviewer ticked off as dealt with. The file
/// and what it said tell it from any other; the line it cited is kept for
/// showing, since a finding still standing after a fix keeps its words but
/// not always its line.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TickedFinding {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// One file's change as a read of it saw it: enough to tell later which
/// lines an edit changed since, and where a finding's line has moved to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReadStep {
    pub path: String,
    /// The step that was read, by [`GuidedFile::key`].
    pub key: String,
    /// Every line the change added or removed, sign included.
    #[serde(default)]
    pub changed: Vec<String>,
    /// The diff line each line number a finding cited was, sign included.
    #[serde(default)]
    pub quotes: Vec<(usize, String)>,
}

/// The model's finished read of one file, in the shape the walk shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileRead {
    pub step: ReadStep,
    pub text: String,
}

/// The model's finished read of the whole change, with every file as it saw it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WholeRead {
    /// The final step it read, by its key.
    pub key: String,
    pub text: String,
    #[serde(default)]
    pub files: Vec<ReadStep>,
}

/// The model's finished reads, kept so a review opened again shows what it
/// found without asking again, and asks only about what changed since.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GuidedReads {
    /// The latest read of each file.
    #[serde(default)]
    pub files: Vec<FileRead>,
    #[serde(default)]
    pub whole: Option<WholeRead>,
    /// The plan the walk is ordered by.
    #[serde(default)]
    pub plan: Option<String>,
}

/// How a guided review orders its files, all from the same plan the model
/// reads at the start: switching between them loses nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WalkMode {
    /// Grouped by what each part of the change is for. A review saved while
    /// the map and triage walks were still there comes back to this.
    #[default]
    #[serde(alias = "Map", alias = "Triage")]
    Concerns,
    /// Every open finding of what has been read, worst first.
    Issues,
}

impl WalkMode {
    pub fn next(self) -> Self {
        match self {
            Self::Concerns => Self::Issues,
            Self::Issues => Self::Concerns,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Concerns => "CONCERNS",
            Self::Issues => "ISSUES",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Concerns => "grouped by concern, each file read with the rest of its concern",
            Self::Issues => "the open findings of every file read, worst first",
        }
    }
}

/// What a guided review remembers between visits: which steps were looked at,
/// the notes written along the way, the findings ticked off, how the walk
/// is ordered, and what the model read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GuidedProgress {
    #[serde(default)]
    pub reviewed: Vec<String>,
    #[serde(default)]
    pub notes: Vec<GuidedNote>,
    #[serde(default)]
    pub ticked: Vec<TickedFinding>,
    #[serde(default)]
    pub walk: WalkMode,
    #[serde(default)]
    pub reads: GuidedReads,
}

/// Where the progress of the review named `key` is kept: inside the git
/// directory, so it is never committed and every worktree of the repository
/// sees the same notes on the same pull request.
pub fn progress_path(key: &str) -> Result<PathBuf> {
    let dir = super::common_git_dir().context("locate the git directory")?;
    Ok(progress_path_in(&dir, key))
}

/// [`progress_path`] in the common git directory `git_dir`.
pub fn progress_path_in(git_dir: &Path, key: &str) -> PathBuf {
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
    git_dir
        .join("lg")
        .join("guided")
        .join(format!("{slug}.json"))
}

pub fn load_progress(key: &str) -> GuidedProgress {
    progress_path(key)
        .map(|path| load_progress_at(&path))
        .unwrap_or_default()
}

pub fn load_progress_at(path: &Path) -> GuidedProgress {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_progress(path: &Path, progress: &GuidedProgress) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(progress)?)
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
    fn every_file_becomes_one_step_holding_all_its_hunks_in_path_order() {
        let steps = parse_files(DIFF);

        let paths: Vec<&str> = steps.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["img.png", "src/a.rs", "src/new.rs"]);
        let a = file(&steps, "src/a.rs");
        assert_eq!(a.hunks(), 2);
        assert!(a.lines.iter().any(|l| l.text == "+y"));
        assert_eq!(
            file(&steps, "img.png").file_note.as_deref(),
            Some("binary file")
        );
        assert_eq!(
            file(&steps, "src/new.rs").file_note.as_deref(),
            Some("new file")
        );
    }

    fn file<'a>(steps: &'a [GuidedFile], path: &str) -> &'a GuidedFile {
        steps.iter().find(|step| step.path == path).unwrap()
    }

    #[test]
    fn lines_are_numbered_on_the_side_they_belong_to() {
        let steps = parse_files(DIFF);
        let file = file(&steps, "src/a.rs");
        let find = |text: &str| file.lines.iter().find(|l| l.text == text).unwrap();

        assert_eq!(find(" keep").anchor(), Some((10, Side::Right)));
        assert_eq!(find("-old line").anchor(), Some((11, Side::Left)));
        assert_eq!(find("+new line").anchor(), Some((11, Side::Right)));
        assert_eq!(find("+another").anchor(), Some((12, Side::Right)));
        assert_eq!(find(" tail").anchor(), Some((13, Side::Right)));
        // Both hunks of the file: +new line, +another, +y; -old line, -x.
        assert_eq!(file.added(), 3);
        assert_eq!(file.removed(), 2);
    }

    #[test]
    fn an_editor_opens_on_a_line_that_still_exists() {
        let steps = parse_files(DIFF);
        let hunk = file(&steps, "src/a.rs");
        let removed = hunk
            .lines
            .iter()
            .position(|l| l.text == "-old line")
            .unwrap();

        assert_eq!(hunk.editor_line(removed), 10);
        assert_eq!(hunk.editor_line(hunk.first_changed()), 10);
    }

    #[test]
    fn a_file_keeps_its_key_when_another_file_changes() {
        let before = parse_files(DIFF);
        let edited = DIFF.replace("+// end", "+// the end");
        let after = parse_files(&edited);

        assert_eq!(
            file(&before, "src/a.rs").key(),
            file(&after, "src/a.rs").key()
        );
        assert_ne!(
            file(&before, "src/new.rs").key(),
            file(&after, "src/new.rs").key()
        );
    }

    /// Keys are what saved progress is filed under, so one worked out today
    /// must be the one a walk saved before.
    #[test]
    fn a_file_has_the_key_its_saved_progress_is_filed_under() {
        assert_eq!(
            file(&parse_files(DIFF), "src/new.rs").key(),
            "src/new.rs#948ff01b113cb086"
        );
    }

    #[test]
    fn a_key_follows_a_line_changed_after_it_was_asked_for() {
        let mut step = file(&parse_files(DIFF), "src/new.rs").clone();
        let before = step.key();
        let end = step
            .lines
            .iter()
            .position(|line| line.text == "+// end")
            .unwrap();

        step.lines[end].text = "+// the end".into();

        let edited = parse_files(&DIFF.replace("+// end", "+// the end"));
        assert_ne!(step.key(), before);
        assert_eq!(step.key(), file(&edited, "src/new.rs").key());
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
            ticked: vec![TickedFinding {
                path: "src/a.rs".into(),
                line: 12,
                text: "-1 is used as an index".into(),
            }],
            walk: WalkMode::Issues,
            reads: GuidedReads {
                files: vec![FileRead {
                    step: ReadStep {
                        path: "src/a.rs".into(),
                        key: "src/a.rs#1".into(),
                        changed: vec!["-old line".into(), "+new line".into()],
                        quotes: vec![(11, "+new line".into())],
                    },
                    text: "Summary: Renames.\nL11 issue: -1 is used as an index\nVerdict: issue"
                        .into(),
                }],
                whole: Some(WholeRead {
                    key: "final#1".into(),
                    text: "Summary: Hangs together.\nVerdict: ok".into(),
                    files: Vec::new(),
                }),
                plan: Some("Summary: A rename.".into()),
            },
        };
        let text = serde_json::to_string(&progress).unwrap();

        assert_eq!(
            serde_json::from_str::<GuidedProgress>(&text).unwrap(),
            progress
        );
    }

    /// A review saved before reads were kept still opens with its ticks and notes.
    #[test]
    fn progress_saved_before_reads_were_kept_still_loads() {
        let text = r#"{"reviewed":["src/a.rs#1"],"notes":[],"ticked":[{"path":"src/a.rs","line":12,"text":"-1 is used as an index"}],"walk":"Concerns"}"#;

        let progress: GuidedProgress = serde_json::from_str(text).unwrap();

        assert_eq!(progress.ticked[0].text, "-1 is used as an index");
        assert_eq!(progress.reads, GuidedReads::default());
    }

    #[test]
    fn a_review_saved_in_a_walk_mode_since_removed_walks_by_concern() {
        for old in ["Map", "Triage"] {
            let text = format!("{{\"walk\":\"{old}\"}}");
            let progress: GuidedProgress = serde_json::from_str(&text).unwrap();
            assert_eq!(progress.walk, WalkMode::Concerns);
        }
    }
}
