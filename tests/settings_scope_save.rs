//! Saving from the settings hub or `lg config set` writes what was changed to
//! the chosen scope, and nothing it merely showed: values inherited from other
//! files, the built-in prompt texts, and environment overrides stay where they
//! came from. One test in its binary, because it points the process's
//! configuration at temporary directories.
use std::path::{Path, PathBuf};
use std::process::Command;

use lg::panel::settings::{self, Category};
use lg::preferences::Scope;
use lg::state::AppState;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

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

/// A configuration home of its own, so each scenario starts with no files.
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

/// Open the hub on `category` at `scope`, apply `edit` to what it shows, and
/// save with Ctrl-S, as a person changing fields there would.
fn save_in_hub(category: Category, scope: Scope, edit: impl FnOnce(&mut Value)) {
    let mut state = AppState::default();
    settings::open(&mut state, category);
    state.settings_hub.scope = scope;
    edit(&mut state.settings_hub.draft);
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
    lg::preferences::invalidate();
}

fn config_set(args: &[&str]) {
    let mut all = vec!["set".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    lg::preferences_cli::run(&all).unwrap();
    lg::preferences::invalidate();
}

fn use_own_commit_prompt(text: &str) {
    let path = lg::settings::ensure_commit_prompt_file().unwrap();
    std::fs::write(path, text).unwrap();
    let path = lg::settings::ensure_review_style_file().unwrap();
    std::fs::write(path, format!("{text} review")).unwrap();
    lg::preferences::invalidate();
}

fn models() -> lg::preferences::Models {
    lg::preferences::load().config.models
}

fn branches() -> lg::preferences::Branches {
    lg::preferences::load().config.branches
}

fn environment_branches() -> Vec<String> {
    branches()
        .environments
        .into_iter()
        .map(|e| e.branch)
        .collect()
}

#[test]
fn saving_writes_only_what_was_changed_to_the_chosen_scope() {
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
    unsafe {
        std::env::set_var("PATH", &bin);
        for name in [
            "LG_LLM_ENABLED",
            "LG_LLM_MODEL",
            "LG_LLM_PROVIDER",
            "LG_MTPLX_CHAT_ENDPOINT",
            "LG_MTPLX_URL",
        ] {
            std::env::remove_var(name);
        }
    }

    // A writing field saved from the hub leaves the commit prompt and review
    // style to the files they are edited in.
    fresh_home(temp.path(), "home-writing");
    let repo = repository(&temp.path().join("writing"), &[]);
    work_in(&repo);
    save_in_hub(Category::Writing, Scope::Repository, |draft| {
        draft["language"] = json!("German")
    });
    use_own_commit_prompt("my own prompt");
    let writing = lg::settings::load();
    assert_eq!(writing.pr_language, "German");
    assert_eq!(writing.commit_prompt, "my own prompt");
    assert_eq!(writing.review_style, "my own prompt review");

    // The same through `lg config set`.
    fresh_home(temp.path(), "home-writing-cli");
    work_in(&repo);
    config_set(&["writing.language", "\"Dutch\"", "--scope", "repository"]);
    use_own_commit_prompt("cli prompt");
    let writing = lg::settings::load();
    assert_eq!(writing.pr_language, "Dutch");
    assert_eq!(writing.commit_prompt, "cli prompt");

    // Saving at user scope in one repository does not hand that repository's
    // own settings to every other one.
    fresh_home(temp.path(), "home-leak");
    let repo_a = repository(&temp.path().join("a"), &[]);
    let repo_b = repository(&temp.path().join("b"), &[]);
    work_in(&repo_a);
    lg::preferences::save_category(
        Scope::Repository,
        "models",
        json!({"model": "repo-a-model"}),
    )
    .unwrap();
    lg::preferences::save_category(
        Scope::Repository,
        "branches",
        json!({"base": "main", "protected": ["main", "staging"],
               "environments": [{"id": "prod", "name": "Production", "branch": "main"},
                                {"id": "staging", "name": "Staging", "branch": "staging"}]}),
    )
    .unwrap();
    lg::preferences::invalidate();
    save_in_hub(Category::Models, Scope::User, |draft| {
        draft["endpoint"] = json!("http://user.example/v1/chat/completions")
    });
    save_in_hub(Category::Branches, Scope::User, |draft| {
        draft["remote"] = json!("upstream")
    });
    config_set(&["models.claude_model", "\"opus\"", "--scope", "user"]);
    work_in(&repo_b);
    assert_ne!(models().model, "repo-a-model");
    assert_eq!(models().endpoint, "http://user.example/v1/chat/completions");
    assert_eq!(models().claude_model, "opus");
    assert_eq!(branches().remote, "upstream");
    assert!(!environment_branches().contains(&"staging".to_string()));

    // Saving at repository scope does not pin what the user's file says, so a
    // later change there still reaches this repository.
    fresh_home(temp.path(), "home-pin");
    work_in(&repo_a);
    lg::preferences::save_category(
        Scope::User,
        "models",
        json!({"endpoint": "http://first.example/v1/chat/completions"}),
    )
    .unwrap();
    lg::preferences::invalidate();
    save_in_hub(Category::Models, Scope::Repository, |draft| {
        draft["model"] = json!("repo-model")
    });
    config_set(&["models.claude_model", "\"haiku\"", "--scope", "repository"]);
    lg::preferences::save_category(
        Scope::User,
        "models",
        json!({"endpoint": "http://second.example/v1/chat/completions"}),
    )
    .unwrap();
    lg::preferences::invalidate();
    assert_eq!(models().model, "repo-model");
    assert_eq!(models().claude_model, "haiku");
    assert_eq!(
        models().endpoint,
        "http://second.example/v1/chat/completions"
    );

    // A model named for one run in the environment is not saved with an
    // unrelated change.
    fresh_home(temp.path(), "home-env");
    work_in(&repo_a);
    // SAFETY: as above.
    unsafe { std::env::set_var("LG_LLM_MODEL", "env-model") };
    lg::preferences::invalidate();
    save_in_hub(Category::Models, Scope::User, |draft| {
        draft["claude_model"] = json!("sonnet")
    });
    config_set(&["models.enabled", "true", "--scope", "repository"]);
    // SAFETY: as above.
    unsafe { std::env::remove_var("LG_LLM_MODEL") };
    lg::preferences::invalidate();
    assert_ne!(models().model, "env-model");
    assert_eq!(models().claude_model, "sonnet");

    // An environment deleted in the hub stays deleted, whether the
    // environments came from a file or were detected from the branches.
    fresh_home(temp.path(), "home-delete");
    let repo = repository(&temp.path().join("deploys"), &["develop", "test"]);
    work_in(&repo);
    save_in_hub(Category::Branches, Scope::Repository, |draft| {
        let remove = |rows: &mut Value, id: &str, key: &str| {
            rows.as_array_mut().unwrap().retain(|row| row[key] != id)
        };
        remove(&mut draft["environments"], "test", "id");
        remove(&mut draft["promotions"], "test", "to");
    });
    assert!(environment_branches().contains(&"develop".to_string()));
    assert!(!environment_branches().contains(&"test".to_string()));
    assert!(branches().protected.contains(&"develop".to_string()));
    save_in_hub(Category::Branches, Scope::Repository, |draft| {
        let remove = |rows: &mut Value, id: &str, key: &str| {
            rows.as_array_mut().unwrap().retain(|row| row[key] != id)
        };
        remove(&mut draft["environments"], "dev", "id");
        remove(&mut draft["promotions"], "dev", "to");
    });
    assert!(!environment_branches().contains(&"develop".to_string()));
    assert!(environment_branches().contains(&"main".to_string()));
}
