//! A branch an environment deploys from is never treated as a feature branch,
//! even when the configuration does not also list it as protected. The only
//! test in its binary, because it points the process's configuration at a
//! temporary HOME.
use std::path::Path;
use std::process::Command;

use lg::preferences::Scope;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
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
fn cleaning_orphans_never_offers_a_deploy_branch() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("project");
    for dir in [&home, &repo] {
        std::fs::create_dir_all(dir).unwrap();
    }
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["commit", "--allow-empty", "-m", "Initial"]);
    git(&repo, &["branch", "staging"]);
    git(&repo, &["branch", "feature/old"]);
    // SAFETY: this is the only test in the binary, and nothing else runs yet.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("LG_PREFERENCES_DIR", home.join("config"));
        std::env::set_var("LG_SETTINGS_DIR", home.join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", home.join("model"));
    }
    lg::git::set_active_repo(&repo);
    lg::preferences::save_category(
        Scope::Repository,
        "branches",
        serde_json::json!({
            "base": "main",
            "protected": [],
            "environments": [
                {"id": "staging", "name": "Staging", "remote": "origin", "branch": "staging"}
            ]
        }),
    )
    .unwrap();

    let orphans: Vec<String> = lg::git::orphan_branches("main")
        .unwrap()
        .into_iter()
        .map(|orphan| orphan.name)
        .collect();

    assert!(
        orphans.contains(&"feature/old".to_owned()),
        "a feature branch with no upstream is an orphan: {orphans:?}"
    );
    assert!(
        !orphans.contains(&"staging".to_owned()),
        "the deploy branch is kept: {orphans:?}"
    );
}
