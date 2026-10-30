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
    let diff =
        "diff --git a/w.txt b/w.txt\n--- a/w.txt\n+++ b/w.txt\n@@ -1,2 +1,2 @@\n-a\r\n+b\r\n c\r\n";
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
