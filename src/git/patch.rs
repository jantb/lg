//! Reading unified diffs as git writes them: the files a patch touches, the
//! hunks of each, and every line numbered on the side it belongs to.
//!
//! A hunk is read by the lengths its `@@` header gives, so a removed line that
//! happens to read `--- x` is still a removed line. Any one hunk can be written
//! back out with its file's header as a patch `git apply` accepts on its own.

use std::borrow::Cow;

use paths::{header_path, marker_path, quote_path};

mod paths;

pub use paths::diff_git_paths;

/// What one line of a hunk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`, about the line before it.
    NoNewline,
}

impl LineKind {
    /// The character the line starts with in a patch.
    pub fn marker(self) -> char {
        match self {
            Self::Context => ' ',
            Self::Added => '+',
            Self::Removed => '-',
            Self::NoNewline => '\\',
        }
    }
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchLine {
    pub kind: LineKind,
    /// The line's number in the old file, for context and removed lines.
    pub old_no: Option<usize>,
    /// The line's number in the new file, for context and added lines.
    pub new_no: Option<usize>,
    /// The line without its marker. A trailing `\r` of a CRLF file is kept.
    pub text: String,
}

impl PatchLine {
    /// The line as a patch has it, marker included.
    pub fn to_patch_line(&self) -> String {
        let mut line = String::with_capacity(self.text.len() + 1);
        line.push(self.kind.marker());
        line.push_str(&self.text);
        line
    }
}

/// One `@@ -a,b +c,d @@` hunk and its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    /// The `@@` line itself, function context included.
    pub header: String,
    pub lines: Vec<PatchLine>,
}

impl Hunk {
    /// Whether the hunk has every line its header counts.
    pub fn is_complete(&self) -> bool {
        let old = self
            .lines
            .iter()
            .filter(|line| matches!(line.kind, LineKind::Context | LineKind::Removed))
            .count();
        let new = self
            .lines
            .iter()
            .filter(|line| matches!(line.kind, LineKind::Context | LineKind::Added))
            .count();
        old == self.old_len && new == self.new_len
    }

    /// The hunk as patch text: its header and every line, each ending in a
    /// newline.
    pub fn to_patch(&self) -> String {
        let mut out = String::new();
        push_line(&mut out, &self.header);
        for line in &self.lines {
            out.push(line.kind.marker());
            push_line(&mut out, &line.text);
        }
        out
    }
}

/// Everything a patch says about one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilePatch {
    /// The path before the change, without its `a/` prefix. For a new file it
    /// is the same as `new_path`, as git's own header has it.
    pub old_path: String,
    /// The path after the change, without its `b/` prefix. For a deleted file
    /// it is the same as `old_path`.
    pub new_path: String,
    /// The lines before the first hunk, from `diff --git` to `+++`, as given.
    pub header: Vec<String>,
    pub is_new: bool,
    pub is_deleted: bool,
    /// Renamed: `old_path` names where it came from.
    pub is_rename: bool,
    /// Copied: `old_path` names what it was copied from.
    pub is_copy: bool,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
}

impl FilePatch {
    /// The file this patch is about: where it is after the change, or where it
    /// was when the change deleted it.
    pub fn path(&self) -> &str {
        if self.new_path.is_empty() {
            &self.old_path
        } else {
            &self.new_path
        }
    }

    /// Hunk `index` of this file alone, with the file's header, as a patch
    /// that applies by itself. A header that lacks the `---`/`+++` lines `git
    /// apply` needs gets them made up from the paths.
    pub fn hunk_patch(&self, index: usize) -> Option<String> {
        let hunk = self.hunks.get(index)?;
        let mut out = String::new();
        if self.header.is_empty() {
            push_line(
                &mut out,
                &format!(
                    "diff --git {} {}",
                    quote_path("a/", &self.old_path),
                    quote_path("b/", self.path())
                ),
            );
        }
        for line in &self.header {
            push_line(&mut out, line);
        }
        if !self.header.iter().any(|line| line.starts_with("--- ")) {
            let old = if self.is_new {
                Cow::Borrowed("/dev/null")
            } else {
                Cow::Owned(quote_path("a/", &self.old_path))
            };
            let new = if self.is_deleted {
                Cow::Borrowed("/dev/null")
            } else {
                Cow::Owned(quote_path("b/", self.path()))
            };
            push_line(&mut out, &format!("--- {old}"));
            push_line(&mut out, &format!("+++ {new}"));
        }
        out.push_str(&hunk.to_patch());
        Some(out)
    }
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

/// The ranges of an `@@ -a,b +c,d @@` line, as a hunk with no lines yet. A
/// length left out is one, as in `@@ -5 +5 @@`.
pub fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ ")?;
    let mut parts = rest.split_whitespace();
    let (old_start, old_len) = parse_range(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_len) = parse_range(parts.next()?.strip_prefix('+')?)?;
    Some(Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
        header: line.to_string(),
        lines: Vec::new(),
    })
}

fn parse_range(range: &str) -> Option<(usize, usize)> {
    match range.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

/// Every file of a unified diff, in order. Lines outside any file — the
/// commit header `git show` starts with — are passed over. Hunks with no file
/// header of their own land in a file with empty paths.
pub fn parse_patch(text: &str) -> Vec<FilePatch> {
    let mut files: Vec<FilePatch> = Vec::new();
    // Lines still owed to the open hunk, on the old side and the new.
    let mut owed = (0usize, 0usize);
    let mut next_no = (0usize, 0usize);

    // `split` rather than `lines`, which would take the `\r` off a CRLF file's
    // lines and leave a hunk that no longer applies to it.
    let text = text.strip_suffix('\n').unwrap_or(text);
    for line in text.split('\n') {
        if owed != (0, 0)
            && let Some(file) = files.last_mut()
            && let Some(hunk) = file.hunks.last_mut()
            && let Some(kind) = hunk_line_kind(line, owed)
        {
            let (old_no, new_no) = match kind {
                LineKind::Context => (Some(next_no.0), Some(next_no.1)),
                LineKind::Removed => (Some(next_no.0), None),
                LineKind::Added => (None, Some(next_no.1)),
                LineKind::NoNewline => (None, None),
            };
            if old_no.is_some() {
                next_no.0 += 1;
                owed.0 -= 1;
            }
            if new_no.is_some() {
                next_no.1 += 1;
                owed.1 -= 1;
            }
            hunk.lines.push(PatchLine {
                kind,
                old_no,
                new_no,
                text: line.get(1..).unwrap_or_default().to_string(),
            });
            continue;
        }
        owed = (0, 0);

        if line.starts_with('\\')
            && let Some(hunk) = files.last_mut().and_then(|file| file.hunks.last_mut())
            && hunk
                .lines
                .last()
                .is_some_and(|last| last.kind != LineKind::NoNewline)
        {
            hunk.lines.push(PatchLine {
                kind: LineKind::NoNewline,
                old_no: None,
                new_no: None,
                text: line[1..].to_string(),
            });
            continue;
        }

        if line.starts_with("diff --git ") {
            let (old_path, new_path) = diff_git_paths(line).unwrap_or_default();
            files.push(FilePatch {
                old_path,
                new_path,
                header: vec![line.to_string()],
                ..FilePatch::default()
            });
            continue;
        }

        if let Some(hunk) = parse_hunk_header(line) {
            if files.is_empty() {
                files.push(FilePatch::default());
            }
            let file = files.last_mut().expect("a file was just ensured");
            owed = (hunk.old_len, hunk.new_len);
            next_no = (hunk.old_start, hunk.new_start);
            file.hunks.push(hunk);
            continue;
        }

        // A plain `diff -u` has no `diff --git` line: its `---` opens a file.
        if line.starts_with("--- ")
            && files
                .last()
                .is_none_or(|file| !file.hunks.is_empty() || file.header.is_empty())
        {
            files.push(FilePatch::default());
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if !file.hunks.is_empty() {
            continue;
        }
        read_extended_header(file, line);
        file.header.push(line.to_string());
    }
    files
}

/// What `line` is as the next line of a hunk still owed `owed` lines, or
/// `None` when it cannot be part of it.
fn hunk_line_kind(line: &str, owed: (usize, usize)) -> Option<LineKind> {
    match line.as_bytes().first() {
        Some(b' ') | None if owed.0 > 0 && owed.1 > 0 => Some(LineKind::Context),
        Some(b'-') if owed.0 > 0 => Some(LineKind::Removed),
        Some(b'+') if owed.1 > 0 => Some(LineKind::Added),
        Some(b'\\') => Some(LineKind::NoNewline),
        _ => None,
    }
}

fn read_extended_header(file: &mut FilePatch, line: &str) {
    if let Some(rest) = line.strip_prefix("--- ") {
        match marker_path(rest, "a/") {
            Some(path) => {
                file.old_path = path;
                if file.new_path.is_empty() {
                    file.new_path = file.old_path.clone();
                }
            }
            None => file.is_new = true,
        }
    } else if let Some(rest) = line.strip_prefix("+++ ") {
        match marker_path(rest, "b/") {
            Some(path) => {
                file.new_path = path;
                if file.old_path.is_empty() {
                    file.old_path = file.new_path.clone();
                }
            }
            None => file.is_deleted = true,
        }
    } else if line.starts_with("new file mode") {
        file.is_new = true;
    } else if line.starts_with("deleted file mode") {
        file.is_deleted = true;
    } else if let Some(from) = line.strip_prefix("rename from ") {
        file.old_path = header_path(from);
        file.is_rename = true;
    } else if let Some(to) = line.strip_prefix("rename to ") {
        file.new_path = header_path(to);
        file.is_rename = true;
    } else if let Some(from) = line.strip_prefix("copy from ") {
        file.old_path = header_path(from);
        file.is_copy = true;
    } else if let Some(to) = line.strip_prefix("copy to ") {
        file.new_path = header_path(to);
        file.is_copy = true;
    } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
        file.is_binary = true;
    }
}

#[cfg(test)]
mod tests;
