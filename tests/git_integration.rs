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

/// The followup lg hands `v` for a release of `feature` into `target`. Tests go
/// through this so they exercise the same continuation the UI does, including
/// the feature merge a conflict may have stopped short of.
fn release_followup<'a>(feature: &'a str, target: &'a str) -> lg::git::Followup<'a> {
    lg::git::Followup {
        merge_branch: Some(feature),
        push_branch: Some(target),
        return_branch: Some(feature),
        safety_cleanup: None,
    }
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

#[test]
fn status_entries_expands_untracked_directories_to_files() {
    let dir = init_repo();
    git_ok(
        dir.path(),
        &["config", "status.showUntrackedFiles", "normal"],
    );
    fs::create_dir_all(dir.path().join("src/test/service")).unwrap();
    fs::write(
        dir.path()
            .join("src/test/service/ExchangeRateServiceTest.kt"),
        "test\n",
    )
    .unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let entries = lg::git::status_entries().unwrap();
    let paths = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();

    assert!(
        paths.contains(&"src/test/service/ExchangeRateServiceTest.kt"),
        "untracked directory contents should be listed: {paths:?}"
    );
    assert!(
        !paths.contains(&"src/test/service/"),
        "collapsed untracked directory row should be replaced: {paths:?}"
    );
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
fn delete_worktree_path_removes_file_or_folder() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::create_dir_all("src/nested").unwrap();
    fs::write("src/main.rs", "fn main() {}\n").unwrap();
    fs::write("src/nested/old.rs", "old\n").unwrap();

    lg::git::delete_worktree_path("src/main.rs", false).unwrap();
    assert!(!dir.path().join("src/main.rs").exists());

    lg::git::delete_worktree_path("src/nested", true).unwrap();
    assert!(!dir.path().join("src/nested").exists());
}

#[test]
fn delete_worktree_path_rejects_unsafe_paths() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    assert!(lg::git::delete_worktree_path("../outside.txt", false).is_err());
    assert!(lg::git::delete_worktree_path("", false).is_err());
}

#[test]
fn rollback_worktree_path_discards_folder_changes() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::create_dir_all("src").unwrap();
    fs::write("src/main.rs", "original\n").unwrap();
    git_ok(dir.path(), &["add", "src/main.rs"]);
    commit_in(dir.path(), "initial");

    fs::write("src/main.rs", "changed\n").unwrap();
    fs::write("src/added.rs", "added\n").unwrap();
    fs::write("src/untracked.rs", "untracked\n").unwrap();
    git_ok(dir.path(), &["add", "src/main.rs", "src/added.rs"]);

    lg::git::rollback_worktree_path("src").unwrap();

    assert_eq!(fs::read_to_string("src/main.rs").unwrap(), "original\n");
    assert!(!dir.path().join("src/added.rs").exists());
    assert!(!dir.path().join("src/untracked.rs").exists());
    let (unstaged, staged) = status_in(dir.path());
    assert!(unstaged.is_empty(), "unstaged: {unstaged:?}");
    assert!(staged.is_empty(), "staged: {staged:?}");
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

#[test]
fn nested_repositories_report_branch_detached_head_and_dirty_state() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    let api = dir.path().join("services/api");
    fs::create_dir_all(&api).unwrap();
    git_ok(&api, &["init", "-b", "main"]);
    git_ok(&api, &["config", "user.email", "test@example.com"]);
    git_ok(&api, &["config", "user.name", "Test User"]);
    fs::write(api.join("README.md"), "api\n").unwrap();
    git_ok(&api, &["add", "README.md"]);
    commit_in(&api, "initial api");
    git_ok(&api, &["checkout", "-b", "feature/api"]);
    fs::write(api.join("scratch.txt"), "dirty\n").unwrap();

    let core = dir.path().join("libs/core");
    fs::create_dir_all(&core).unwrap();
    git_ok(&core, &["init", "-b", "main"]);
    git_ok(&core, &["config", "user.email", "test@example.com"]);
    git_ok(&core, &["config", "user.name", "Test User"]);
    fs::write(core.join("README.md"), "core\n").unwrap();
    git_ok(&core, &["add", "README.md"]);
    commit_in(&core, "initial core");
    git_ok(&core, &["checkout", "--detach", "HEAD"]);

    let repos = lg::git::nested_repositories().unwrap();
    let api_status = repos
        .iter()
        .find(|repo| repo.path == "services/api")
        .expect("api repo");
    assert_eq!(api_status.branch.as_deref(), Some("feature/api"));
    assert!(api_status.detached_at.is_none());
    assert!(api_status.has_changes);

    let core_status = repos
        .iter()
        .find(|repo| repo.path == "libs/core")
        .expect("core repo");
    assert_eq!(core_status.branch, None);
    assert!(core_status.detached_at.is_some());
    assert!(!core_status.has_changes);
}

#[test]
fn file_diff_includes_untracked_file_contents() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::write("KNOWLEDGE.md", "# Notes\n\nnew detail\n").unwrap();

    let diff = lg::git::file_diff("KNOWLEDGE.md").unwrap();

    assert!(diff.contains("== worktree =="), "{diff}");
    assert!(diff.contains("diff --git"), "{diff}");
    assert!(diff.contains("b/KNOWLEDGE.md"), "{diff}");
    assert!(diff.contains("+# Notes"), "{diff}");
    assert!(diff.contains("+new detail"), "{diff}");
}

#[test]
fn all_diffs_includes_untracked_file_contents() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());
    fs::create_dir_all("docs").unwrap();
    fs::write("docs/README.md", "# Docs\n").unwrap();

    let diff = lg::git::all_diffs().unwrap();

    assert!(diff.contains("== worktree =="), "{diff}");
    assert!(diff.contains("b/docs/README.md"), "{diff}");
    assert!(diff.contains("+# Docs"), "{diff}");
}

#[test]
fn transfer_diff_to_feature_branch_recreates_branch_changes_on_main() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    fs::write("app.txt", "one\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/messy"]);
    fs::write("app.txt", "one\ntwo\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "first messy commit");
    fs::write("new.txt", "new file\n").unwrap();
    stage_in(dir.path(), "new.txt");
    commit_in(dir.path(), "second messy commit");

    let status =
        lg::git::flow_transfer_diff_to_feature_branch("feature/messy", "feature/clean").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/clean");
    assert!(
        status.contains("transferred feature/messy diff"),
        "unexpected status: {status}"
    );
    let log = git(
        dir.path(),
        &["log", "--oneline", "--decorate", "--max-count=3"],
    );
    let log_text = String::from_utf8_lossy(&log.stdout);
    assert!(
        !log_text.contains("messy commit"),
        "new branch should not copy source commits: {log_text}"
    );

    let cached = git(dir.path(), &["diff", "--cached", "--name-only"]);
    let cached_text = String::from_utf8_lossy(&cached.stdout);
    assert!(cached_text.contains("app.txt"), "{cached_text}");
    assert!(cached_text.contains("new.txt"), "{cached_text}");
    assert_eq!(fs::read_to_string("app.txt").unwrap(), "one\ntwo\n");
    assert_eq!(fs::read_to_string("new.txt").unwrap(), "new file\n");
}

#[test]
fn create_feature_branch_from_main_pushes_and_sets_upstream() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::flow_create_feature_branch("main", "feature/tracked-create").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/tracked-create");
    assert!(
        out.contains("tracking origin/feature/tracked-create"),
        "status should mention upstream: {out}"
    );
    let upstream = git(
        dir.path(),
        &[
            "rev-parse",
            "--abbrev-ref",
            "feature/tracked-create@{upstream}",
        ],
    );
    assert!(
        upstream.status.success(),
        "upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/feature/tracked-create"
    );
    let remote = git(
        bare.path(),
        &["rev-parse", "--verify", "refs/heads/feature/tracked-create"],
    );
    assert!(
        remote.status.success(),
        "remote feature branch was not created: {}",
        String::from_utf8_lossy(&remote.stderr)
    );
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

fn commit_in_at(dir: &std::path::Path, msg: &str, date: &str) {
    let out = Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_DATE", date)
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

fn stash_list(dir: &Path) -> String {
    let out = git(dir, &["stash", "list"]);
    assert!(
        out.status.success(),
        "git stash list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn branch_list(dir: &Path) -> String {
    let out = git(dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        out.status.success(),
        "git branch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
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
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let _cwd = CwdGuard::new(dir.path());
    assert_eq!(lg::git::head_branch().unwrap(), "main");
}

#[test]
fn worktrees_are_listed_from_any_checkout_with_their_dirty_state() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let linked = elsewhere.path().join("feat-x");
    git_ok(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feat/x",
            linked.to_str().expect("worktree path"),
        ],
    );
    fs::write(linked.join("work.txt"), "in progress").unwrap();

    let from_main = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(from_main.len(), 2, "{from_main:#?}");
    assert!(from_main[0].is_main);
    assert_eq!(from_main[0].branch.as_deref(), Some("main"));
    assert!(!from_main[0].has_changes, "main worktree should be clean");
    assert_eq!(from_main[1].branch.as_deref(), Some("feat/x"));
    assert!(
        from_main[1].has_changes,
        "the linked worktree has an untracked file"
    );

    // Asking a linked worktree returns the same set, main worktree first.
    let from_linked = lg::git::with_repo(&linked, lg::git::worktrees).expect("worktrees");
    let branches: Vec<_> = from_linked
        .iter()
        .map(|worktree| worktree.branch.as_deref())
        .collect();
    assert_eq!(branches, [Some("main"), Some("feat/x")]);
    assert!(from_linked[0].is_main);
}

#[test]
fn a_worktree_can_be_added_for_a_new_branch_and_removed_again() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("nested").join("feat-x");

    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "feat/x", "main")
    })
    .expect("add worktree");

    assert!(path.join("init.txt").is_file(), "worktree was checked out");
    assert_eq!(head_branch(&path), "feat/x", "on the new branch");

    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(listed.len(), 2, "{listed:#?}");

    lg::git::with_repo(repo.path(), || lg::git::worktree_remove(&path, false))
        .expect("remove worktree");
    assert!(!path.exists(), "the directory is gone");
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(listed.len(), 1, "{listed:#?}");
}

#[test]
fn adding_a_worktree_checks_out_a_branch_that_already_exists() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");
    git_ok(repo.path(), &["branch", "existing"]);

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("existing");

    // The base is ignored for a branch that already exists, so a nonsense one
    // must not stop it being checked out.
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "existing", "no/such/ref")
    })
    .expect("add worktree");

    assert_eq!(head_branch(&path), "existing");
}

#[test]
fn adding_a_worktree_reports_bad_input_instead_of_running_git() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("x");

    let empty = lg::git::with_repo(repo.path(), || lg::git::worktree_add(&path, "  ", "main"));
    assert!(
        empty.unwrap_err().to_string().contains("cannot be empty"),
        "an empty branch name is caught before git runs"
    );

    let invalid = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "bad..name", "main")
    });
    assert!(
        invalid.unwrap_err().to_string().contains("invalid branch"),
        "an invalid branch name is caught before git runs"
    );

    let unknown_base = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "fresh", "no/such/ref")
    });
    assert!(
        unknown_base
            .unwrap_err()
            .to_string()
            .contains("unknown base ref"),
        "an unknown base is named as such"
    );
    assert!(!path.exists(), "nothing was created");
}

#[test]
fn removing_a_dirty_worktree_needs_force() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("dirty");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "dirty", "main")
    })
    .expect("add worktree");
    fs::write(path.join("scratch.txt"), "work in progress").unwrap();

    let refused = lg::git::with_repo(repo.path(), || lg::git::worktree_remove(&path, false));
    assert!(refused.is_err(), "git should refuse to discard the work");
    assert!(path.exists(), "the worktree survives a refusal");

    lg::git::with_repo(repo.path(), || lg::git::worktree_remove(&path, true))
        .expect("force removal");
    assert!(!path.exists());
}

#[test]
fn pruning_forgets_a_worktree_whose_directory_is_gone() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = elsewhere.path().join("gone");
    lg::git::with_repo(repo.path(), || lg::git::worktree_add(&path, "gone", "main"))
        .expect("add worktree");
    fs::remove_dir_all(&path).expect("remove worktree directory");

    lg::git::with_repo(repo.path(), lg::git::worktree_prune).expect("prune");
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(
        listed.len(),
        1,
        "only the main worktree is left: {listed:#?}"
    );
}

#[test]
fn a_removed_worktree_directory_is_reported_as_prunable() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let linked = elsewhere.path().join("gone");
    git_ok(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "gone",
            linked.to_str().expect("worktree path"),
        ],
    );
    fs::remove_dir_all(&linked).expect("remove worktree directory");

    let worktrees = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    let missing = worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some("gone"))
        .expect("the removed worktree is still registered");
    assert!(
        missing.prunable.is_some(),
        "git should call it prunable: {missing:#?}"
    );
    assert!(missing.is_missing());
}

#[test]
fn with_repo_points_git_at_a_checkout_without_moving_the_process() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    git_ok(dir.path(), &["checkout", "-b", "pinned-branch"]);

    let before = std::env::current_dir().expect("current dir");
    let branch = lg::git::with_repo(dir.path(), lg::git::head_branch).expect("head branch");

    assert_eq!(branch, "pinned-branch");
    assert_eq!(
        std::env::current_dir().expect("current dir"),
        before,
        "pinning a checkout must not move the process"
    );
}

#[test]
fn a_pin_outranks_the_working_directory_and_is_released_after() {
    let outer = init_repo();
    fs::write(outer.path().join("init.txt"), "init").unwrap();
    stage_in(outer.path(), "init.txt");
    commit_in(outer.path(), "initial commit");

    let inner = init_repo();
    fs::write(inner.path().join("init.txt"), "init").unwrap();
    stage_in(inner.path(), "init.txt");
    commit_in(inner.path(), "initial commit");
    git_ok(inner.path(), &["checkout", "-b", "inner-branch"]);

    let _cwd = CwdGuard::new(outer.path());
    assert_eq!(lg::git::head_branch().unwrap(), "main");
    assert_eq!(
        lg::git::with_repo(inner.path(), lg::git::head_branch).unwrap(),
        "inner-branch"
    );
    assert_eq!(
        lg::git::head_branch().unwrap(),
        "main",
        "the pin must not outlive its scope"
    );
}

#[test]
fn a_spawned_job_runs_against_the_checkout_it_was_started_from() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    git_ok(dir.path(), &["checkout", "-b", "job-branch"]);

    let branch = lg::git::with_repo(dir.path(), || {
        lg::git::spawn_pinned(lg::git::head_branch)
            .join()
            .expect("join pinned job")
    })
    .expect("head branch");

    assert_eq!(branch, "job-branch");
}

#[test]
fn head_branch_returns_unborn_branch_before_first_commit() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    assert_eq!(lg::git::head_branch().unwrap(), "main");
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
    let production_pos = review
        .nodes
        .iter()
        .position(|node| node.id == "branch:category:production")
        .expect("production category node");
    assert_eq!(review.nodes[0].title, "Full diff against main");
    assert_eq!(
        review.nodes[production_pos].parent.as_deref(),
        Some("branch"),
        "production category should be directly under the full diff root"
    );
    assert_eq!(review.nodes[production_pos].depth, 1);
    assert_eq!(
        review.nodes[file_pos].parent.as_deref(),
        Some("branch:category:production"),
        "file should be nested under its review category"
    );
    assert_eq!(review.nodes[file_pos].depth, 2);
    assert_eq!(
        review.nodes[entry_pos].parent.as_deref(),
        Some(review.nodes[file_pos].id.as_str()),
        "entry point should be nested under its file"
    );
    assert_eq!(review.nodes[entry_pos].depth, 3);
    assert!(file_pos < 3, "file node should appear before metadata");
    assert!(
        review.nodes.iter().all(|node| node.id != "full-diff"),
        "interactive review should not have a flat full-diff lump"
    );
    assert!(
        review.nodes[entry_pos]
            .title
            .contains(":2 in fn greet - updates "),
        "entry title should include line and description: {}",
        review.nodes[entry_pos].title
    );
    assert!(
        review.nodes[entry_pos]
            .body
            .iter()
            .any(|line| line.contains("+    \"hello review\"")),
        "entry node should carry the patch for source view: {:?}",
        review.nodes[entry_pos].body
    );
    assert!(
        review
            .nodes
            .iter()
            .all(|node| !node.id.starts_with("branch:hunk:")),
        "entry point tree should not include leaf hunk nodes: {:?}",
        review.nodes
    );
}

#[test]
fn assisted_review_includes_worktree_and_untracked_changes() {
    let dir = init_repo();
    fs::create_dir_all(dir.path().join("src/main/kotlin")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hello\"\n}\n",
    )
    .unwrap();
    stage_in(dir.path(), "src/lib.rs");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "feature/worktree-review"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hello worktree\"\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/main/kotlin/NewFlow.kt"),
        "class NewFlow {\n    fun newFlow() = \"worktree\"\n}\n",
    )
    .unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let review = lg::git::build_assisted_review_against_main().unwrap();

    assert!(
        review
            .report
            .contains("including staged, unstaged, and untracked files"),
        "{}",
        review.report
    );
    assert!(
        review.report.contains("\"hello worktree\""),
        "{}",
        review.report
    );
    assert!(
        review.report.contains("src/main/kotlin/NewFlow.kt"),
        "{}",
        review.report
    );
    assert!(review.report.contains("fun newFlow"), "{}", review.report);
    assert!(
        review.nodes.iter().any(|node| {
            node.id.starts_with("branch:file:") && node.title.contains("src/main/kotlin/NewFlow.kt")
        }),
        "{:?}",
        review.nodes
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
    let hunk_header_count = entry_nodes[0]
        .body
        .iter()
        .filter(|line| line.starts_with("@@"))
        .count();
    assert_eq!(
        hunk_header_count, 2,
        "separate hunks should be carried by the shared entry point"
    );
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
    assert_eq!(
        file_nodes[0].parent.as_deref(),
        Some("branch:category:production")
    );
    assert_eq!(file_nodes[0].depth, 2);
    assert_eq!(review.nodes[next_step].depth, 3);
    assert_eq!(review.nodes[maybe_transfer].depth, 4);
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
fn assisted_review_includes_uncommitted_local_changes() {
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
        review.report.contains("local only") && review.report.contains("scratch.txt"),
        "local changes should be included: {}",
        review.report
    );
    assert!(
        review
            .nodes
            .iter()
            .any(|node| node.title.contains("tracked.txt")),
        "tracked local node should exist: {:?}",
        review.nodes
    );
    assert!(
        review
            .nodes
            .iter()
            .any(|node| node.title.contains("scratch.txt")),
        "untracked local node should exist: {:?}",
        review.nodes
    );
    assert!(
        review
            .report
            .contains("including staged, unstaged, and untracked files"),
        "{}",
        review.report
    );
}

#[test]
fn list_commits_on_empty_repo_is_empty() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    let commits = lg::git::list_commits(10).unwrap();
    let current_branch_commits = lg::git::list_commits_for_ref("main", 10).unwrap();

    assert!(commits.is_empty());
    assert!(current_branch_commits.is_empty());
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
fn commit_on_empty_repo_creates_initial_commit() {
    let dir = init_repo();
    fs::write(dir.path().join("first.txt"), "first").unwrap();
    stage_in(dir.path(), "first.txt");
    let _cwd = CwdGuard::new(dir.path());

    lg::git::commit("initial commit").unwrap();

    let commits = lg::git::list_commits(10).unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].subject, "initial commit");
}

#[test]
fn commit_on_release_branch_updates_from_origin_main_first() {
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

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    git_ok(dir.path(), &["push", "-u", "origin", "develop"]);

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

    fs::write(dir.path().join("direct.txt"), "direct").unwrap();
    stage_in(dir.path(), "direct.txt");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::commit("direct release commit").expect("commit on stale release branch");

    let main_is_ancestor = git(
        dir.path(),
        &["merge-base", "--is-ancestor", "origin/main", "HEAD"],
    );
    assert!(
        main_is_ancestor.status.success(),
        "release branch should include origin/main before the direct commit"
    );
    let log = git(dir.path(), &["log", "--oneline", "develop"]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(log.contains("main update"), "missing main update: {log}");
    assert!(
        log.contains("direct release commit"),
        "missing direct commit: {log}"
    );
}

#[test]
fn pull_merges_upstream_when_current_branch_diverged() {
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

    fs::write(dir.path().join("local.txt"), "local").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in(dir.path(), "local commit");

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
    fs::write(updater.path().join("remote.txt"), "remote").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote commit");
    git_ok(updater.path(), &["push", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::pull("origin", "main").expect("pull diverged branch");

    assert!(
        out.contains("Merge") || out.contains("merge"),
        "expected merge output for diverged pull: {out}"
    );
    let remote_is_ancestor = git(
        dir.path(),
        &["merge-base", "--is-ancestor", "origin/main", "HEAD"],
    );
    assert!(
        remote_is_ancestor.status.success(),
        "pull should merge origin/main into the local branch"
    );
    let log = git(dir.path(), &["log", "--oneline", "main"]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(log.contains("local commit"), "missing local commit: {log}");
    assert!(
        log.contains("remote commit"),
        "missing remote commit: {log}"
    );
}

#[test]
fn pull_stashes_dirty_work_before_updating_main() {
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
    fs::write(updater.path().join("remote.txt"), "remote\n").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote commit");
    git_ok(updater.path(), &["push", "origin", "main"]);

    fs::write(dir.path().join("dirty.txt"), "dirty work\n").unwrap();
    stage_in(dir.path(), "dirty.txt");

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::pull("origin", "main").expect("pull with dirty work");

    assert!(
        out.contains("applied stashed local changes after pull"),
        "pull output should mention stash: {out}"
    );
    assert!(
        dir.path().join("remote.txt").exists(),
        "remote update should be pulled"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("dirty.txt")).unwrap(),
        "dirty work\n"
    );
    assert!(
        status_in(dir.path()).1.contains(&"dirty.txt".to_string()),
        "dirty file should still be staged"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

#[test]
fn list_branches_orders_newest_commit_first() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");

    git_ok(dir.path(), &["checkout", "-b", "older"]);
    fs::write(dir.path().join("older.txt"), "older\n").unwrap();
    stage_in(dir.path(), "older.txt");
    commit_in_at(dir.path(), "older branch", "2026-01-01T00:00:00Z");

    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "newer"]);
    fs::write(dir.path().join("newer.txt"), "newer\n").unwrap();
    stage_in(dir.path(), "newer.txt");
    commit_in_at(dir.path(), "newer branch", "2026-01-02T00:00:00Z");

    let _cwd = CwdGuard::new(dir.path());
    let branches = lg::git::list_branches().unwrap();
    let names = branches
        .iter()
        .map(|branch| branch.name.as_str())
        .collect::<Vec<_>>();
    let newer = names.iter().position(|name| *name == "newer").unwrap();
    let older = names.iter().position(|name| *name == "older").unwrap();
    assert!(
        newer < older,
        "newer branch should sort before older branch: {names:?}"
    );
}

#[test]
fn list_branches_reports_commits_behind_main() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");

    git_ok(dir.path(), &["checkout", "-b", "feature/stale"]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("main.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "main.txt");
    commit_in(dir.path(), "main update");

    let _cwd = CwdGuard::new(dir.path());
    let branches = lg::git::list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/stale")
        .expect("feature/stale branch should be listed");
    let main = branches
        .iter()
        .find(|branch| branch.name == "main")
        .expect("main branch should be listed");

    assert_eq!(feature.behind_main, 1);
    assert_eq!(main.behind_main, 0);
}

#[test]
fn checkout_branch_stashes_unstaged_changes_before_switching() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(dir.path(), &["checkout", "-b", "feature/target"]);
    fs::write(dir.path().join("target.txt"), "target\n").unwrap();
    stage_in(dir.path(), "target.txt");
    commit_in(dir.path(), "target branch");
    git_ok(dir.path(), &["checkout", "main"]);

    fs::write(dir.path().join("README.md"), "dirty\n").unwrap();
    let _cwd = CwdGuard::new(dir.path());

    let out = lg::git::checkout_branch("feature/target").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/target");
    assert!(
        out.contains("applied stashed local changes after checkout"),
        "checkout output should mention stash: {out}"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.contains(&"README.md".to_string()) && staged.is_empty(),
        "unstaged change should be applied on target branch"
    );
    assert!(
        !stash_list(dir.path()).contains("lg: auto-stash before checkout"),
        "stash should be popped after checkout"
    );
}

#[test]
fn checkout_branch_stashes_staged_changes_before_switching() {
    let dir = init_repo();
    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(dir.path(), &["checkout", "-b", "feature/target"]);
    fs::write(dir.path().join("target.txt"), "target\n").unwrap();
    stage_in(dir.path(), "target.txt");
    commit_in(dir.path(), "target branch");
    git_ok(dir.path(), &["checkout", "main"]);

    fs::write(dir.path().join("staged.txt"), "staged\n").unwrap();
    stage_in(dir.path(), "staged.txt");
    let _cwd = CwdGuard::new(dir.path());

    let out = lg::git::checkout_branch("feature/target").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/target");
    assert!(
        out.contains("applied stashed local changes after checkout"),
        "checkout output should mention stash: {out}"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.is_empty() && staged.contains(&"staged.txt".to_string()),
        "staged change should be applied on target branch"
    );
    assert!(
        !stash_list(dir.path()).contains("lg: auto-stash before checkout"),
        "stash should be popped after checkout"
    );
}

#[test]
fn remote_branches_can_be_listed_and_checked_out_locally() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "feature/remote"]);
    fs::write(dir.path().join("remote.txt"), "remote\n").unwrap();
    stage_in(dir.path(), "remote.txt");
    commit_in(dir.path(), "remote branch");
    git_ok(dir.path(), &["push", "-u", "origin", "feature/remote"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["branch", "-D", "feature/remote"]);

    let _cwd = CwdGuard::new(dir.path());
    let remotes = lg::git::list_remote_branches().unwrap();
    let remote = remotes
        .iter()
        .find(|branch| branch.name == "origin/feature/remote")
        .expect("origin/feature/remote should be listed");
    assert_eq!(remote.remote, "origin");
    assert_eq!(remote.local_name, "feature/remote");
    assert!(remote.last_commit_unix.is_some());

    lg::git::checkout_remote_branch("origin/feature/remote").unwrap();
    assert_eq!(head_branch(dir.path()), "feature/remote");

    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "feature/remote@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/feature/remote"
    );
}

#[test]
fn set_branch_upstream_tracks_existing_remote_branch() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "solution"]);
    fs::write(dir.path().join("solution.txt"), "solution\n").unwrap();
    stage_in(dir.path(), "solution.txt");
    commit_in(dir.path(), "solution branch");
    git_ok(dir.path(), &["push", "origin", "solution"]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::set_branch_upstream("solution", "origin/solution").unwrap();

    assert_eq!(out, "solution tracks origin/solution");
    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "solution@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/solution"
    );
}

#[test]
fn checkout_remote_branch_applies_dirty_changes_after_switching() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "feature/remote"]);
    fs::write(dir.path().join("remote.txt"), "remote\n").unwrap();
    stage_in(dir.path(), "remote.txt");
    commit_in(dir.path(), "remote branch");
    git_ok(dir.path(), &["push", "-u", "origin", "feature/remote"]);
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["branch", "-D", "feature/remote"]);

    fs::write(dir.path().join("README.md"), "dirty\n").unwrap();
    let _cwd = CwdGuard::new(dir.path());

    let out = lg::git::checkout_remote_branch("origin/feature/remote").unwrap();

    assert_eq!(head_branch(dir.path()), "feature/remote");
    assert!(
        out.contains("applied stashed local changes after checkout"),
        "checkout output should mention stash: {out}"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(
        unstaged.contains(&"README.md".to_string()) && staged.is_empty(),
        "dirty change should be applied on remote tracking branch"
    );
    assert!(
        !stash_list(dir.path()).contains("lg: auto-stash before remote checkout"),
        "stash should be popped after remote checkout"
    );
}

#[test]
fn list_remote_branches_orders_newest_commit_first() {
    let dir = init_repo();
    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);

    fs::write(dir.path().join("README.md"), "main\n").unwrap();
    stage_in(dir.path(), "README.md");
    commit_in(dir.path(), "initial");
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "older"]);
    fs::write(dir.path().join("older.txt"), "older\n").unwrap();
    stage_in(dir.path(), "older.txt");
    commit_in_at(dir.path(), "older remote branch", "2026-01-01T00:00:00Z");
    git_ok(dir.path(), &["push", "-u", "origin", "older"]);

    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "newer"]);
    fs::write(dir.path().join("newer.txt"), "newer\n").unwrap();
    stage_in(dir.path(), "newer.txt");
    commit_in_at(dir.path(), "newer remote branch", "2026-01-02T00:00:00Z");
    git_ok(dir.path(), &["push", "-u", "origin", "newer"]);

    let _cwd = CwdGuard::new(dir.path());
    let branches = lg::git::list_remote_branches().unwrap();
    let names = branches
        .iter()
        .map(|branch| branch.name.as_str())
        .collect::<Vec<_>>();
    let newer = names
        .iter()
        .position(|name| *name == "origin/newer")
        .unwrap();
    let older = names
        .iter()
        .position(|name| *name == "origin/older")
        .unwrap();
    assert!(
        newer < older,
        "newer remote branch should sort before older remote branch: {names:?}"
    );
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-return";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");

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
    lg::git::flow_release_current(feature, "develop").expect("release to develop");
    assert_eq!(head_branch(dir.path()), feature);

    lg::git::flow_release_current(feature, "test").expect("release to test");
    assert_eq!(head_branch(dir.path()), feature);

    let develop_log = git(bare.path(), &["log", "--oneline", "develop"]);
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("feature commit"),
        "develop did not receive feature commit"
    );
    assert!(
        String::from_utf8_lossy(&develop_log.stdout).contains("main update"),
        "develop did not receive origin/main before release"
    );
    let release_log = git(bare.path(), &["log", "--oneline", "test"]);
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("feature commit"),
        "test did not receive feature commit"
    );
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("main update"),
        "test did not receive origin/main before release"
    );
    let local_release = git(dir.path(), &["rev-parse", "test"]);
    let remote_release = git(bare.path(), &["rev-parse", "test"]);
    assert_eq!(
        String::from_utf8_lossy(&local_release.stdout),
        String::from_utf8_lossy(&remote_release.stdout),
        "origin/test was not pushed to the merged test HEAD"
    );
    let upstream = git(
        dir.path(),
        &["rev-parse", "--abbrev-ref", "test@{upstream}"],
    );
    assert!(
        upstream.status.success(),
        "test upstream was not configured: {}",
        String::from_utf8_lossy(&upstream.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/test"
    );
    let branches = git(dir.path(), &["branch", "--format=%(refname:short)"]);
    let branch_text = String::from_utf8_lossy(&branches.stdout);
    assert!(
        !branch_text
            .lines()
            .any(|branch| branch.starts_with("lg/backup/")),
        "successful release should clean up safety branches: {branch_text}"
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("target_only.txt"), "release\n").unwrap();
    stage_in(dir.path(), "target_only.txt");
    commit_in(dir.path(), "release target file");
    git_ok(dir.path(), &["push", "origin", "test"]);

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

    lg::git::flow_release_current(feature, "test").expect("release to test");
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
    let release_log = git(bare.path(), &["log", "--oneline", "test"]);
    assert!(
        String::from_utf8_lossy(&release_log.stdout).contains("feature commit"),
        "test did not receive feature commit"
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-conflict";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "test");

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue release conflict");

    assert_eq!(head_branch(dir.path()), feature);
    let released_file = git(bare.path(), &["show", "test:conflict.txt"]);
    assert!(
        released_file.status.success(),
        "test file missing: {}",
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-conflict-manual";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "test");

    fs::write(dir.path().join("conflict.txt"), "manually resolved\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    git_ok(dir.path(), &["commit", "--no-edit"]);
    git_ok(dir.path(), &["checkout", feature]);

    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("validate manually completed release conflict");

    assert_eq!(head_branch(dir.path()), feature);
    let released_file = git(bare.path(), &["show", "test:conflict.txt"]);
    assert!(
        released_file.status.success(),
        "test file missing: {}",
        String::from_utf8_lossy(&released_file.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&released_file.stdout),
        "manually resolved\n"
    );
}

#[test]
fn release_conflict_validate_merges_advanced_remote_target_before_push() {
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("conflict.txt"), "release\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/release-conflict-remote-advanced";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("release should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), "test");

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
    git_ok(updater.path(), &["checkout", "test"]);
    fs::write(updater.path().join("remote.txt"), "remote update\n").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote target update");
    git_ok(updater.path(), &["push", "origin", "test"]);

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue release conflict after target advanced");

    assert!(
        out.contains("origin/test advanced"),
        "validation should explain the remote-target retry: {out}"
    );
    assert_eq!(head_branch(dir.path()), feature);
    let released_conflict = git(bare.path(), &["show", "test:conflict.txt"]);
    assert!(
        released_conflict.status.success(),
        "test conflict file missing: {}",
        String::from_utf8_lossy(&released_conflict.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&released_conflict.stdout),
        "resolved\n"
    );
    let remote_file = git(bare.path(), &["show", "test:remote.txt"]);
    assert!(
        remote_file.status.success(),
        "test remote update missing: {}",
        String::from_utf8_lossy(&remote_file.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&remote_file.stdout),
        "remote update\n"
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
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/merge-main-feature-merge-main-"),
        "successful merge-main should clean safety backup: {branches}"
    );
}

#[test]
fn merge_main_flow_pulls_current_branch_before_merging_main() {
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

    let feature = "feature/pull-current-before-merge-main";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

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

    git_ok(updater.path(), &["checkout", feature]);
    fs::write(updater.path().join("remote-feature.txt"), "remote feature").unwrap();
    stage_in(updater.path(), "remote-feature.txt");
    commit_in(updater.path(), "remote feature update");
    git_ok(updater.path(), &["push", "origin", feature]);

    git_ok(updater.path(), &["checkout", "main"]);
    fs::write(updater.path().join("main.txt"), "main update").unwrap();
    stage_in(updater.path(), "main.txt");
    commit_in(updater.path(), "main update");
    git_ok(updater.path(), &["push", "origin", "main"]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature).expect("merge main into feature");

    assert_eq!(head_branch(dir.path()), feature);
    let log = git(dir.path(), &["log", "--oneline", feature]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(
        log.contains("remote feature update"),
        "feature branch did not pull the remote feature update: {log}"
    );
    assert!(
        log.contains("main update"),
        "feature branch did not receive origin/main: {log}"
    );

    let remote_log = git(bare.path(), &["log", "--oneline", feature]);
    let remote_log = String::from_utf8_lossy(&remote_log.stdout);
    assert!(
        remote_log.contains("main update"),
        "remote feature branch was not pushed after merge-main: {remote_log}"
    );
}

#[test]
fn merge_main_all_branches_merges_and_pushes_tracked_branches() {
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

    let tracked = "feature/tracked";
    git_ok(dir.path(), &["checkout", "-b", tracked]);
    fs::write(dir.path().join("tracked.txt"), "tracked").unwrap();
    stage_in(dir.path(), "tracked.txt");
    commit_in(dir.path(), "tracked commit");
    git_ok(dir.path(), &["push", "-u", "origin", tracked]);

    let local_only = "feature/local-only";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", local_only]);
    fs::write(dir.path().join("local.txt"), "local").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in(dir.path(), "local commit");
    git_ok(dir.path(), &["checkout", tracked]);
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
    let out = lg::git::flow_merge_main_into_all_local_branches().expect("sync branches");

    assert_eq!(head_branch(dir.path()), tracked);
    assert!(
        dir.path().join("dirty.txt").exists(),
        "dirty work should be restored on original branch"
    );
    assert!(
        out.contains("merged origin/main into 2 branches, pushed 1, skipped push 1"),
        "unexpected summary: {out}"
    );

    let tracked_log = git(bare.path(), &["log", "--oneline", tracked]);
    assert!(
        String::from_utf8_lossy(&tracked_log.stdout).contains("main update"),
        "tracked branch was not pushed with main update"
    );
    let local_log = git(dir.path(), &["log", "--oneline", local_only]);
    assert!(
        String::from_utf8_lossy(&local_log.stdout).contains("main update"),
        "local-only branch did not receive main update"
    );
    let remote_local = git(bare.path(), &["rev-parse", "--verify", local_only]);
    assert!(
        !remote_local.status.success(),
        "local-only branch should not be pushed"
    );
    let stash_list = git(dir.path(), &["stash", "list"]);
    assert!(
        String::from_utf8_lossy(&stash_list.stdout).is_empty(),
        "auto-stash should be restored and dropped"
    );
}

#[test]
fn merge_main_all_branches_merges_remote_updates_before_pushing() {
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

    let rejected = "feature/rejected-push";
    git_ok(dir.path(), &["checkout", "-b", rejected]);
    fs::write(dir.path().join("rejected.txt"), "local").unwrap();
    stage_in(dir.path(), "rejected.txt");
    commit_in_at(dir.path(), "rejected local commit", "2026-01-03T00:00:00Z");
    git_ok(dir.path(), &["push", "-u", "origin", rejected]);

    let local_only = "feature/local-after-reject";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", local_only]);
    fs::write(dir.path().join("local.txt"), "local").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in_at(dir.path(), "local-only commit", "2026-01-02T00:00:00Z");
    git_ok(dir.path(), &["checkout", rejected]);

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
    git_ok(updater.path(), &["checkout", rejected]);
    fs::write(updater.path().join("remote.txt"), "remote-only").unwrap();
    stage_in(updater.path(), "remote.txt");
    commit_in(updater.path(), "remote-only rejected update");
    git_ok(updater.path(), &["push", "origin", rejected]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::flow_merge_main_into_all_local_branches().expect("sync branches");

    assert_eq!(head_branch(dir.path()), rejected);
    assert!(
        out.contains("merged origin/main into 2 branches, pushed 1, skipped push 1"),
        "unexpected summary: {out}"
    );

    let rejected_log = git(dir.path(), &["log", "--oneline", rejected]);
    assert!(
        String::from_utf8_lossy(&rejected_log.stdout).contains("main update"),
        "rejected-push branch did not receive main update locally"
    );
    let local_log = git(dir.path(), &["log", "--oneline", local_only]);
    assert!(
        String::from_utf8_lossy(&local_log.stdout).contains("main update"),
        "local-only branch after rejected push did not receive main update"
    );
    let remote_rejected_log = git(bare.path(), &["log", "--oneline", rejected]);
    assert!(
        String::from_utf8_lossy(&remote_rejected_log.stdout).contains("main update"),
        "remote branch should receive merged main update after upstream sync"
    );
}

#[test]
fn merge_main_all_branches_reports_git_conflict_output() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let feature = "feature/sync-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["checkout", feature]);

    let _cwd = CwdGuard::new(dir.path());
    let err = lg::git::flow_merge_main_into_all_local_branches()
        .expect_err("sync branches should stop on conflict")
        .to_string();

    assert!(
        err.contains("merge main into feature/sync-conflict failed"),
        "missing branch context: {err}"
    );
    assert!(
        err.contains("CONFLICT") && err.contains("conflict.txt"),
        "missing git conflict output: {err}"
    );
    assert!(
        err.contains("Automatic merge failed"),
        "missing git merge failure message: {err}"
    );
}

#[test]
fn delete_current_feature_branch_checks_out_main_first() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    let branch = "feature/delete-current";
    git_ok(dir.path(), &["checkout", "-b", branch]);

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::delete_local_branch(branch, false).expect("delete current branch");

    assert_eq!(head_branch(dir.path()), "main");
    assert!(
        out.contains("checked out main"),
        "delete output should mention checkout: {out}"
    );
    let deleted = git(dir.path(), &["rev-parse", "--verify", branch]);
    assert!(!deleted.status.success(), "branch should be deleted");
}

#[test]
fn merge_main_flow_allows_release_branches_when_main_is_ahead() {
    for target in ["develop", "test"] {
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

        git_ok(dir.path(), &["checkout", "-b", target]);
        fs::write(dir.path().join("target.txt"), target).unwrap();
        stage_in(dir.path(), "target.txt");
        commit_in(dir.path(), "target commit");
        git_ok(dir.path(), &["push", "-u", "origin", target]);

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
        lg::git::flow_merge_main_into_current(target).expect("merge main into release branch");

        assert_eq!(head_branch(dir.path()), target);
        let log = git(dir.path(), &["log", "--oneline", target]);
        assert!(
            String::from_utf8_lossy(&log.stdout).contains("main update"),
            "{target} did not receive origin/main"
        );
    }
}

#[test]
fn reset_flow_cleans_safety_backup_after_success() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init\n").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "develop"]);
    fs::write(dir.path().join("develop.txt"), "develop\n").unwrap();
    stage_in(dir.path(), "develop.txt");
    commit_in(dir.path(), "develop commit");
    git_ok(dir.path(), &["push", "-u", "origin", "develop"]);

    let feature = "feature/reset-return";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_reset_branch_from_main(feature, "develop").expect("reset develop");

    assert_eq!(head_branch(dir.path()), feature);
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/reset-develop-develop-"),
        "successful reset should clean safety backup: {branches}"
    );
}

#[test]
fn discard_checkout_flow_resets_current_branch_from_remote_and_cleans_worktree() {
    let dir = init_repo();
    fs::write(dir.path().join("app.txt"), "main\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let feature = "feature/reload-from-remote";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("app.txt"), "feature base\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "feature base");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

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
    git_ok(updater.path(), &["checkout", feature]);
    fs::write(updater.path().join("app.txt"), "server head\n").unwrap();
    stage_in(updater.path(), "app.txt");
    commit_in(updater.path(), "server update");
    git_ok(updater.path(), &["push", "origin", feature]);

    fs::write(dir.path().join("app.txt"), "local only\n").unwrap();
    stage_in(dir.path(), "app.txt");
    commit_in(dir.path(), "local only");
    fs::write(dir.path().join("app.txt"), "dirty staged\n").unwrap();
    stage_in(dir.path(), "app.txt");
    fs::write(dir.path().join("scratch.txt"), "untracked\n").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let out = lg::git::flow_discard_checkout_from_remote(feature).expect("discard checkout");

    assert_eq!(head_branch(dir.path()), feature);
    assert!(
        out.contains("reset to origin/feature/reload-from-remote"),
        "status should mention remote ref: {out}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "server head\n"
    );
    assert!(
        !dir.path().join("scratch.txt").exists(),
        "untracked file should be removed"
    );
    let (unstaged, staged) = status_in(dir.path());
    assert!(unstaged.is_empty(), "unstaged: {unstaged:?}");
    assert!(staged.is_empty(), "staged: {staged:?}");

    let log = git(dir.path(), &["log", "--oneline", "--max-count=5"]);
    let log_text = String::from_utf8_lossy(&log.stdout);
    assert!(log_text.contains("server update"), "{log_text}");
    assert!(
        !log_text.contains("local only"),
        "local-only commit should not remain on branch: {log_text}"
    );
}

#[test]
fn merge_main_conflict_validation_cleans_safety_backup() {
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
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let feature = "feature/merge-main-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["push", "origin", "main"]);
    git_ok(dir.path(), &["checkout", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature)
        .expect_err("merge-main should stop for manual conflict resolution");
    assert_eq!(head_branch(dir.path()), feature);
    assert!(
        branch_list(dir.path()).contains("lg/backup/merge-main-feature-merge-main-conflict-"),
        "merge-main conflict should leave a safety backup before validation"
    );

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::validate_conflict_resolution(lg::git::Followup {
        push_branch: Some(feature),
        return_branch: Some(feature),
        safety_cleanup: Some(("merge-main", feature)),
        ..Default::default()
    })
    .expect("continue merge-main conflict");

    assert!(
        out.contains("removed lg/backup/merge-main-feature-merge-main-conflict-"),
        "validation should report backup cleanup: {out}"
    );
    assert_eq!(head_branch(dir.path()), feature);
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/merge-main-feature-merge-main-conflict-"),
        "validation should clean merge-main safety backup: {branches}"
    );
}

#[test]
fn merge_main_conflict_abort_cleans_safety_backup() {
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
    git_ok(dir.path(), &["push", "-u", "origin", "main"]);

    let feature = "feature/merge-main-abort";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");
    git_ok(dir.path(), &["push", "-u", "origin", feature]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["push", "origin", "main"]);
    git_ok(dir.path(), &["checkout", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_merge_main_into_current(feature)
        .expect_err("merge-main should stop for manual conflict resolution");
    assert!(
        branch_list(dir.path()).contains("lg/backup/merge-main-feature-merge-main-abort-"),
        "merge-main conflict should leave a safety backup before abort"
    );

    let out = lg::git::abort_in_progress_operation_with_cleanup(
        Some(feature),
        Some(("merge-main", feature)),
    )
    .expect("abort merge-main conflict");
    assert!(
        out.contains("removed lg/backup/merge-main-feature-merge-main-abort-"),
        "abort should report backup cleanup: {out}"
    );
    assert_eq!(head_branch(dir.path()), feature);
    let branches = branch_list(dir.path());
    assert!(
        !branches.contains("lg/backup/merge-main-feature-merge-main-abort-"),
        "abort should clean merge-main safety backup: {branches}"
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
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    git_ok(dir.path(), &["push", "origin", "test"]);

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
    let main = status.main.expect("main release status");
    assert!(
        main.released_at.is_empty(),
        "main should not have a release timestamp"
    );
    assert_eq!(main.missing_commits, 2);
    let develop = status.develop.expect("develop release status");
    assert!(!develop.released_at.is_empty(), "missing release timestamp");
    assert_eq!(develop.missing_commits, 1);
    let test = status.test.expect("test release status");
    assert!(
        test.released_at.is_empty(),
        "test should not have a release timestamp"
    );
    assert_eq!(test.missing_commits, 2);
}

#[test]
fn release_branches_detects_each_deploy_branch_on_its_own() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");

    let _cwd = CwdGuard::new(dir.path());
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), None);
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Test), None);
    assert!(!targets.any(), "a repo with only main deploys nothing");

    // alv.no shape: a test branch and no develop branch at all.
    git_ok(dir.path(), &["branch", "test"]);
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), None);
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Test), Some("test"));
    assert!(targets.any(), "a test branch alone is a deploy target");

    // Either spelling of the dev branch counts.
    git_ok(dir.path(), &["branch", "dev"]);
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), Some("dev"));

    // develop wins when a checkout has both spellings.
    git_ok(dir.path(), &["branch", "develop"]);
    let targets = lg::git::release_branches();
    assert_eq!(targets.branch(lg::git::ReleaseEnv::Dev), Some("develop"));
}

#[test]
fn release_status_reports_test_only_repository() {
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
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    git_ok(dir.path(), &["push", "origin", "test"]);

    let feature = "feature/test-only";
    git_ok(dir.path(), &["checkout", "main"]);
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "released").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "released feature commit");

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test").expect("release to test");

    let status = lg::git::branch_release_status(feature).expect("branch release status");
    assert!(
        status.develop.is_none(),
        "a repo without a develop branch has no develop status"
    );
    let test = status.test.expect("test release status");
    assert_eq!(test.missing_commits, 0);
    assert!(!test.released_at.is_empty(), "missing release timestamp");
    let main = status.main.expect("main release status");
    assert_eq!(main.missing_commits, 1);
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

/// Repository with one commit, a bare `origin` it tracks, and a linked
/// worktree on `feat/x` carrying one commit of its own — the shape both
/// handover flows start from.
fn repo_with_landable_worktree() -> (TempDir, TempDir, TempDir, std::path::PathBuf) {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init\n").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        repo.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(repo.path(), &["push", "-u", "origin", "main"]);

    let elsewhere = tempfile::tempdir().expect("worktree tempdir");
    let path = elsewhere.path().join("feat-x");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "feat/x", "main")
    })
    .expect("add worktree");
    fs::write(path.join("feature.txt"), "work\n").unwrap();
    stage_in(&path, "feature.txt");
    commit_in(&path, "add feature");

    (repo, bare, elsewhere, path)
}

#[test]
fn landing_a_worktree_merges_it_into_main_and_clears_the_branch_away() {
    let (repo, bare, _elsewhere, path) = repo_with_landable_worktree();
    git_ok(&path, &["push", "-u", "origin", "feat/x"]);

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect("land worktree");

    assert!(
        repo.path().join("feature.txt").is_file(),
        "main carries the work: {report}"
    );
    assert_eq!(head_branch(repo.path()), "main", "main stayed checked out");
    assert!(!path.exists(), "the worktree directory is gone: {report}");
    assert!(
        !branch_list(repo.path()).contains("feat/x"),
        "the local branch is gone: {report}"
    );

    let pushed = git(bare.path(), &["rev-parse", "refs/heads/main"]);
    let local = git(repo.path(), &["rev-parse", "refs/heads/main"]);
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        String::from_utf8_lossy(&local.stdout).trim(),
        "main was pushed: {report}"
    );
    assert!(
        !git(bare.path(), &["rev-parse", "--verify", "refs/heads/feat/x"])
            .status
            .success(),
        "the remote branch is gone: {report}"
    );
}

#[test]
fn landing_a_worktree_without_a_remote_still_merges_and_cleans_up() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init\n").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("worktree tempdir");
    let path = elsewhere.path().join("feat-x");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&path, "feat/x", "main")
    })
    .expect("add worktree");
    fs::write(path.join("feature.txt"), "work\n").unwrap();
    stage_in(&path, "feature.txt");
    commit_in(&path, "add feature");

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect("land worktree");

    assert!(repo.path().join("feature.txt").is_file(), "{report}");
    assert!(!path.exists(), "{report}");
    assert!(!branch_list(repo.path()).contains("feat/x"), "{report}");
}

#[test]
fn a_conflicting_land_is_aborted_and_leaves_main_where_it_was() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("shared.txt"), "from the branch\n").unwrap();
    stage_in(&path, "shared.txt");
    commit_in(&path, "branch writes shared");
    fs::write(repo.path().join("shared.txt"), "from main\n").unwrap();
    stage_in(repo.path(), "shared.txt");
    commit_in(repo.path(), "main writes shared");
    let before = git(repo.path(), &["rev-parse", "HEAD"]);

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect_err("the merge conflicts");

    assert!(
        err.to_string().contains("by hand"),
        "the error says what to do next: {err}"
    );
    let after = git(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&before.stdout).trim(),
        String::from_utf8_lossy(&after.stdout).trim(),
        "main is back where it started"
    );
    assert!(
        !repo.path().join(".git/MERGE_HEAD").exists(),
        "the merge was aborted, not left open"
    );
    assert!(path.exists(), "the worktree is untouched");
    assert!(branch_list(repo.path()).contains("feat/x"));
}

/// The case landing is stuck on: main moved on, so the merge into main is no
/// longer a fast-forward. Syncing first moves the merge into the worktree,
/// after which landing goes through.
#[test]
fn syncing_a_worktree_behind_main_lets_it_land() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(repo.path().join("moved-on.txt"), "main moved on\n").unwrap();
    stage_in(repo.path(), "moved-on.txt");
    commit_in(repo.path(), "main moves on");
    git_ok(repo.path(), &["push", "origin", "main"]);

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect("sync worktree");

    assert!(
        path.join("moved-on.txt").is_file(),
        "the branch carries main's work now: {report}"
    );
    assert_eq!(
        head_branch(repo.path()),
        "main",
        "main stayed checked out where it was"
    );

    let landed = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect("land after syncing");
    assert!(repo.path().join("feature.txt").is_file(), "{landed}");
    assert!(!path.exists(), "{landed}");
}

/// A sync conflict is the branch's to resolve, so it is left open rather than
/// aborted — that is what lets the conflict modal drive it to a finish.
#[test]
fn a_conflicting_sync_stays_open_in_the_worktree_and_leaves_main_alone() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("shared.txt"), "from the branch\n").unwrap();
    stage_in(&path, "shared.txt");
    commit_in(&path, "branch writes shared");
    fs::write(repo.path().join("shared.txt"), "from main\n").unwrap();
    stage_in(repo.path(), "shared.txt");
    commit_in(repo.path(), "main writes shared");
    let main_before = git(repo.path(), &["rev-parse", "HEAD"]);

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect_err("the merge conflicts");
    assert!(
        err.to_string().contains("shared.txt"),
        "the error names the conflicted file: {err}"
    );

    let conflicts = lg::git::with_repo(&path, lg::git::conflicted_files).expect("conflicted files");
    assert_eq!(
        conflicts,
        vec!["shared.txt".to_string()],
        "the conflict is left open in the worktree for the modal to pick up"
    );

    let main_after = git(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&main_before.stdout).trim(),
        String::from_utf8_lossy(&main_after.stdout).trim(),
        "main never moved"
    );
    assert!(
        !repo.path().join(".git/MERGE_HEAD").exists(),
        "the main checkout has no merge in progress"
    );
}

#[test]
fn syncing_a_worktree_that_is_already_current_changes_nothing() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    let before = git(&path, &["rev-parse", "HEAD"]);

    let report = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect("sync worktree");

    assert!(
        report.contains("already up to date"),
        "it says there was nothing to do: {report}"
    );
    let after = git(&path, &["rev-parse", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&before.stdout).trim(),
        String::from_utf8_lossy(&after.stdout).trim(),
        "the branch did not move"
    );
}

#[test]
fn syncing_refuses_while_the_worktree_has_uncommitted_work() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(repo.path().join("moved-on.txt"), "main moved on\n").unwrap();
    stage_in(repo.path(), "moved-on.txt");
    commit_in(repo.path(), "main moves on");
    fs::write(path.join("scratch.txt"), "unsaved\n").unwrap();
    stage_in(&path, "scratch.txt");

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_sync_main(&path, "feat/x"))
        .expect_err("the worktree is dirty");

    assert!(
        err.to_string().contains("commit or discard"),
        "the error says what to do first: {err}"
    );
    assert!(
        !path.join("moved-on.txt").exists(),
        "nothing was merged into a dirty worktree"
    );
}

#[test]
fn a_handover_refuses_while_the_worktree_has_uncommitted_work() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("scratch.txt"), "unsaved\n").unwrap();

    for outcome in [
        lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x")),
        lg::git::with_repo(repo.path(), || {
            lg::git::worktree_bring_home(&path, "feat/x")
        }),
    ] {
        let err = outcome.expect_err("uncommitted work stops the handover");
        assert!(
            err.to_string().contains("commit or discard"),
            "the error names the way out: {err}"
        );
    }
    assert!(path.exists(), "the worktree is untouched");
    assert!(!repo.path().join("feature.txt").is_file(), "nothing merged");
}

#[test]
fn a_handover_refuses_a_branch_the_worktree_has_since_left() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    git_ok(&path, &["checkout", "-b", "feat/y"]);

    let err = lg::git::with_repo(repo.path(), || lg::git::worktree_land(&path, "feat/x"))
        .expect_err("the confirmed branch is no longer there");

    assert!(
        err.to_string().contains("on feat/y now, not feat/x"),
        "the error names both branches: {err}"
    );
    assert!(path.exists(), "the worktree is untouched");
}

#[test]
fn bringing_a_branch_home_checks_it_out_in_the_main_worktree() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();

    let report = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_bring_home(&path, "feat/x")
    })
    .expect("bring the branch home");

    assert!(!path.exists(), "the worktree directory is gone: {report}");
    assert_eq!(
        head_branch(repo.path()),
        "feat/x",
        "the branch moved to the main checkout: {report}"
    );
    assert!(
        repo.path().join("feature.txt").is_file(),
        "its work came along: {report}"
    );
    assert!(
        branch_list(repo.path()).contains("feat/x"),
        "the branch is kept, not merged away: {report}"
    );
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    assert_eq!(
        listed.len(),
        1,
        "only the main checkout is left: {listed:#?}"
    );
}

#[test]
fn bringing_a_branch_home_refuses_while_the_main_checkout_is_dirty() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(repo.path().join("init.txt"), "edited\n").unwrap();

    let err = lg::git::with_repo(repo.path(), || {
        lg::git::worktree_bring_home(&path, "feat/x")
    })
    .expect_err("a dirty main checkout has nowhere to put the branch");

    assert!(
        err.to_string().contains("commit or stash"),
        "the error names the way out: {err}"
    );
    assert!(path.exists(), "the worktree is untouched");
}

#[test]
fn a_worktree_reports_how_far_its_branch_has_run_ahead_of_main() {
    let (repo, _bare, _elsewhere, path) = repo_with_landable_worktree();
    fs::write(path.join("second.txt"), "more\n").unwrap();
    stage_in(&path, "second.txt");
    commit_in(&path, "second commit");

    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    let main = listed.iter().find(|w| w.is_main).expect("main worktree");
    let feature = listed
        .iter()
        .find(|w| w.branch.as_deref() == Some("feat/x"))
        .expect("feature worktree");

    assert_eq!(feature.unmerged, Some(2), "two commits main does not have");
    assert_eq!(main.unmerged, None, "main is not ahead of itself");

    // Landing empties it, and the row should say so before the cleanup runs.
    git_ok(repo.path(), &["merge", "--no-edit", "feat/x"]);
    let listed = lg::git::with_repo(repo.path(), lg::git::worktrees).expect("worktrees");
    let feature = listed
        .iter()
        .find(|w| w.branch.as_deref() == Some("feat/x"))
        .expect("feature worktree");
    assert_eq!(feature.unmerged, Some(0), "merged, so only cleanup is left");
}

/// Merging main from a linked worktree used to run `git checkout main` there,
/// which git refuses because the main checkout still holds it. The flow died
/// after its auto-stash, so the worktree went empty with the work in a stash
/// nobody mentioned.
#[test]
fn merge_main_flow_merges_from_a_worktree_without_checking_main_out() {
    let repo = init_repo();
    fs::write(repo.path().join("init.txt"), "init").unwrap();
    stage_in(repo.path(), "init.txt");
    commit_in(repo.path(), "initial commit");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let linked = elsewhere.path().join("feat-x");
    git_ok(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/x",
            linked.to_str().expect("worktree path"),
        ],
    );

    fs::write(repo.path().join("main.txt"), "main update").unwrap();
    stage_in(repo.path(), "main.txt");
    commit_in(repo.path(), "main update");

    fs::write(linked.join("dirty.txt"), "dirty work").unwrap();

    let summary = lg::git::with_repo(&linked, || {
        lg::git::flow_merge_main_into_current("feature/x")
    })
    .expect("merge main into the worktree's branch");

    let log = git(&linked, &["log", "--oneline", "feature/x"]);
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("main update"),
        "the worktree's branch did not receive main: {summary}"
    );
    assert!(
        linked.join("dirty.txt").exists(),
        "the stashed work should be back: {summary}"
    );
    assert!(
        stash_list(&linked).is_empty(),
        "auto-stash should be restored and dropped: {}",
        stash_list(&linked)
    );
    assert_eq!(head_branch(&linked), "feature/x");
    assert_eq!(
        head_branch(repo.path()),
        "main",
        "the main checkout should be left where it was"
    );
    assert!(
        summary.contains("kept feature/x local"),
        "a branch without an upstream is not published by a merge: {summary}"
    );
    assert!(
        !branch_list(repo.path()).contains("lg/backup/merge-main-"),
        "a successful merge-main cleans its safety backup: {}",
        branch_list(repo.path())
    );
}

/// The branch list counts how far behind the local `main` a branch is, so that
/// is the `main` a merge has to deliver — merging only `origin/main` reported
/// success while leaving the branch exactly as behind as it was.
#[test]
fn merge_main_flow_merges_commits_only_the_local_main_has() {
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

    let feature = "feature/local-main";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("local.txt"), "not pushed").unwrap();
    stage_in(dir.path(), "local.txt");
    commit_in(dir.path(), "local main commit");
    git_ok(dir.path(), &["checkout", feature]);

    let summary = lg::git::with_repo(dir.path(), || {
        lg::git::flow_merge_main_into_current(feature)
    })
    .expect("merge main into feature");

    let log = git(dir.path(), &["log", "--oneline", feature]);
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("local main commit"),
        "the unpushed main commit should have been merged: {summary}"
    );
}

/// A flow that fails after auto-stashing has to put the work back. Leaving it
/// stashed empties the checkout with nothing on screen saying why.
#[test]
fn merge_main_flow_restores_the_auto_stash_when_it_fails() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    git_ok(dir.path(), &["checkout", "-b", "feature/no-main"]);
    git_ok(dir.path(), &["branch", "-D", "main"]);

    fs::write(dir.path().join("dirty.txt"), "dirty work").unwrap();

    let err = lg::git::with_repo(dir.path(), || {
        lg::git::flow_merge_main_into_current("feature/no-main")
    })
    .expect_err("a repository without main cannot merge it");

    let message = err.to_string();
    assert!(
        message.contains("could not find main"),
        "the failure should say what was missing: {message}"
    );
    assert!(
        message.contains("restored your uncommitted changes"),
        "the failure should say the work came back: {message}"
    );
    assert!(
        dir.path().join("dirty.txt").exists(),
        "the stashed work should be back"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "nothing should be left stashed: {}",
        stash_list(dir.path())
    );
}

/// A conflicted merge cannot take the stash back, so the error says where the
/// work is and validating the resolution restores it.
#[test]
fn merge_main_conflict_keeps_the_stash_until_the_conflict_is_validated() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    let feature = "feature/stashed-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("conflict.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "feature side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "main\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "main side");
    git_ok(dir.path(), &["checkout", feature]);

    fs::write(dir.path().join("dirty.txt"), "dirty work").unwrap();

    let _cwd = CwdGuard::new(dir.path());
    let err = lg::git::flow_merge_main_into_current(feature)
        .expect_err("merge-main should stop for manual conflict resolution");
    let message = err.to_string();
    assert!(
        message.contains("git stash pop"),
        "the error should say how to get the work back: {message}"
    );
    assert!(
        !dir.path().join("dirty.txt").exists(),
        "the work stays stashed while the merge is conflicted"
    );

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::validate_conflict_resolution(lg::git::Followup {
        return_branch: Some(feature),
        safety_cleanup: Some(("merge-main", feature)),
        ..Default::default()
    })
    .expect("continue merge-main conflict");

    assert!(
        out.contains("restored"),
        "validation should report the stash coming back: {out}"
    );
    assert!(
        dir.path().join("dirty.txt").exists(),
        "the stashed work should be back after validation: {out}"
    );
    assert!(
        stash_list(dir.path()).is_empty(),
        "nothing should be left stashed: {}",
        stash_list(dir.path())
    );
}

/// A git process that died mid-write leaves `index.lock` behind, and from then
/// on everything that writes the index fails while `git status` keeps working
/// — so lg shows the files and refuses to stage them. Git's four-line refusal
/// puts the part worth acting on last, which a one-line status bar cuts off.
#[test]
fn staging_against_a_leftover_index_lock_says_what_to_do_about_it() {
    let dir = init_repo();
    fs::write(dir.path().join("init.txt"), "init").unwrap();
    stage_in(dir.path(), "init.txt");
    commit_in(dir.path(), "initial commit");
    fs::write(dir.path().join("work.txt"), "work").unwrap();
    fs::write(dir.path().join(".git/index.lock"), "").unwrap();

    let err = lg::git::with_repo(dir.path(), || lg::git::stage("work.txt"))
        .expect_err("git cannot take the index lock twice");
    let first_line = err
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();

    assert!(
        first_line.contains("index.lock"),
        "the first line should name the lock: {first_line}"
    );
    assert!(
        first_line.contains("delete it if none is"),
        "the first line should say what to do: {first_line}"
    );
}

/// A release can conflict at two different merges, and the one that goes wrong
/// first is `origin/main` into the deploy branch — before the feature has been
/// merged at all. Continuing must finish the release, not just push whatever
/// the deploy branch happens to hold.
#[test]
fn release_conflict_on_the_main_merge_still_releases_the_feature() {
    let dir = init_repo();
    fs::write(dir.path().join("shared.txt"), "base\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    // test diverges from main on shared.txt...
    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    // ...and then main moves on the same file, so merging it into test breaks.
    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    // The feature touches a different file, so it is not part of the conflict.
    let feature = "feature/release-main-conflict";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test")
        .expect_err("the main merge should stop for manual resolution");
    assert_eq!(head_branch(dir.path()), "test");

    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release");

    assert_eq!(head_branch(dir.path()), feature);
    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(
        released.status.success(),
        "the feature never reached test: {}",
        String::from_utf8_lossy(&released.stderr)
    );
}

/// git reports conflicted paths relative to the repository, and lg never
/// changes the process working directory — so a file still full of markers must
/// not read as resolved just because it cannot be found next to the process.
#[test]
fn a_conflicted_file_is_read_from_the_repository_not_the_process_directory() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "theirs"]);
    fs::write(dir.path().join("conflict.txt"), "theirs\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "their side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "ours\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "our side");

    let merge = git(dir.path(), &["merge", "theirs"]);
    assert!(!merge.status.success(), "the merge should have conflicted");

    // The process sits somewhere else entirely, as lg's does.
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    let _cwd = CwdGuard::new(elsewhere.path());

    let staged = lg::git::with_repo(dir.path(), lg::git::stage_resolved_conflicts)
        .expect("stage resolved conflicts");
    assert!(
        staged.is_empty(),
        "a file that still holds markers must not be staged: {staged:?}"
    );
    let still_conflicted =
        lg::git::with_repo(dir.path(), lg::git::conflicted_files).expect("conflicted files");
    assert_eq!(still_conflicted, ["conflict.txt"]);

    // Resolved for real, it stages from the same distance.
    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let staged = lg::git::with_repo(dir.path(), lg::git::stage_resolved_conflicts)
        .expect("stage resolved conflicts");
    assert_eq!(staged, ["conflict.txt"]);
}

/// The whole round trip the loop was stuck in: the release conflicts, the
/// conflict is resolved, `v` continues it — and running the release again
/// finds nothing left to do. A second conflict here means the first one was
/// never really finished.
#[test]
fn a_resolved_release_conflict_does_not_come_back_on_the_next_release() {
    let dir = init_repo();
    fs::write(dir.path().join("shared.txt"), "base\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    let feature = "feature/loop";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());

    // First run stops on the origin/main merge.
    lg::git::flow_release_current(feature, "test").expect_err("the main merge conflicts");
    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release");
    assert_eq!(head_branch(dir.path()), feature);

    // Second run has nothing left to conflict over.
    let summary = lg::git::flow_release_current(feature, "test")
        .expect("the release should be clean the second time");
    assert!(
        summary.contains("released"),
        "unexpected summary: {summary}"
    );
    assert!(
        lg::git::conflicted_files()
            .expect("conflicted files")
            .is_empty(),
        "the resolved conflict came back"
    );

    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(
        released.status.success(),
        "the feature never reached test: {}",
        String::from_utf8_lossy(&released.stderr)
    );
    let shared = git(bare.path(), &["show", "test:shared.txt"]);
    assert_eq!(
        String::from_utf8_lossy(&shared.stdout),
        "resolved\n",
        "the resolution should be what test carries"
    );
}

/// The loop itself: a release run on top of an unfinished conflict used to
/// reset the deploy branch to its remote, throwing the resolution away and
/// walking into the same conflict again. It has to refuse instead, and say how
/// to get out.
#[test]
fn a_release_refuses_to_run_on_top_of_an_unfinished_conflict() {
    let dir = init_repo();
    fs::write(dir.path().join("shared.txt"), "base\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    let feature = "feature/refuse";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test").expect_err("the main merge conflicts");

    // Resolved, but not yet continued — the state the loop happened in.
    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    let before = git(dir.path(), &["rev-parse", "test"]);

    let err = lg::git::flow_release_current(feature, "test")
        .expect_err("a second release must not run over the resolution");
    let message = format!("{err:#}");
    assert!(
        message.contains("part-way through a merge"),
        "the refusal should say what is in the way: {message}"
    );
    assert!(
        message.contains("F") && message.contains("v") && message.contains("a"),
        "and which keys get out of it: {message}"
    );

    assert_eq!(
        String::from_utf8_lossy(&before.stdout),
        String::from_utf8_lossy(&git(dir.path(), &["rev-parse", "test"]).stdout),
        "the deploy branch must not have been reset under the resolution"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("shared.txt")).unwrap(),
        "resolved\n",
        "the resolution must survive the refusal"
    );

    // And continuing still works from there.
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release after the refusal");
    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(released.status.success(), "the feature should have landed");
}

/// The other half of the loop: the conflict was resolved *and committed* — by a
/// claude session, or by hand — so nothing is in progress any more, but the
/// merge commit is only local. Releasing again would reset the deploy branch to
/// its remote, drop that commit, and hit the same conflict.
#[test]
fn a_release_refuses_to_reset_a_deploy_branch_that_is_ahead_of_its_remote() {
    let dir = init_repo();
    fs::write(dir.path().join("shared.txt"), "base\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "initial commit");

    let bare = tempfile::tempdir().expect("bare tempdir");
    git_ok(bare.path(), &["init", "--bare", "-b", "main"]);
    git_ok(
        dir.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git_ok(dir.path(), &["push", "origin", "main"]);

    git_ok(dir.path(), &["checkout", "-b", "test"]);
    fs::write(dir.path().join("shared.txt"), "release\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "release side");
    git_ok(dir.path(), &["push", "origin", "test"]);

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("shared.txt"), "main update\n").unwrap();
    stage_in(dir.path(), "shared.txt");
    commit_in(dir.path(), "main update");
    git_ok(dir.path(), &["push", "origin", "main"]);

    let feature = "feature/committed";
    git_ok(dir.path(), &["checkout", "-b", feature]);
    fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
    stage_in(dir.path(), "feature.txt");
    commit_in(dir.path(), "feature commit");
    git_ok(dir.path(), &["push", "origin", feature]);

    let _cwd = CwdGuard::new(dir.path());
    lg::git::flow_release_current(feature, "test").expect_err("the main merge conflicts");

    // Resolved and committed on test, the way a session would leave it.
    fs::write(dir.path().join("shared.txt"), "resolved\n").unwrap();
    git_ok(dir.path(), &["add", "shared.txt"]);
    commit_in(dir.path(), "resolve the main merge");
    assert!(
        lg::git::conflicted_files()
            .expect("conflicted files")
            .is_empty(),
        "nothing should be in progress any more"
    );
    let resolved = git(dir.path(), &["rev-parse", "test"]);

    git_ok(dir.path(), &["checkout", feature]);
    let err = lg::git::flow_release_current(feature, "test")
        .expect_err("a release must not reset test over the resolution");
    let message = format!("{err:#}");
    assert!(
        message.contains("would lose them") && message.contains("release again"),
        "the refusal should say what is at stake and what to do: {message}"
    );

    assert_eq!(
        String::from_utf8_lossy(&resolved.stdout),
        String::from_utf8_lossy(&git(dir.path(), &["rev-parse", "test"]).stdout),
        "the committed resolution must still be there"
    );

    // Continuing from there finishes the release it belongs to.
    lg::git::validate_conflict_resolution(release_followup(feature, "test"))
        .expect("continue the release");
    let released = git(bare.path(), &["show", "test:feature.txt"]);
    assert!(released.status.success(), "the feature should have landed");
    let shared = git(bare.path(), &["show", "test:shared.txt"]);
    assert_eq!(String::from_utf8_lossy(&shared.stdout), "resolved\n");
}

/// The bug that made the release loop: git reports its in-progress markers
/// relative to git's own directory, and lg points git at a repository with `-C`
/// instead of moving the process. Tested against the process directory they all
/// read as absent, so a conflicted merge looked finished — validation skipped
/// the commit that completes it and the push moved nothing.
#[test]
fn a_merge_in_progress_is_detected_from_outside_the_repository() {
    let dir = init_repo();
    fs::write(dir.path().join("conflict.txt"), "base\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "initial commit");

    git_ok(dir.path(), &["checkout", "-b", "theirs"]);
    fs::write(dir.path().join("conflict.txt"), "theirs\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "their side");

    git_ok(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("conflict.txt"), "ours\n").unwrap();
    stage_in(dir.path(), "conflict.txt");
    commit_in(dir.path(), "our side");

    let merge = git(dir.path(), &["merge", "theirs"]);
    assert!(!merge.status.success(), "the merge should have conflicted");

    // The process sits somewhere else entirely, as lg's does.
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    let _cwd = CwdGuard::new(elsewhere.path());

    fs::write(dir.path().join("conflict.txt"), "resolved\n").unwrap();
    let out = lg::git::with_repo(dir.path(), || {
        lg::git::validate_conflict_resolution(lg::git::Followup::default())
    })
    .expect("validate the resolution");

    assert!(
        !out.contains("no merge, rebase, or cherry-pick operation is in progress"),
        "the merge in progress should have been found: {out}"
    );
    let parents = git(dir.path(), &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&parents.stdout)
            .split_whitespace()
            .count(),
        3,
        "validation must commit the merge, leaving a commit with two parents"
    );
    assert!(
        lg::git::with_repo(dir.path(), lg::git::conflicted_files)
            .expect("conflicted files")
            .is_empty(),
        "nothing should still be conflicted"
    );
}
