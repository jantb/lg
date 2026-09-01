//! What `.gitattributes` says about a path, and what that means for review.
//!
//! Two attributes decide whether lg shows a file's contents. Git enforces
//! `-diff` itself — the patch for such a file is already `Binary files ...
//! differ` — but `linguist-generated` and `linguist-vendored` are conventions
//! git knows nothing about, so a bundle or a lockfile still arrives as a full
//! patch. Both mean the same thing here: the file changed, which is worth
//! reporting, but its contents are machine output that no reviewer reads and
//! no prompt should carry.

use anyhow::Result;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::process::Stdio;

use super::git_command;

/// The line standing in for a suppressed file's patch.
///
/// Distinctive enough that a real diff body cannot be mistaken for it: a patch
/// line always starts with a space, `+`, `-`, `@`, or `\`.
pub const SUPPRESSED_DIFF_MARKER: &str = "~ contents suppressed";

/// Whether a rendered patch body is a suppressed file's stand-in rather than a
/// diff, so callers can skip reading the file to display it.
pub fn is_suppressed_diff_body(body: &[String]) -> bool {
    body.iter()
        .any(|line| line.trim_start().starts_with(SUPPRESSED_DIFF_MARKER))
}

/// The attributes that decide whether a path's contents are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileAttrs {
    /// `-diff`: git itself withholds the patch.
    pub no_diff: bool,
    /// `linguist-generated`: machine output committed for convenience.
    pub generated: bool,
    /// `linguist-vendored`: third-party code carried in-tree.
    pub vendored: bool,
}

impl FileAttrs {
    /// Whether this file's contents should be withheld from the review and any
    /// prompt built from it.
    pub fn suppressed(&self) -> bool {
        self.no_diff || self.generated || self.vendored
    }

    /// Why it was withheld, for the line shown in place of the patch.
    fn reason(&self) -> &'static str {
        if self.generated {
            "linguist-generated"
        } else if self.vendored {
            "linguist-vendored"
        } else {
            "-diff"
        }
    }
}

/// Whether an attribute's value counts as switched on.
///
/// `check-attr` reports a value, not a flag: `set` for a bare `attr`, `unset`
/// for `-attr`, `unspecified` when no pattern matched, and otherwise whatever
/// string was assigned. `attr=true` is the form these are written in — and its
/// `false` counterpart is how one file is carved back out of a directory-wide
/// rule, so it has to read as an explicit no rather than as absent.
fn attr_is_set(value: &str) -> bool {
    matches!(value, "set" | "true")
}

/// The attributes of every path in one `check-attr` call.
///
/// Asking git rather than matching the patterns here is not just less code: it
/// picks up nested `.gitattributes`, `.git/info/attributes` and the user's
/// global file, and it gets the pattern semantics right. `dist/*` matches
/// `dist/app.js` but not `dist/sub/deep.js`, which almost nobody expects.
pub fn file_attrs<I, S>(paths: I) -> Result<HashMap<String, FileAttrs>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.as_ref().to_owned())
        .collect();
    if paths.is_empty() {
        return Ok(HashMap::new());
    }

    // With -z the input is NUL-separated too, not just the output.
    let mut stdin = String::new();
    for path in &paths {
        stdin.push_str(path);
        stdin.push('\0');
    }

    let mut child = git_command(&[
        "check-attr",
        "-z",
        "--stdin",
        "diff",
        "linguist-generated",
        "linguist-vendored",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()?;
    child
        .stdin
        .take()
        .map(|mut pipe| pipe.write_all(stdin.as_bytes()))
        .transpose()?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Ok(HashMap::new());
    }

    Ok(parse_check_attr(&String::from_utf8_lossy(&out.stdout)))
}

/// `path\0attr\0value\0` triples, in the order the attributes were asked for.
fn parse_check_attr(text: &str) -> HashMap<String, FileAttrs> {
    let mut attrs: HashMap<String, FileAttrs> = HashMap::new();
    let mut fields = text.split('\0').filter(|field| !field.is_empty());
    while let (Some(path), Some(attr), Some(value)) = (fields.next(), fields.next(), fields.next())
    {
        let entry = attrs.entry(path.to_owned()).or_default();
        match attr {
            "diff" => entry.no_diff = value == "unset",
            "linguist-generated" => entry.generated = attr_is_set(value),
            "linguist-vendored" => entry.vendored = attr_is_set(value),
            _ => {}
        }
    }
    attrs
}

/// Replace the body of every suppressed file's patch with a one-line stand-in,
/// keeping its header so the file still reads as changed.
///
/// Filtering the diff itself, rather than each of its readers, means the report,
/// the entry-point scan, the node tree and every prompt built from them all
/// inherit the suppression without knowing about it.
pub fn suppress_generated_diff(diff: &str) -> String {
    match diff_paths(diff) {
        paths if paths.is_empty() => diff.to_owned(),
        paths => {
            let attrs = file_attrs(&paths).unwrap_or_default();
            if attrs.values().any(FileAttrs::suppressed) {
                rewrite_suppressed(diff, &attrs)
            } else {
                diff.to_owned()
            }
        }
    }
}

fn diff_paths(diff: &str) -> Vec<String> {
    diff.lines()
        .filter_map(parse_diff_path)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parse_diff_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let (_, path) = rest.split_once(" b/")?;
    let path = path.trim();
    (!path.is_empty() && path != "/dev/null").then(|| path.to_owned())
}

/// A patch header line, kept so the file still shows as changed.
fn is_patch_header(line: &str) -> bool {
    line.starts_with("index ")
        || line.starts_with("old mode ")
        || line.starts_with("new mode ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("similarity index ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
}

fn rewrite_suppressed(diff: &str, attrs: &HashMap<String, FileAttrs>) -> String {
    let mut out = String::with_capacity(diff.len());
    let mut suppressing: Option<FileAttrs> = None;
    let mut added = 0usize;
    let mut removed = 0usize;

    for line in diff.lines() {
        if let Some(path) = parse_diff_path(line) {
            flush_suppressed(&mut out, suppressing.take(), added, removed);
            added = 0;
            removed = 0;
            suppressing = attrs.get(&path).copied().filter(FileAttrs::suppressed);
            push_line(&mut out, line);
            continue;
        }
        if suppressing.is_none() {
            push_line(&mut out, line);
            continue;
        }
        if is_patch_header(line) {
            push_line(&mut out, line);
        } else if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
    }
    flush_suppressed(&mut out, suppressing, added, removed);
    out
}

fn flush_suppressed(out: &mut String, attrs: Option<FileAttrs>, added: usize, removed: usize) {
    let Some(attrs) = attrs else {
        return;
    };
    push_line(
        out,
        &format!(
            "{SUPPRESSED_DIFF_MARKER} ({}): +{added} -{removed}",
            attrs.reason()
        ),
    );
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_is_set_by_value_as_well_as_by_flag() {
        let text = "a.js\0linguist-generated\0true\0b.js\0linguist-generated\0set\0";
        let attrs = parse_check_attr(text);
        assert!(attrs["a.js"].suppressed());
        assert!(attrs["b.js"].suppressed());
    }

    /// `dist/keep.js linguist-generated=false` is how one file is carved back
    /// out of a directory-wide rule.
    #[test]
    fn generated_false_is_not_suppressed() {
        let text =
            "keep.js\0linguist-generated\0false\0other.js\0linguist-generated\0unspecified\0";
        let attrs = parse_check_attr(text);
        assert!(!attrs["keep.js"].suppressed());
        assert!(!attrs["other.js"].suppressed());
    }

    #[test]
    fn minus_diff_is_suppressed_but_plain_diff_is_not() {
        let text = "gen.txt\0diff\0unset\0src.rs\0diff\0unspecified\0hl.rs\0diff\0rust\0";
        let attrs = parse_check_attr(text);
        assert!(attrs["gen.txt"].suppressed());
        assert!(!attrs["src.rs"].suppressed());
        assert!(!attrs["hl.rs"].suppressed());
    }

    #[test]
    fn suppressed_file_keeps_its_header_and_counts_but_loses_its_body() {
        let diff = "\
diff --git a/dist/app.js b/dist/app.js
index 1111111..2222222 100644
--- a/dist/app.js
+++ b/dist/app.js
@@ -1,2 +1,3 @@
 keep
-gone
+new
+more
diff --git a/src/main.rs b/src/main.rs
index 3333333..4444444 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1 +1 @@
-old
+real change
";
        let attrs = HashMap::from([(
            "dist/app.js".to_string(),
            FileAttrs {
                generated: true,
                ..Default::default()
            },
        )]);
        let out = rewrite_suppressed(diff, &attrs);

        assert!(out.contains("diff --git a/dist/app.js b/dist/app.js"));
        assert!(!out.contains("+more"));
        assert!(out.contains("~ contents suppressed (linguist-generated): +2 -1"));
        // The file that isn't suppressed comes through untouched.
        assert!(out.contains("+real change"));
        assert!(out.contains("@@ -1 +1 @@"));
    }

    #[test]
    fn a_suppressed_body_is_recognised_and_a_patch_body_is_not() {
        assert!(is_suppressed_diff_body(&[format!(
            "{SUPPRESSED_DIFF_MARKER} (linguist-generated): +1 -0"
        )]));
        assert!(!is_suppressed_diff_body(&[
            "@@ -1 +1 @@".to_string(),
            "+contents".to_string(),
        ]));
    }
}
