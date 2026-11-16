//! The deploy branches a checkout implies stay in force under any file that
//! sets only some branch fields. One test in its binary, because it points the
//! process's configuration at temporary directories.
use std::path::{Path, PathBuf};
use std::process::Command;

use lg::panel::settings::{self, Category};
use lg::preferences::Scope;
use lg::state::AppState;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

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

fn repository(dir: &Path, branches: &[&str]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["commit", "--allow-empty", "-m", "Initial"]);
    for branch in branches {
        git(dir, &["branch", branch]);
    }
    dir.to_path_buf()
}

fn fresh_home(temp: &Path, name: &str) {
    let home = temp.join(name);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: the only test in the binary sets these between its own steps.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("LG_PREFERENCES_DIR", home.join("config"));
        std::env::set_var("LG_SETTINGS_DIR", home.join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", home.join("model"));
    }
    lg::preferences::invalidate();
}

fn work_in(repo: &Path) {
    lg::git::set_active_repo(repo);
    lg::preferences::invalidate();
}

fn environment_branches() -> Vec<String> {
    lg::preferences::load()
        .config
        .branches
        .environments
        .into_iter()
        .map(|e| e.branch)
        .collect()
}

#[test]
fn a_file_setting_some_branch_fields_keeps_the_rest_detected() {
    let temp = tempfile::tempdir().unwrap();
    // The claude CLI is asked for its models when the hub opens; keep it to
    // git so nothing outside the test runs.
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let git_path = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    std::os::unix::fs::symlink(git_path.trim(), bin.join("git")).unwrap();
    // SAFETY: the only test in the binary, before anything else runs.
    unsafe { std::env::set_var("PATH", &bin) };

    let develops = repository(&temp.path().join("develops"), &["develop"]);
    let tests = repository(&temp.path().join("tests"), &["test"]);

    // The remote saved for every repository from the hub in one of them
    // leaves each repository its own deploy branches.
    fresh_home(temp.path(), "home-hub");
    work_in(&develops);
    let mut state = AppState::default();
    settings::open(&mut state, Category::Branches);
    state.settings_hub.scope = Scope::User;
    state.settings_hub.draft["remote"] = json!("upstream");
    settings::handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(
        state.settings_hub.notice.starts_with("Saved"),
        "{}",
        state.settings_hub.notice
    );
    work_in(&tests);
    assert_eq!(lg::preferences::remote(), "upstream");
    assert!(environment_branches().contains(&"test".to_string()));
    assert!(!environment_branches().contains(&"develop".to_string()));
    assert!(
        lg::preferences::load()
            .config
            .branches
            .protected
            .contains(&"test".to_string())
    );
    assert!(lg::preferences::configured_category("branches"));
    work_in(&develops);
    assert!(environment_branches().contains(&"develop".to_string()));

    // The same through `lg config set`.
    fresh_home(temp.path(), "home-cli");
    work_in(&develops);
    lg::preferences_cli::run(&[
        "set".into(),
        "branches.remote".into(),
        "\"upstream\"".into(),
        "--scope".into(),
        "user".into(),
    ])
    .unwrap();
    work_in(&tests);
    assert_eq!(lg::preferences::remote(), "upstream");
    assert!(environment_branches().contains(&"test".to_string()));

    // A file naming its own environments, and no promotions, is still read:
    // the promotions detected for the checkout's environments do not apply.
    fresh_home(temp.path(), "home-own-environments");
    work_in(&tests);
    lg::preferences::save_category(
        Scope::User,
        "branches",
        json!({"environments": [{"id": "prod", "name": "Production", "branch": "main"}]}),
    )
    .unwrap();
    lg::preferences::invalidate();
    let loaded = lg::preferences::load();
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    assert_eq!(environment_branches(), vec!["main".to_string()]);
}
