//! Which checkout git runs against: the workspace, nested repositories and pins.

use super::common::*;

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
fn a_linked_worktree_in_the_workspace_is_listed_under_its_repository() {
    let dir = init_repo();
    let _cwd = CwdGuard::new(dir.path());

    // Sorted by path alone, `app-tools` would sit between `app` and its
    // worktree; the worktree must still come right after `app`.
    for name in ["app", "app-tools"] {
        let repo = dir.path().join(name);
        fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-b", "main"]);
        git_ok(&repo, &["config", "user.email", "test@example.com"]);
        git_ok(&repo, &["config", "user.name", "Test User"]);
        fs::write(repo.join("README.md"), format!("{name}\n")).unwrap();
        git_ok(&repo, &["add", "README.md"]);
        commit_in(&repo, "initial");
    }
    let app = dir.path().join("app");
    let worktree = dir.path().join("app.worktrees/feat");
    git_ok(
        &app,
        &["worktree", "add", "-b", "feat", worktree.to_str().unwrap()],
    );

    let repos = lg::git::nested_repositories().unwrap();
    let paths = repos
        .iter()
        .map(|repo| repo.path.as_str())
        .collect::<Vec<_>>();
    let app_at = paths.iter().position(|p| *p == "app").expect("app row");
    assert_eq!(
        paths.get(app_at + 1).copied(),
        Some("app.worktrees/feat"),
        "the worktree follows its repository: {paths:?}"
    );

    let linked = repos
        .iter()
        .find(|repo| repo.path == "app.worktrees/feat")
        .expect("worktree row");
    assert_eq!(linked.worktree_of.as_deref(), Some("app"));
    assert_eq!(linked.branch.as_deref(), Some("feat"));
    for plain in ["app", "app-tools"] {
        let repo = repos.iter().find(|repo| repo.path == plain).unwrap();
        assert_eq!(repo.worktree_of, None, "{plain} is a repository of its own");
    }
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
