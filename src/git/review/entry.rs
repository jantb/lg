//! Turning a diff into the entry points worth reading first.

use super::source::{infer_entry_symbol, matches_csharp_path, matches_kotlin_path, source_context};
use super::{ReviewEntryPoint, truncate_review_text};

pub(super) fn review_entry_points(diff: &str) -> Vec<ReviewEntryPoint> {
    let mut entries = Vec::new();
    let mut current_path = String::new();
    let mut current_hunk: Option<ReviewHunk> = None;

    for line in diff.lines() {
        if let Some(path) = parse_review_diff_path(line) {
            flush_review_hunk(&mut entries, &current_path, current_hunk.take());
            current_path = path;
            continue;
        }
        if line.starts_with("@@") {
            flush_review_hunk(&mut entries, &current_path, current_hunk.take());
            let new_line = parse_new_hunk_start(line).unwrap_or(0);
            current_hunk = Some(ReviewHunk {
                start_line: new_line,
                current_line: new_line,
                first_added_line: None,
                hunk: line.to_string(),
                patch: vec![line.to_string()],
                added: 0,
                removed: 0,
            });
            continue;
        }
        if let Some(hunk) = current_hunk.as_mut() {
            hunk.patch.push(line.to_string());
            if line.starts_with('+') && !line.starts_with("+++") {
                hunk.added += 1;
                hunk.first_added_line.get_or_insert(hunk.current_line);
                hunk.current_line = hunk.current_line.saturating_add(1);
            } else if line.starts_with('-') && !line.starts_with("---") {
                hunk.removed += 1;
            } else if !line.starts_with('\\') {
                hunk.current_line = hunk.current_line.saturating_add(1);
            }
        }
    }
    flush_review_hunk(&mut entries, &current_path, current_hunk.take());
    entries
}

struct ReviewHunk {
    start_line: usize,
    current_line: usize,
    first_added_line: Option<usize>,
    hunk: String,
    patch: Vec<String>,
    added: usize,
    removed: usize,
}

fn parse_review_diff_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let (_, b_path) = rest.split_once(" b/")?;
    Some(b_path.to_owned())
}

fn flush_review_hunk(entries: &mut Vec<ReviewEntryPoint>, path: &str, hunk: Option<ReviewHunk>) {
    let Some(hunk) = hunk else {
        return;
    };
    if path.is_empty() {
        return;
    }
    if is_import_only_hunk(path, &hunk.patch) {
        return;
    }
    let line = hunk.first_added_line.unwrap_or(hunk.start_line);
    let symbol = infer_entry_symbol(path, line, &hunk.hunk);
    let context = source_context(path, line);
    let description = describe_hunk(&hunk.patch, hunk.added, hunk.removed);
    entries.push(ReviewEntryPoint {
        path: path.to_string(),
        line: (line > 0).then_some(line),
        symbol,
        description,
        hunk: hunk.hunk,
        patch: hunk.patch,
        context,
        added: hunk.added,
        removed: hunk.removed,
    });
}

/// The added and removed lines of a patch, without their leading marker.
fn changed_bodies(patch: &[String]) -> impl Iterator<Item = &str> {
    patch
        .iter()
        .filter(|line| {
            (line.starts_with('+') && !line.starts_with("+++"))
                || (line.starts_with('-') && !line.starts_with("---"))
        })
        .map(|line| &line[1..])
}

fn is_import_only_hunk(path: &str, patch: &[String]) -> bool {
    let mut changed = 0usize;
    for body in changed_bodies(patch).map(str::trim) {
        if body.is_empty() {
            continue;
        }
        changed += 1;
        if !is_import_line(path, body) {
            return false;
        }
    }
    changed > 0
}

fn is_import_line(path: &str, line: &str) -> bool {
    let line = line
        .strip_prefix("pub ")
        .or_else(|| line.strip_prefix("public "))
        .unwrap_or(line);
    if path.ends_with(".rs") {
        line.starts_with("use ") || line.starts_with("extern crate ")
    } else if matches_kotlin_path(path) || path.ends_with(".java") {
        line.starts_with("import ") || line.starts_with("package ")
    } else if matches_csharp_path(path) {
        line.starts_with("using ") || line.starts_with("namespace ")
    } else {
        line.starts_with("import ") || line.starts_with("from ") || line.starts_with("export ")
    }
}

fn describe_hunk(patch: &[String], added: usize, removed: usize) -> String {
    let operation = match (added > 0, removed > 0) {
        (true, true) => "updates",
        (true, false) => "adds",
        (false, true) => "removes",
        (false, false) => "touches",
    };
    let mut signals = Vec::new();
    for body in changed_bodies(patch) {
        collect_signal_words(body, &mut signals);
        if signals.len() >= 4 {
            break;
        }
    }
    if signals.is_empty() {
        format!("{operation} this block (+{added} -{removed})")
    } else {
        format!(
            "{operation} {} (+{added} -{removed})",
            signals.into_iter().take(4).collect::<Vec<_>>().join(", ")
        )
    }
}

fn collect_signal_words(line: &str, signals: &mut Vec<String>) {
    let trimmed = line.trim();
    if trimmed.is_empty() || matches!(trimmed, "{" | "}" | ");" | ")" | "]") {
        return;
    }
    for word in trimmed
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map(str::trim)
        .filter(|word| word.chars().count() >= 3)
        .filter(|word| !is_low_signal_word(word))
    {
        let word = truncate_review_text(word, 32);
        if !signals.contains(&word) {
            signals.push(word);
        }
        if signals.len() >= 4 {
            return;
        }
    }
}

fn is_low_signal_word(word: &str) -> bool {
    matches!(
        word,
        "let"
            | "mut"
            | "pub"
            | "fn"
            | "impl"
            | "self"
            | "Some"
            | "None"
            | "true"
            | "false"
            | "String"
            | "Vec"
            | "format"
            | "return"
            | "val"
            | "var"
            | "fun"
            | "class"
            | "object"
    )
}

fn parse_new_hunk_start(line: &str) -> Option<usize> {
    let plus = line.find(" +")? + 2;
    let rest = &line[plus..];
    let end = rest
        .find(|c: char| c == ',' || c.is_whitespace())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_hunk_start_reads_added_side() {
        assert_eq!(parse_new_hunk_start("@@ -12,3 +42,8 @@ fn main"), Some(42));
        assert_eq!(parse_new_hunk_start("@@ -1 +1 @@"), Some(1));
        assert_eq!(parse_new_hunk_start("not a hunk"), None);
    }

    #[test]
    fn is_import_line_recognises_per_language() {
        assert!(is_import_line("foo.rs", "use std::fs;"));
        assert!(is_import_line("foo.rs", "extern crate serde;"));
        assert!(!is_import_line("foo.rs", "let x = 1;"));

        assert!(is_import_line("foo.kt", "import a.b.C"));
        assert!(is_import_line("foo.kt", "package a.b"));
        // Public modifier in front of an import still counts.
        assert!(is_import_line("foo.kt", "public import a.b.C"));
    }

    #[test]
    fn is_import_only_hunk_returns_true_only_when_all_changes_are_imports() {
        let imports = vec![
            "@@ -1 +1 @@".to_string(),
            "+use std::fs;".to_string(),
            "-use std::io;".to_string(),
        ];
        assert!(is_import_only_hunk("foo.rs", &imports));

        let mixed = vec![
            "@@ -1 +1 @@".to_string(),
            "+use std::fs;".to_string(),
            "+let x = 1;".to_string(),
        ];
        assert!(!is_import_only_hunk("foo.rs", &mixed));

        // No actual changes (only headers / context) → false.
        let empty = vec!["@@ -1 +1 @@".to_string(), " context".to_string()];
        assert!(!is_import_only_hunk("foo.rs", &empty));
    }

    #[test]
    fn describe_hunk_picks_operation_word_from_added_removed() {
        let patch = vec!["+let foo = 1;".to_string()];
        let desc = describe_hunk(&patch, 1, 0);
        assert!(desc.starts_with("adds "));
        assert!(desc.contains("(+1 -0)"));

        let patch = vec!["-let bar = 1;".to_string()];
        let desc = describe_hunk(&patch, 0, 1);
        assert!(desc.starts_with("removes "));

        let patch = vec!["+a".to_string(), "-b".to_string()];
        let desc = describe_hunk(&patch, 1, 1);
        assert!(desc.starts_with("updates "));
    }
}
