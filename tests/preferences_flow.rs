//! Exercise configuration through the installed entry point with an isolated HOME.
use std::{
    path::Path,
    process::{Command, Output},
};
fn git(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn lg(home: &Path, root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lg"))
        .args(args)
        .current_dir(root)
        .env("HOME", home)
        .env("LG_PREFERENCES_DIR", home.join("config"))
        .env("LG_SETTINGS_DIR", home.join("legacy"))
        .env("LG_CONFIG_FILE", home.join("model"))
        .env_remove("LG_LLM_MODEL")
        .env_remove("LG_MTPLX_CHAT_ENDPOINT")
        .env_remove("LG_MTPLX_URL")
        .output()
        .unwrap()
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into()
}
fn setup(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "-b", "trunk"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "me@personal.example"]);
    git(root, &["commit", "--allow-empty", "-m", "Initial"]);
}
#[test]
fn folder_identity_and_worktree_preferences_follow_the_project() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let client = temp.path().join("client");
    let repo = client.join("project");
    setup(&repo);
    success(lg(
        &home,
        &repo,
        &[
            "config",
            "set",
            "tools.decorative_animations",
            "true",
            "--scope",
            "user",
        ],
    ));
    success(lg(
        &home,
        &repo,
        &[
            "config",
            "set",
            "tools.quiet_author_emails",
            "[\"*@client.example\"]",
            "--scope",
            "user",
        ],
    ));
    assert!(success(lg(&home, &repo, &["config", "appearance"])).contains("=true"));
    git(&repo, &["config", "user.email", "me@client.example"]);
    assert!(success(lg(&home, &repo, &["config", "appearance"])).contains("=false"));
    git(&repo, &["config", "user.email", "me@personal.example"]);
    success(lg(
        &home,
        &repo,
        &[
            "config",
            "set",
            "tools.decorative_animations",
            "false",
            "--scope",
            "folder",
            "--folder",
            client.to_str().unwrap(),
        ],
    ));
    assert!(success(lg(&home, &repo, &["config", "appearance"])).contains("=false"));
    success(lg(
        &home,
        &repo,
        &[
            "config",
            "set",
            "tools.decorative_animations",
            "true",
            "--scope",
            "repository",
        ],
    ));
    let worktree = client.join("worktree");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feature",
            worktree.to_str().unwrap(),
        ],
    );
    assert!(success(lg(&home, &worktree, &["config", "appearance"])).contains("=true"));
    success(lg(
        &home,
        &worktree,
        &[
            "config",
            "set",
            "tools.decorative_animations",
            "false",
            "--scope",
            "worktree",
        ],
    ));
    assert!(success(lg(&home, &worktree, &["config", "appearance"])).contains("=false"));
    assert!(success(lg(&home, &repo, &["config", "appearance"])).contains("=true"));
    success(lg(
        &home,
        &worktree,
        &["config", "reset", "tools", "--scope", "worktree"],
    ));
    assert!(success(lg(&home, &worktree, &["config", "appearance"])).contains("=true"));
}
#[test]
fn import_previews_and_rejects_invalid_policy_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    setup(&repo);
    let home = temp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let policy = temp.path().join("policy.toml");
    std::fs::write(&policy, "version = 1\n[branches]\nbase = 'trunk'\nremote = 'upstream'\nprotected = ['trunk', 'production']\npromotions = []\n[[branches.environments]]\nid = 'production'\nname = 'Production'\nremote = 'upstream'\nbranch = 'production'\n").unwrap();
    success(lg(
        &home,
        &repo,
        &["config", "import", policy.to_str().unwrap()],
    ));
    assert!(success(lg(&home, &repo, &["config", "show"])).contains("base = \"main\""));
    success(lg(
        &home,
        &repo,
        &["config", "import", policy.to_str().unwrap(), "--apply"],
    ));
    assert!(success(lg(&home, &repo, &["config", "show"])).contains("base = \"trunk\""));
    std::fs::write(&policy, "version = 999\n").unwrap();
    assert!(
        !lg(
            &home,
            &repo,
            &["config", "import", policy.to_str().unwrap(), "--apply"]
        )
        .status
        .success()
    );
    assert!(success(lg(&home, &repo, &["config", "show"])).contains("base = \"trunk\""));
}
#[test]
fn bundled_sandbox_cli_works_without_a_terrarium_executable() {
    let output = Command::new(env!("CARGO_BIN_EXE_lg"))
        .args(["sandbox", "--help"])
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let text = success(output);
    assert!(text.contains("sandbox") && text.contains("profile"));
}
