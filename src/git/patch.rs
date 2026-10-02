//! Reading unified diffs as git writes them: the files a patch touches, the
//! hunks of each, and every line numbered on the side it belongs to.
//!
//! A hunk is read by the lengths its `@@` header gives, so a removed line that
//! happens to read `--- x` is still a removed line. Any one hunk can be written
//! back out with its file's header as a patch `git apply` accepts on its own.

use std::borrow::Cow;

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

/// The old and new paths a `diff --git` line names, without their `a/` and
/// `b/` prefixes.
///
/// Git quotes a path holding a quote, a backslash, a control character or
/// (by default) anything outside ASCII, and leaves one with spaces bare. A
/// bare pair is split where both halves name the same file, which is the case
/// for everything but a rename; the `rename from`/`rename to` lines settle
/// those, and [`parse_patch`] reads them.
pub fn diff_git_paths(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    let (old, new) = split_header_paths(rest)?;
    Some((strip_prefix_owned(old, "a/"), strip_prefix_owned(new, "b/")))
}

fn split_header_paths(rest: &str) -> Option<(String, String)> {
    if rest.starts_with('"') {
        let (old, len) = unquote(rest)?;
        let new = rest[len..].strip_prefix(' ')?;
        return Some((old, header_path(new)));
    }
    if rest.ends_with('"')
        && let Some(at) = rest.find(" \"")
        && let Some((new, len)) = unquote(&rest[at + 1..])
        && at + 1 + len == rest.len()
    {
        return Some((rest[..at].to_string(), new));
    }
    for (at, _) in rest.match_indices(' ') {
        let (old, new) = (&rest[..at], &rest[at + 1..]);
        if old
            .strip_prefix("a/")
            .is_some_and(|old| Some(old) == new.strip_prefix("b/"))
        {
            return Some((old.to_string(), new.to_string()));
        }
    }
    if let Some(at) = rest.find(" b/") {
        return Some((rest[..at].to_string(), rest[at + 1..].to_string()));
    }
    let (old, new) = rest.split_once(' ')?;
    Some((old.to_string(), new.to_string()))
}

/// A path as a header line ends with it: quoted, or bare.
fn header_path(text: &str) -> String {
    match unquote(text) {
        Some((path, _)) if text.starts_with('"') => path,
        _ => text.to_string(),
    }
}

fn strip_prefix_owned(path: String, prefix: &str) -> String {
    match path.strip_prefix(prefix) {
        Some(rest) => rest.to_string(),
        None => path,
    }
}

/// The path of a `--- a/x` or `+++ b/x` line, `None` for `/dev/null`. Git
/// ends the name with a tab when it has spaces in it, and `diff -u` puts a
/// timestamp after that tab.
fn marker_path(rest: &str, prefix: &str) -> Option<String> {
    let path = if rest.starts_with('"') {
        unquote(rest)?.0
    } else {
        rest.split('\t').next().unwrap_or(rest).to_string()
    };
    (path != "/dev/null").then(|| strip_prefix_owned(path, prefix))
}

/// A C-style quoted string at the start of `text`: what it says, and how many
/// bytes of `text` it took up, quotes included.
fn unquote(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), i + 1)),
            b'\\' => {
                let escaped = *bytes.get(i + 1)?;
                i += 2;
                match escaped {
                    b'0'..=b'7' => {
                        let mut value = u32::from(escaped - b'0');
                        for _ in 0..2 {
                            match bytes.get(i) {
                                Some(digit @ b'0'..=b'7') => {
                                    value = value * 8 + u32::from(digit - b'0');
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(u8::try_from(value).ok()?);
                    }
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'a' => out.push(0x07),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'v' => out.push(0x0b),
                    other => out.push(other),
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    None
}

/// `prefix` and `path`, quoted the way git would need to read them back when
/// the path holds a quote, a backslash or a control character.
fn quote_path(prefix: &str, path: &str) -> String {
    let full = format!("{prefix}{path}");
    if !full
        .chars()
        .any(|ch| ch == '"' || ch == '\\' || ch.is_control())
    {
        return full;
    }
    let mut out = String::from("\"");
    for ch in full.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ch if ch.is_control() => {
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("\\{byte:03o}"));
                }
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
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
mod tests {
    use super::*;

    fn kinds(hunk: &Hunk) -> Vec<LineKind> {
        hunk.lines.iter().map(|line| line.kind).collect()
    }

    #[test]
    fn a_hunk_header_reads_both_ranges_and_defaults_a_missing_length_to_one() {
        let hunk = parse_hunk_header("@@ -12,3 +42,8 @@ fn main()").unwrap();
        assert_eq!(
            (hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len),
            (12, 3, 42, 8)
        );
        assert_eq!(hunk.header, "@@ -12,3 +42,8 @@ fn main()");

        let short = parse_hunk_header("@@ -5 +5 @@").unwrap();
        assert_eq!(
            (
                short.old_start,
                short.old_len,
                short.new_start,
                short.new_len
            ),
            (5, 1, 5, 1)
        );

        let new_file = parse_hunk_header("@@ -0,0 +1,2 @@").unwrap();
        assert_eq!((new_file.old_start, new_file.old_len), (0, 0));
        assert_eq!((new_file.new_start, new_file.new_len), (1, 2));
    }

    #[test]
    fn a_line_that_is_not_a_hunk_header_is_refused() {
        assert!(parse_hunk_header("not a hunk").is_none());
        assert!(parse_hunk_header("@@@ -1 -1 +1 @@@").is_none());
        assert!(parse_hunk_header("@@ -a +1 @@").is_none());
        assert!(parse_hunk_header("@@ +1 -1 @@").is_none());
    }

    #[test]
    fn header_paths_with_spaces_are_split_where_both_sides_agree() {
        assert_eq!(
            diff_git_paths("diff --git a/src/main.rs b/src/main.rs"),
            Some(("src/main.rs".into(), "src/main.rs".into()))
        );
        assert_eq!(
            diff_git_paths("diff --git a/my file b/x.txt b/my file b/x.txt"),
            Some(("my file b/x.txt".into(), "my file b/x.txt".into()))
        );
        assert_eq!(
            diff_git_paths("diff --git a/old.rs b/new.rs"),
            Some(("old.rs".into(), "new.rs".into()))
        );
        assert_eq!(diff_git_paths("index 123..456"), None);
    }

    #[test]
    fn quoted_header_paths_are_unescaped() {
        assert_eq!(
            diff_git_paths(r#"diff --git "a/t\303\251st \"q\".txt" "b/t\303\251st \"q\".txt""#),
            Some(("tést \"q\".txt".into(), "tést \"q\".txt".into()))
        );
        assert_eq!(
            diff_git_paths(r#"diff --git a/plain.txt "b/tab\there.txt""#),
            Some(("plain.txt".into(), "tab\there.txt".into()))
        );
        assert_eq!(
            diff_git_paths(r#"diff --git "a/tab\there.txt" b/plain.txt"#),
            Some(("tab\there.txt".into(), "plain.txt".into()))
        );
    }

    #[test]
    fn every_file_and_hunk_is_read_with_lines_numbered_on_their_side() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@ mod a;
 keep
-old
+new
 tail
@@ -10 +10,2 @@
 ten
+eleven
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -4,2 +4 @@
 four
-five
";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 2);
        let lib = &files[0];
        assert_eq!(lib.path(), "src/lib.rs");
        assert_eq!(lib.hunks.len(), 2);
        assert_eq!(
            kinds(&lib.hunks[0]),
            [
                LineKind::Context,
                LineKind::Removed,
                LineKind::Added,
                LineKind::Context
            ]
        );
        let old = &lib.hunks[0].lines[1];
        assert_eq!(
            (old.old_no, old.new_no, old.text.as_str()),
            (Some(2), None, "old")
        );
        let new = &lib.hunks[0].lines[2];
        assert_eq!(
            (new.old_no, new.new_no, new.text.as_str()),
            (None, Some(2), "new")
        );
        let tail = &lib.hunks[0].lines[3];
        assert_eq!((tail.old_no, tail.new_no), (Some(3), Some(3)));
        assert_eq!(lib.hunks[1].lines[1].new_no, Some(11));

        let b = &files[1];
        assert_eq!(b.path(), "b.txt");
        assert_eq!(b.hunks[0].lines[1].old_no, Some(5));
    }

    #[test]
    fn a_removed_line_that_looks_like_a_header_stays_in_its_hunk() {
        let diff = "\
diff --git a/notes.md b/notes.md
--- a/notes.md
+++ b/notes.md
@@ -1,2 +1,2 @@
--- a rule
+++ a heading
 after
";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        let hunk = &files[0].hunks[0];
        assert_eq!(
            kinds(hunk),
            [LineKind::Removed, LineKind::Added, LineKind::Context]
        );
        assert_eq!(hunk.lines[0].text, "-- a rule");
        assert_eq!(files[0].header.len(), 3);
    }

    #[test]
    fn new_deleted_renamed_and_binary_files_say_so() {
        let diff = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/new.rs
@@ -0,0 +1 @@
+fn main() {}
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn gone() {}
diff --git a/old name.rs b/new name.rs
similarity index 100%
rename from old name.rs
rename to new name.rs
diff --git a/logo.png b/logo.png
index 1..2 100644
Binary files a/logo.png and b/logo.png differ
";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 4);

        assert!(files[0].is_new && !files[0].is_deleted);
        assert_eq!(files[0].path(), "new.rs");
        assert_eq!(files[0].hunks[0].lines[0].new_no, Some(1));

        assert!(files[1].is_deleted && !files[1].is_new);
        assert_eq!(files[1].path(), "gone.rs");
        assert_eq!(files[1].hunks[0].lines[0].old_no, Some(1));

        assert!(files[2].is_rename);
        assert_eq!(files[2].old_path, "old name.rs");
        assert_eq!(files[2].new_path, "new name.rs");
        assert!(files[2].hunks.is_empty());

        assert!(files[3].is_binary);
        assert_eq!(files[3].path(), "logo.png");
        assert!(files[3].hunks.is_empty());
    }

    #[test]
    fn a_missing_newline_at_the_end_is_kept_with_the_line_it_is_about() {
        let diff = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
";
        let files = parse_patch(diff);
        let hunk = &files[0].hunks[0];
        assert_eq!(
            kinds(hunk),
            [
                LineKind::Removed,
                LineKind::NoNewline,
                LineKind::Added,
                LineKind::NoNewline
            ]
        );
        assert_eq!(
            hunk.lines[3].to_patch_line(),
            "\\ No newline at end of file"
        );
    }

    #[test]
    fn a_marker_path_with_spaces_drops_the_tab_git_ends_it_with() {
        let diff = "--- a/my file.txt\t\n+++ b/my file.txt\t\n@@ -1 +1 @@\n-a\n+b\n";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].old_path, "my file.txt");
        assert_eq!(files[0].new_path, "my file.txt");
        assert_eq!(files[0].hunks[0].lines.len(), 2);
    }

    #[test]
    fn a_commit_header_before_the_diff_is_not_a_file() {
        let diff = "commit abc\nAuthor: A <a@b>\n\n    subject\n\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path(), "x");
    }

    #[test]
    fn one_hunk_is_written_back_as_a_patch_of_its_own() {
        let diff = "\
diff --git a/a.txt b/a.txt
index 1..2 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
-one
+uno
 two
@@ -9 +9 @@ ctx
-nine
\\ No newline at end of file
+nueve
\\ No newline at end of file
";
        let files = parse_patch(diff);
        assert_eq!(
            files[0].hunk_patch(1).unwrap(),
            "\
diff --git a/a.txt b/a.txt
index 1..2 100644
--- a/a.txt
+++ b/a.txt
@@ -9 +9 @@ ctx
-nine
\\ No newline at end of file
+nueve
\\ No newline at end of file
"
        );
        assert!(files[0].hunk_patch(2).is_none());
    }

    #[test]
    fn a_hunk_patch_reparses_to_the_same_hunk_and_keeps_crlf_lines() {
        let diff = "diff --git a/w.txt b/w.txt\n--- a/w.txt\n+++ b/w.txt\n@@ -1,2 +1,2 @@\n-a\r\n+b\r\n c\r\n";
        let file = &parse_patch(diff)[0];
        assert_eq!(file.hunks[0].lines[0].text, "a\r");
        let again = parse_patch(&file.hunk_patch(0).unwrap());
        assert_eq!(again[0].hunks, file.hunks);
        assert_eq!(again[0].path(), "w.txt");
    }

    #[test]
    fn a_hunk_without_a_file_header_gets_one_made_up() {
        let file = FilePatch {
            old_path: "new file.txt".into(),
            new_path: "new file.txt".into(),
            is_new: true,
            hunks: parse_patch("@@ -0,0 +1 @@\n+hi\n").remove(0).hunks,
            ..FilePatch::default()
        };
        let patch = file.hunk_patch(0).unwrap();
        assert_eq!(
            patch,
            "diff --git a/new file.txt b/new file.txt\n--- /dev/null\n+++ b/new file.txt\n@@ -0,0 +1 @@\n+hi\n"
        );
        let reparsed = &parse_patch(&patch)[0];
        assert!(reparsed.is_new);
        assert_eq!(reparsed.path(), "new file.txt");
    }

    #[test]
    fn a_made_up_header_quotes_a_path_git_would_quote() {
        let file = FilePatch {
            old_path: "say \"hi\".txt".into(),
            new_path: "say \"hi\".txt".into(),
            hunks: parse_patch("@@ -1 +1 @@\n-a\n+b\n").remove(0).hunks,
            ..FilePatch::default()
        };
        let reparsed = &parse_patch(&file.hunk_patch(0).unwrap())[0];
        assert_eq!(reparsed.old_path, "say \"hi\".txt");
        assert_eq!(reparsed.new_path, "say \"hi\".txt");
    }

    #[test]
    fn an_empty_line_inside_a_hunk_counts_as_blank_context() {
        let diff = "@@ -1,3 +1,3 @@\n a\n\n-b\n+c\n";
        let hunk = &parse_patch(diff)[0].hunks[0];
        assert_eq!(
            kinds(hunk),
            [
                LineKind::Context,
                LineKind::Context,
                LineKind::Removed,
                LineKind::Added
            ]
        );
        assert_eq!(hunk.lines[1].old_no, Some(2));
    }

    /// What a later hunk-staging step relies on: one hunk, written back out,
    /// stages that hunk and nothing else.
    #[test]
    fn a_single_hunk_patch_applies_to_the_index_on_its_own() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let git = |args: &[&str], stdin: Option<&str>| {
            use std::io::Write;
            let mut child = std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "core.autocrlf=false",
                ])
                .args(["-c", "commit.gpgsign=false", "-c", "color.ui=false"])
                .args(["-c", "diff.noprefix=false"])
                .args(args)
                .current_dir(tmp.path())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("run git");
            if let Some(input) = stdin {
                child
                    .stdin
                    .take()
                    .expect("stdin")
                    .write_all(input.as_bytes())
                    .expect("write patch");
            }
            let out = child.wait_with_output().expect("wait for git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
        std::fs::write(tmp.path().join("my file.txt"), lines.join("\n") + "\n").unwrap();
        git(&["init", "-q"], None);
        git(&["add", "."], None);
        git(&["commit", "-q", "-m", "base"], None);
        let mut changed = lines.clone();
        changed[1] = "line two".into();
        changed[27] = "line twenty-eight".into();
        std::fs::write(tmp.path().join("my file.txt"), changed.join("\n") + "\n").unwrap();

        let files = parse_patch(&git(&["diff"], None));
        assert_eq!(files[0].path(), "my file.txt");
        assert_eq!(files[0].hunks.len(), 2);
        git(
            &["apply", "--cached", "-"],
            Some(&files[0].hunk_patch(1).unwrap()),
        );

        let staged = git(&["diff", "--cached"], None);
        assert!(staged.contains("+line twenty-eight"), "{staged}");
        assert!(!staged.contains("+line two\n"), "{staged}");
        let unstaged = git(&["diff"], None);
        assert!(unstaged.contains("+line two\n"), "{unstaged}");
    }
}
