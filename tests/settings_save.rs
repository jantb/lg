//! The settings modal saves a handful of fields and knows nothing of scopes.
//! What it saves has to be what lg reads back, even when a preferences file
//! already has a say in the same category. The only test in its binary,
//! because it points the process's configuration at a temporary HOME.
use std::{path::Path, process::Command};

use lg::preferences::Scope;

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
fn settings_saved_from_the_modal_win_over_preferences_already_set() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["commit", "--allow-empty", "-m", "Initial"]);
    // SAFETY: this is the only test in the binary, and nothing else runs yet.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("LG_PREFERENCES_DIR", home.join("config"));
        std::env::set_var("LG_SETTINGS_DIR", home.join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", home.join("model"));
        for name in [
            "LG_LLM_MODEL",
            "LG_LLM_PROVIDER",
            "LG_MTPLX_CHAT_ENDPOINT",
            "LG_MTPLX_URL",
        ] {
            std::env::remove_var(name);
        }
    }
    lg::git::set_active_repo(&repo);

    // Preferences already decide both categories, from the user's file and
    // from the repository's, which is more specific than the user scope the
    // model choice used to be saved at.
    lg::preferences::save_category(
        Scope::User,
        "writing",
        serde_json::json!({"language": "Norwegian", "subject_max": 50}),
    )
    .unwrap();
    lg::preferences::save_category(
        Scope::Repository,
        "writing",
        serde_json::json!({"subject_max": 60, "comment_style": "repo style"}),
    )
    .unwrap();
    lg::preferences::save_category(
        Scope::Repository,
        "models",
        serde_json::json!({"model": "repo-model", "provider": "local"}),
    )
    .unwrap();

    lg::settings::save(&lg::settings::RepoSettings {
        pr_language: "English".into(),
        comment_style: "Conventional Commits".into(),
        commit_subject_max_chars: 40,
        commit_body_max_lines: 3,
        ..lg::settings::RepoSettings::default()
    })
    .unwrap();
    lg::llm::save_llm_settings("saved-model", lg::llm::LlmProvider::Mtplx).unwrap();

    let loaded = lg::settings::load();
    assert_eq!(loaded.pr_language, "English");
    assert_eq!(loaded.comment_style, "Conventional Commits");
    assert_eq!(loaded.commit_subject_max_chars, 40);
    assert_eq!(loaded.commit_body_max_lines, 3);
    assert_eq!(lg::preferences::load().config.models.model, "saved-model");
    assert_eq!(lg::llm::current_model(), "saved-model");
    assert_eq!(lg::llm::current_provider(), lg::llm::LlmProvider::Mtplx);

    // Resetting from the modal takes back what it saved, so the preferences
    // underneath show through again.
    lg::settings::clear().unwrap();
    lg::llm::clear_saved_llm_settings().unwrap();

    let loaded = lg::settings::load();
    assert_eq!(loaded.pr_language, "Norwegian");
    assert_eq!(loaded.commit_subject_max_chars, 60);
    assert_ne!(lg::preferences::load().config.models.model, "saved-model");
}
