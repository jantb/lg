use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Mutex, MutexGuard},
};
use tempfile::TempDir;

static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Run a git command inside `dir`, with author/committer env vars set.
fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("failed to run git")
}

/// Same but panics if exit code != 0.
fn git_ok(dir: &std::path::Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git_ok(dir.path(), &["init", "-b", "main"]);
    git_ok(dir.path(), &["config", "user.email", "test@example.com"]);
    git_ok(dir.path(), &["config", "user.name", "Test User"]);
    dir
}

/// Call `lg::git::status_porcelain` inside a specific working directory.
fn status_in(dir: &std::path::Path) -> (Vec<String>, Vec<String>) {
    // We need to run in that directory, so we temporarily change the process
    // working directory — but that's not thread-safe.  Instead we shell out
    // to git directly and feed the bytes to the parser.
    let out = Command::new("git")
        .args(["status", "-z", "--porcelain=v1"])
        .current_dir(dir)
        .output()
        .expect("git status");
    lg::git::parse_porcelain(&out.stdout)
}

fn stage_in(dir: &std::path::Path, path: &str) {
    git_ok(dir, &["add", "--", path]);
}

fn unstage_in(dir: &std::path::Path, path: &str) {
    // pre-initial-commit: use rm --cached
    let out = git(dir, &["reset", "-q", "HEAD", "--", path]);
    if !out.status.success() {
        let out2 = git(dir, &["rm", "--cached", "--", path]);
        assert!(
            out2.status.success(),
            "unstage failed: {}",
            String::from_utf8_lossy(&out2.stderr)
        );
    }
}

#[test]
fn add_to_gitignore_appends_file_and_folder_entries_once() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    assert_eq!(
        lg::git::add_to_gitignore("./logs/debug.log", false).unwrap(),
        "ignored logs/debug.log"
    );
    assert_eq!(
        lg::git::add_to_gitignore("tmp/cache", true).unwrap(),
        "ignored tmp/cache/"
    );
    assert_eq!(
        lg::git::add_to_gitignore("tmp/cache/", true).unwrap(),
        "tmp/cache/ already ignored"
    );

    let ignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(ignore, "logs/debug.log\ntmp/cache/\n");
}

#[test]
fn project_open_command_opens_rust_repo_root() {
    let dir = init_repo();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"sample\"\n",
    )
    .unwrap();
    let _cwd = CwdGuard::new(dir.path());

    let command = lg::git::project_open_command().unwrap();

    assert_eq!(command.program, "rustrover");
    assert_eq!(command.args.len(), 1);
    assert_eq!(
        fs::canonicalize(&command.args[0]).unwrap(),
        fs::canonicalize(dir.path()).unwrap()
    );
    assert_eq!(command.line, 1);
}

fn commit_in(dir: &std::path::Path, msg: &str) {
    let out = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit_in_as(dir: &std::path::Path, msg: &str, author: &str, email: &str) {
    let out = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", author)
        .env("GIT_COMMITTER_EMAIL", email)
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct CwdGuard {
    old: std::path::PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl CwdGuard {
    fn new(dir: &Path) -> Self {
        let lock = CWD_LOCK.lock().expect("cwd lock poisoned");
        let old = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir).expect("set current dir");
        Self { old, _lock: lock }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.old).expect("restore current dir");
    }
}

fn head_branch(dir: &Path) -> String {
    let out = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert!(
        out.status.success(),
        "git rev-parse failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn status_porcelain_on_fresh_repo_is_empty() {
    let dir = init_repo();
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.is_empty(),
        "expected no unstaged, got {unstaged:?}"
    );
    assert!(staged.is_empty(), "expected no staged, got {staged:?}");
}

#[test]
fn stage_then_unstage_round_trips() {
    let dir = init_repo();
    let file = dir.path().join("hello.txt");
    fs::write(&file, "hello").unwrap();

    // After writing: file is untracked (unstaged only).
    let (u, s) = status_in(dir.path());
    assert!(u.contains(&"hello.txt".to_string()), "should be untracked");
    assert!(!s.contains(&"hello.txt".to_string()));

    // Stage it.
    stage_in(dir.path(), "hello.txt");
    let (u, s) = status_in(dir.path());
    assert!(s.contains(&"hello.txt".to_string()), "should be staged");
    assert!(!u.contains(&"hello.txt".to_string()));

    // Unstage it.
    unstage_in(dir.path(), "hello.txt");
    let (u, s) = status_in(dir.path());
    assert!(
        u.contains(&"hello.txt".to_string()),
        "should be back in unstaged"
    );
    assert!(!s.contains(&"hello.txt".to_string()));
}

#[test]
fn head_branch_returns_current_branch() {
    let dir = init_repo();
    // Need at least one commit for HEAD to resolve.
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    assert_eq!(branch, "main");
}

#[test]
fn assisted_review_reports_diff_and_entry_points_against_main() {
    let dir = init_repo();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hello\"\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/review"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hello review\"\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "update greeting");

    let _cwd = CwdGuard::new(dir.path());
    let report = lg::git::assisted_review_against_main().unwrap();

    assert!(report.contains("Assisted review against main"), "{report}");
    assert!(report.contains("Base: main"), "{report}");
    assert!(report.contains("Full diff against main"), "{report}");
    assert!(report.contains("src/lib.rs"), "{report}");
    assert!(report.contains("fn greet"), "{report}");
    assert!(report.contains("\"hello review\""), "{report}");

    let review = lg::git::build_assisted_review_against_main().unwrap();
    let hunk_pos = review
        .nodes
        .iter()
        .position(|node| node.id.starts_with("branch:hunk:"))
        .expect("hunk node");
    let entry_pos = review
        .nodes
        .iter()
        .position(|node| node.title.contains("fn greet"))
        .expect("entry node");
    let file_pos = review
        .nodes
        .iter()
        .position(|node| node.id.starts_with("branch:file:") && node.title.contains("src/lib.rs"))
        .expect("file node");
    assert_eq!(review.nodes[0].title, "Full diff against main");
    assert_eq!(
        review.nodes[file_pos].parent.as_deref(),
        Some("branch"),
        "file should be directly under the full diff root"
    );
    assert_eq!(review.nodes[file_pos].depth, 1);
    assert_eq!(
        review.nodes[entry_pos].parent.as_deref(),
        Some(review.nodes[file_pos].id.as_str()),
        "entry point should be nested under its file"
    );
    assert_eq!(review.nodes[entry_pos].depth, 2);
    assert_eq!(
        review.nodes[hunk_pos].parent.as_deref(),
        Some(review.nodes[entry_pos].id.as_str()),
        "hunk should be nested under its entry point"
    );
    assert_eq!(review.nodes[hunk_pos].depth, 3);
    assert!(file_pos < 3, "file node should appear before metadata");
    assert!(
        review.nodes.iter().all(|node| node.id != "full-diff"),
        "interactive review should not have a flat full-diff lump"
    );
    assert!(
        review.nodes[hunk_pos].title.contains(" - updates "),
        "hunk title should include a description: {}",
        review.nodes[hunk_pos].title
    );
    assert!(
        review.nodes[hunk_pos]
            .body
            .first()
            .is_some_and(|line| line.starts_with("effect: updates")),
        "expanded hunk should start with effect description: {:?}",
        review.nodes[hunk_pos].body
    );
}

#[test]
fn assisted_review_groups_multiple_hunks_under_same_entry_point() {
    let dir = init_repo();
    fs::create_dir(dir.path().join("src")).unwrap();
    let filler = (0..20)
        .map(|i| format!("    out.push_str(\"{i}\");"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        dir.path().join("src/lib.rs"),
        format!(
            "pub fn greet() -> String {{\n    let mut out = String::new();\n    out.push_str(\"hello\");\n{filler}\n    out.push_str(\"world\");\n    out\n}}\n"
        ),
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/review-group"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        format!(
            "pub fn greet() -> String {{\n    let mut out = String::new();\n    out.push_str(\"hello review\");\n{filler}\n    out.push_str(\"world review\");\n    out\n}}\n"
        ),
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "update greeting parts");

    let _cwd = CwdGuard::new(dir.path());
    let review = lg::git::build_assisted_review_against_main().unwrap();
    let entry_nodes: Vec<_> = review
        .nodes
        .iter()
        .filter(|node| node.title.contains("fn greet"))
        .collect();
    assert_eq!(entry_nodes.len(), 1, "same entry point should be grouped");
    let file_nodes: Vec<_> = review
        .nodes
        .iter()
        .filter(|node| node.id.starts_with("branch:file:") && node.title.contains("src/lib.rs"))
        .collect();
    assert_eq!(file_nodes.len(), 1, "same file should be listed once");
    let hunk_count = review
        .nodes
        .iter()
        .filter(|node| node.parent.as_deref() == Some(entry_nodes[0].id.as_str()))
        .count();
    assert_eq!(hunk_count, 2, "separate hunks should share one entry point");
}

#[test]
fn assisted_review_nests_entry_points_when_hunk_calls_changed_function() {
    let dir = init_repo();
    fs::create_dir_all(dir.path().join("src/main/kotlin")).unwrap();
    let spacer = (0..12)
        .map(|idx| format!("    fun spacer{idx}() = {idx}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        dir.path().join("src/main/kotlin/App.kt"),
        format!(
            "class App {{\n    fun nextStep(): String {{\n        return \"done\"\n    }}\n\n{spacer}\n\n    fun maybeTransferPointsToHousehold(): String {{\n        return \"skip\"\n    }}\n}}\n"
        ),
    )
    .unwrap();
    stage_in(dir.path(), "src/main/kotlin/App.kt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/nested-review"]);
    fs::write(
        dir.path().join("src/main/kotlin/App.kt"),
        format!(
            "class App {{\n    fun nextStep(): String {{\n        return maybeTransferPointsToHousehold()\n    }}\n\n{spacer}\n\n    fun maybeTransferPointsToHousehold(): String {{\n        return \"transfer\"\n    }}\n}}\n"
        ),
    )
    .unwrap();
    stage_in(dir.path(), "src/main/kotlin/App.kt");
    commit_in(dir.path(), "wire nested flow");

    let _cwd = CwdGuard::new(dir.path());
    let review = lg::git::build_assisted_review_against_main().unwrap();
    let next_step = review
        .nodes
        .iter()
        .position(|node| node.title.contains("fun nextStep"))
        .expect("nextStep entry");
    let maybe_transfer = review
        .nodes
        .iter()
        .position(|node| node.title.contains("fun maybeTransferPointsToHousehold"))
        .expect("callee entry");
    let file_nodes: Vec<_> = review
        .nodes
        .iter()
        .filter(|node| {
            node.id.starts_with("branch:file:") && node.title.contains("src/main/kotlin/App.kt")
        })
        .collect();

    assert_eq!(file_nodes.len(), 1, "same file should be listed once");
    assert_eq!(
        review.nodes[next_step].parent.as_deref(),
        Some(file_nodes[0].id.as_str()),
        "caller entry should be nested under its file"
    );
    assert_eq!(
        review.nodes[maybe_transfer].parent.as_deref(),
        Some(review.nodes[next_step].id.as_str()),
        "callee entry should be nested under caller entry: {:?}",
        review.nodes
    );
    assert_eq!(file_nodes[0].depth, 1);
    assert_eq!(review.nodes[next_step].depth, 2);
    assert_eq!(review.nodes[maybe_transfer].depth, 3);
}

#[test]
fn assisted_review_filters_import_only_hunks_from_entrypoints() {
    let dir = init_repo();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub fn greet() {}\n").unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/import-only"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        "use std::fmt;\n\npub fn greet() {}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "add import");

    let _cwd = CwdGuard::new(dir.path());
    let review = lg::git::build_assisted_review_against_main().unwrap();
    assert!(
        review
            .nodes
            .iter()
            .all(|node| !node.id.starts_with("branch:hunk:")),
        "import-only hunks should not become entry points: {:?}",
        review.nodes
    );
    assert!(
        review.nodes[0]
            .body
            .iter()
            .any(|line| line.contains("import changes hidden")),
        "root should explain hidden import-only changes: {:?}",
        review.nodes[0].body
    );
}

#[test]
fn assisted_review_ignores_whitespace_only_changes() {
    let dir = init_repo();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/format-only"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() {\n        println!(\"hello\");\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "format greeting");

    let _cwd = CwdGuard::new(dir.path());
    let review = lg::git::build_assisted_review_against_main().unwrap();

    assert!(
        review.report.contains("(empty)"),
        "whitespace-only branch diff should be empty: {}",
        review.report
    );
    assert!(
        review
            .nodes
            .iter()
            .all(|node| !node.id.starts_with("branch:hunk:")),
        "whitespace-only hunks should not become entry points: {:?}",
        review.nodes
    );
}

#[test]
fn assisted_review_reports_kotlin_entry_points() {
    let dir = init_repo();
    fs::create_dir_all(dir.path().join("src/main/kotlin")).unwrap();
    fs::write(
        dir.path().join("src/main/kotlin/App.kt"),
        "class App {\n    fun greeting(): String = \"hello\"\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/main/kotlin/App.kt");
    commit_in(dir.path(), "initial kotlin");

    git_ok(dir.path(), &["checkout", "-b", "feature/kotlin-review"]);
    fs::write(
        dir.path().join("src/main/kotlin/App.kt"),
        "class App {\n    fun greeting(): String = \"hello review\"\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/main/kotlin/App.kt");
    commit_in(dir.path(), "update kotlin greeting");

    let _cwd = CwdGuard::new(dir.path());
    let report = lg::git::assisted_review_against_main().unwrap();

    assert!(report.contains("src/main/kotlin/App.kt"), "{report}");
    assert!(report.contains("fun greeting"), "{report}");
    assert!(report.contains("\"hello review\""), "{report}");
}

#[test]
fn assisted_review_ignores_uncommitted_local_changes() {
    let dir = init_repo();
    fs::write(dir.path().join("tracked.txt"), "main\n").unwrap();
    stage_in(dir.path(), "tracked.txt");
    commit_in(dir.path(), "initial tracked");

    fs::write(dir.path().join("tracked.txt"), "local only\n").unwrap();
    fs::write(dir.path().join("scratch.txt"), "untracked local\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let review = lg::git::build_assisted_review_against_main().unwrap();

    assert!(
        review.report.contains("Full diff against main"),
        "{}",
        review.report
    );
    assert!(
        review.report.contains("(empty)"),
        "branch diff should be empty: {}",
        review.report
    );
    assert!(
        !review.report.contains("local only") && !review.report.contains("scratch.txt"),
        "local changes should not be included: {}",
        review.report
    );
    assert!(
        review
            .nodes
            .iter()
            .all(|node| !node.title.contains("local")),
        "local nodes should not exist: {:?}",
        review.nodes
    );
}

#[test]
fn list_commits_includes_short_author_name() {
    let dir = init_repo();
    fs::write(dir.path().join("a.txt"), "one").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in_as(
        dir.path(),
        "add authored commit",
        "Alice Example",
        "alice@example.com",
    );

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits(10).unwrap();

    assert_eq!(commits[0].author, "Alice Example");
    assert_eq!(commits[0].author_short, "AE");
    assert!(commits[0].is_first_parent);
    assert_eq!(commits[0].subject, "add authored commit");
}

#[test]
fn list_commits_for_ref_reads_selected_branch_history() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/log"]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature branch commit");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main branch commit");

    let _cwd = CwdGuard::new(dir.path());
    let feature_commits = lg::git::list_commits_for_ref("feature/log", 10).unwrap();
    let main_commits = lg::git::list_commits_for_ref("main", 10).unwrap();

    assert_eq!(feature_commits[0].subject, "feature branch commit");
    assert_eq!(main_commits[0].subject, "main branch commit");
}

#[test]
fn list_commits_marks_merge_commits_with_multiple_parents() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/merge"]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main side");
    git_ok(
        dir.path(),
        &["merge", "--no-ff", "feature/merge", "-m", "merge feature"],
    );

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits_for_ref("main", 10).unwrap();

    assert_eq!(commits[0].subject, "merge feature");
    assert_eq!(commits[0].parent_count(), 2);
    assert!(commits[0].is_first_parent);
    assert!(
        commits
            .iter()
            .any(|commit| commit.subject == "feature side" && !commit.is_first_parent),
        "merged-in feature commit should not be on the first-parent branch: {commits:?}"
    );
}

#[test]
fn list_commits_renders_complex_merges_with_lazygit_glyphs() {
    let dir = init_repo();
    fs::write(dir.path().join("base.txt"), "base").unwrap();
    stage_in(dir.path(), "base.txt");
    commit_in(dir.path(), "base");

    git_ok(dir.path(), &["checkout", "-b", "side-a"]);
    fs::write(dir.path().join("a.txt"), "a1").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "a1");
    fs::write(dir.path().join("a.txt"), "a2").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "a2");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main1").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main1");
    git_ok(dir.path(), &["merge", "--no-ff", "side-a", "-m", "merge-a"]);

    git_ok(dir.path(), &["checkout", "-b", "side-b", "HEAD~1"]);
    fs::write(dir.path().join("b.txt"), "b1").unwrap();
    stage_in(dir.path(), "b.txt");
    commit_in(dir.path(), "b1");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main2").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main2");
    git_ok(dir.path(), &["merge", "--no-ff", "side-b", "-m", "merge-b"]);

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits_for_ref("main", 20).unwrap();

    // 8 real commits: base, a1, a2, main1, merge-a, b1, main2, merge-b.
    assert_eq!(commits.len(), 8);
    assert!(
        commits.iter().all(|commit| !commit.subject.is_empty()),
        "every rendered row should be a real commit with a subject: {commits:?}"
    );
    let merge_a = commits
        .iter()
        .find(|commit| commit.subject == "merge-a")
        .expect("merge-a commit");
    assert_eq!(merge_a.parent_count(), 2);
    let merge_b = commits
        .iter()
        .find(|commit| commit.subject == "merge-b")
        .expect("merge-b commit");
    assert_eq!(merge_b.parent_count(), 2);

    let mut state = lg::state::AppState::new();
    state.commits = commits;
    let backend = ratatui::backend::TestBackend::new(120, 20);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            lg::panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    // Lazygit-style merge marker followed directly by ─╮ (no padding).
    assert!(
        rendered.contains("\u{23e3}\u{2500}\u{256e}"),
        "rendered graph should include merge connector: {rendered}"
    );
    // Round corners only — no slash diagonals or backslashes.
    assert!(
        !rendered.contains('\\')
            && !rendered.contains('\u{2572}')
            && !rendered.contains('\u{2571}'),
        "rendered graph should use curved connector glyphs instead of slash diagonals: {rendered}"
    );
    assert!(
        rendered.contains("merge-a") && rendered.contains("a2"),
        "rendered graph should include merge and side branch commits: {rendered}"
    );
}

#[test]
fn list_commits_renders_repeated_main_merges_into_feature_branch() {
    let dir = init_repo();
    fs::write(dir.path().join("base.txt"), "base").unwrap();
    stage_in(dir.path(), "base.txt");
    commit_in(dir.path(), "base");

    git_ok(dir.path(), &["checkout", "-b", "feature"]);
    fs::write(dir.path().join("feature.txt"), "feature-1").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature-1");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main-1.txt"), "main-1").unwrap();
    stage_in(dir.path(), "main-1.txt");
    commit_in(dir.path(), "main-1");

    git_ok(dir.path(), &["checkout", "feature"]);
    git_ok(
        dir.path(),
        &["merge", "--no-ff", "main", "-m", "merge-main-1"],
    );
    fs::write(dir.path().join("feature.txt"), "feature-2").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature-2");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main-2.txt"), "main-2").unwrap();
    stage_in(dir.path(), "main-2.txt");
    commit_in(dir.path(), "main-2");

    git_ok(dir.path(), &["checkout", "feature"]);
    git_ok(
        dir.path(),
        &["merge", "--no-ff", "main", "-m", "merge-main-2"],
    );
    fs::write(dir.path().join("feature.txt"), "feature-3").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature-3");

    let _cwd = CwdGuard::new(dir.path());
    let commits = lg::git::list_commits_for_ref("feature", 30).unwrap();

    let merge_main = commits
        .iter()
        .find(|commit| commit.subject == "merge-main-1")
        .expect("merge-main-1 commit");
    assert_eq!(merge_main.parent_count(), 2);

    let mut state = lg::state::AppState::new();
    state.commits = commits;
    let backend = ratatui::backend::TestBackend::new(120, 20);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            lg::panel::commits::render(&state, frame.area(), frame, false);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains('\u{23e3}') && rendered.contains('\u{256e}'),
        "rendered main merges should include visible merge connectors: {rendered}"
    );
}

#[test]
fn branch_log_renders_decorated_graph_log() {
    let dir = init_repo();
    fs::write(dir.path().join("a.txt"), "one").unwrap();
    stage_in(dir.path(), "a.txt");
    commit_in(dir.path(), "initial commit");

    let _cwd = CwdGuard::new(dir.path());
    let log = lg::git::branch_log("main", 10).unwrap();

    assert!(
        log.contains("* commit "),
        "missing graph commit line: {log}"
    );
    assert!(log.contains("Author:"), "missing author line: {log}");
    assert!(log.contains("Date:"), "missing date line: {log}");
    assert!(log.contains("initial commit"), "missing message: {log}");
}

#[test]
fn commit_on_empty_message_fails() {
    // lg::git::commit guards against empty messages.
    let result = lg::git::commit("");
    assert!(result.is_err(), "expected Err for empty message");
}

#[test]
fn release_flow_returns_to_original_branch_after_target_push() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "origin", "develop"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "release/next"]);
    git_ok(dir.path(), &["push", "origin", "release/next"]);

    let feature = "feature/release-return";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "develop").expect("release to develop");
    assert_eq!(head_branch(dir.path()), feature);

    lg::git::flow_release_current(feature, "release/next").expect("release to release/next");
    assert_eq!(head_branch(dir.path()), feature);

    let develop_log = git(bare.path(), &["log", "--oneline", "develop"]);
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("feature commit"),
        "develop did not receive feature commit"
    );
    let release_log = git(bare.path(), &["log", "--oneline", "release/next"]);
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("feature commit"),
        "release/next did not receive feature commit"
    );
    let local_release = git(dir.path(), &["rev-parse", "release/next"]);
    let remote_release = git(bare.path(), &["rev-parse", "release/next"]);
    assert_eq!(
        String::from_utf8_lossy(&local_release.stdout),
        String::from_utf8_lossy(&remote_release.stdout),
        "origin/release/next was not pushed to the merged release/next HEAD"
    );
    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "release/next@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "release/next upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/release/next"
    );
}

#[test]
fn release_flow_stashes_dirty_work_for_target_checkouts() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    fs::write(dir.path().join("target_only.txt"), "develop\n").unwrap();
    stage_in(dir.path(), "target_only.txt");
    commit_in(dir.path(), "develop target file");
    git_ok(dir.path(), &["push", "origin", "develop"]);

    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "release/next"]);
    fs::write(dir.path().join("target_only.txt"), "release\n").unwrap();
    stage_in(dir.path(), "target_only.txt");
    commit_in(dir.path(), "release target file");
    git_ok(dir.path(), &["push", "origin", "release/next"]);

    let feature = "feature/release-dirty";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

    fs::write(dir.path().join("init.txt"), "dirty init").unwrap();
    fs::write(dir.path().join("target_only.txt"), "untracked local").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "develop").expect("release to develop");
    assert_eq!(head_branch(dir.path()), feature);
    assert_eq!(
        fs::read_to_string(dir.path().join("init.txt")).unwrap(),
        "dirty init"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("target_only.txt")).unwrap(),
        "untracked local"
    );

    lg::git::flow_release_current(feature, "release/next").expect("release to release/next");
    assert_eq!(head_branch(dir.path()), feature);
    assert_eq!(
        fs::read_to_string(dir.path().join("init.txt")).unwrap(),
        "dirty init"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("target_only.txt")).unwrap(),
        "untracked local"
    );

    let develop_log = git(bare.path(), &["log", "--oneline", "develop"]);
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("feature commit"),
        "develop did not receive feature commit"
    );
    let release_log = git(bare.path(), &["log", "--oneline", "release/next"]);
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("feature commit"),
        "release/next did not receive feature commit"
    );
    let stash_list = git(dir.path(), &["stash", "list"]);
    assert!(
        String::from_utf8_lossy(&stash_list.stdout).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

#[test]
fn release_conflict_continue_auto_stages_pushes_target_and_returns_to_feature() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "origin", "develop"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "release/next"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "release/next"]);

    let feature = "feature/release-conflict";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "release/next")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "release/next");

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution_with_followup(Some("release/next"), Some(feature))
        .expect("continue release conflict");

    assert_eq!(head_branch(dir.path()), feature);
    let released_file = git(bare.path(), &["show", "release/next:conflict.txt"]);
    assert!(
        released_file.status.success(),
        "release/next file missing: {}",
        String::from_utf8_lossy(&released_file.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&released_file.stdout), "resolved\n");
}

#[test]
fn release_conflict_validate_pushes_target_after_user_returns_to_feature() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "origin", "develop"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "release/next"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "release/next"]);

    let feature = "feature/release-conflict-manual";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "release/next")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "release/next");

    fs::write(dir.path().join("conflict.txt"), "manually resolved\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    git_ok(dir.path(), &["commit", "--no-edit"]);
    git_ok(dir.path(), &["checkout", feature]);

    lg::git::validate_conflict_resolution_with_followup(Some("release/next"), Some(feature))
        .expect("validate manually completed release conflict");

    assert_eq!(head_branch(dir.path()), feature);
    let released_file = git(bare.path(), &["show", "release/next:conflict.txt"]);
    assert!(
        released_file.status.success(),
        "release/next file missing: {}",
        String::from_utf8_lossy(&released_file.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&released_file.stdout),
        "manually resolved\n"
    );
}

#[test]
fn merge_main_flow_stashes_dirty_work_updates_main_and_returns_to_feature() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let feature = "feature/merge-main";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

    fs::write(dir.path().join("dirty.txt"), "dirty work").unwrap();

    let updater = tempfile::tempdir().expect("updater tempdir");
    git_ok(
        updater.path(),
        &["clone", bare.path().to_str().unwrap(), "."],
    );
    git_ok(
        updater.path(),
        &["config", "user.email", "test@example.com"],
    );
    git_ok(updater.path(), &["config", "user.name", "Test User"]);
    fs::write(updater.path().join("main.txt"), "main update").unwrap();
    stage_in(updater.path(), "main.txt");
    commit_in(updater.path(), "main update");
    git_ok(updater.path(), &["push", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature).expect("merge main into feature");

    assert_eq!(head_branch(dir.path()), feature);
    assert!(
        dir.path().join("dirty.txt").exists(),
        "dirty work should be restored"
    );

    let main_rev = git(dir.path(), &["rev-parse", "main"]);
    let origin_main_rev = git(dir.path(), &["rev-parse", "origin/main"]);
    assert_eq!(
        String::from_utf8_lossy(&main_rev.stdout),
        String::from_utf8_lossy(&origin_main_rev.stdout),
        "local main should be updated to origin/main"
    );

    let log = git(dir.path(), &["log", "--oneline", feature]);
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("main update"),
        "feature branch did not receive origin/main"
    );

    let stash_list = git(dir.path(), &["stash", "list"]);
    assert!(
        String::from_utf8_lossy(&stash_list.stdout).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

#[test]
fn branch_release_status_reports_missing_commits_after_release() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "origin", "develop"]);

    let feature = "feature/release-status";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "released").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "released feature commit");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "develop").expect("release to develop");

    fs::write(dir.path().join("followup.txt"), "not released").unwrap();
    stage_in(dir.path(), "followup.txt");
    commit_in(dir.path(), "unreleased followup");

    let status = lg::git::branch_release_status(feature).expect("branch release status");
    let develop = status.develop.expect("develop release status");
    assert!(!develop.released_at.is_empty(), "missing release timestamp");
    assert_eq!(develop.missing_commits, 1);
    assert!(status.test.is_none(), "release/next should not be marked");
}

// ── parse_porcelain unit tests (comprehensive) ─────────────────────────────

#[test]
fn parse_porcelain_modified_untracked_renamed_and_both() {
    // Build a synthetic -z byte string:
    //  " M modified.rs"  — worktree-only modified (unstaged)
    //  "?? untracked.txt" — untracked (unstaged)
    //  "R  renamed_new.rs" + "renamed_old.rs"  — staged rename
    //  "MM both.rs"       — staged AND unstaged modified
    let input: Vec<u8> = {
        let mut v = Vec::new();
        v.extend_from_slice(b" M modified.rs");
        v.push(0);
        v.extend_from_slice(b"?? untracked.txt");
        v.push(0);
        v.extend_from_slice(b"R  renamed_new.rs");
        v.push(0);
        v.extend_from_slice(b"renamed_old.rs");
        v.push(0);
        v.extend_from_slice(b"MM both.rs");
        v.push(0);
        v
    };

    let (unstaged, staged) = lg::git::parse_porcelain(&input);

    // Staged: renamed_new.rs (R) + both.rs (MM index side)
    assert!(
        staged.contains(&"renamed_new.rs".to_string()),
        "staged: {staged:?}"
    );
    assert!(
        staged.contains(&"both.rs".to_string()),
        "staged: {staged:?}"
    );
    assert!(
        !staged.contains(&"modified.rs".to_string()),
        "modified.rs should not be staged"
    );
    assert!(!staged.contains(&"untracked.txt".to_string()));

    // Unstaged: modified.rs + untracked.txt + both.rs (MM worktree side)
    assert!(
        unstaged.contains(&"modified.rs".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(
        unstaged.contains(&"untracked.txt".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(
        unstaged.contains(&"both.rs".to_string()),
        "unstaged: {unstaged:?}"
    );
    assert!(!unstaged.contains(&"renamed_new.rs".to_string()));
}
