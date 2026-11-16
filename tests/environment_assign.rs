//! `a` in the environments modal assigns the branch selected in the branch
//! list to the selected environment, and wires it the way choosing it in the
//! settings field does. The only test in its binary, because it points the
//! process's configuration at a temporary HOME.
use std::path::Path;
use std::process::Command;

use lg::git::RemoteBranch;
use lg::state::{AppState, BranchView, Modal};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_assigns_the_remote_branch_on_screen_and_protects_it() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("project");
    let bin = temp.path().join("bin");
    for dir in [&home, &repo, &bin] {
        std::fs::create_dir_all(dir).unwrap();
    }
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["commit", "--allow-empty", "-m", "Initial"]);
    git(&repo, &["branch", "develop"]);
    git(&repo, &["branch", "test"]);
    let git_path = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    std::os::unix::fs::symlink(git_path.trim(), bin.join("git")).unwrap();
    // SAFETY: this is the only test in the binary, and nothing else runs yet.
    unsafe {
        // The claude CLI is asked for its models when settings open; keep it
        // to git so nothing outside the test runs.
        std::env::set_var("PATH", &bin);
        std::env::set_var("HOME", &home);
        std::env::set_var("LG_PREFERENCES_DIR", home.join("config"));
        std::env::set_var("LG_SETTINGS_DIR", home.join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", home.join("model"));
    }
    lg::git::set_active_repo(&repo);

    let environments = lg::preferences::load().config.branches.environments;
    let test_env = environments
        .iter()
        .position(|e| e.id == "test")
        .expect("test is detected as an environment");

    let mut state = AppState::default();
    state.branch = Some("main".into());
    state.branch_view = BranchView::Remote;
    state.remote_branches = vec![RemoteBranch {
        name: "upstream/staging".into(),
        remote: "upstream".into(),
        local_name: "staging".into(),
        last_commit_unix: None,
    }];
    lg::panel::deployment::open(&mut state);
    state.environment_view.selected = test_env;

    lg::panel::deployment::handle_key(&mut state, KeyEvent::from(KeyCode::Char('a'))).unwrap();
    assert_eq!(state.modal, Modal::Settings);
    lg::panel::settings::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    lg::preferences::invalidate();

    let branches = lg::preferences::load().config.branches;
    let assigned = branches
        .environments
        .iter()
        .find(|e| e.id == "test")
        .unwrap();
    assert_eq!(assigned.branch, "staging");
    assert_eq!(assigned.remote, "upstream");
    assert!(
        branches.protected.contains(&"staging".to_string()),
        "{:?}",
        branches.protected
    );
}
