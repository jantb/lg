use super::layers::parse_patch;

use super::*;

fn parse(toml_text: &str) -> std::result::Result<Preferences, String> {
    let json = parse_patch(toml_text).expect("valid TOML");
    serde_json::from_value::<Preferences>(json).map_err(|e| e.to_string())
}

#[test]
fn a_file_that_still_sets_the_retired_commit_limits_reads_and_saves_without_them() {
    let text =
        "version = 1\n[writing]\nlanguage = \"Norwegian\"\nsubject_max = 72\nbody_lines = 8\n";
    let prefs = parse(text).expect("the retired limits are not an error");
    assert_eq!(prefs.writing.language, "Norwegian");
    let rewritten = toml::to_string_pretty(&parse_patch(text).unwrap()).unwrap();
    assert!(rewritten.contains("Norwegian"), "{rewritten}");
    assert!(!rewritten.contains("subject_max"), "{rewritten}");
    assert!(!rewritten.contains("body_lines"), "{rewritten}");
}

#[test]
fn the_words_a_file_already_holds_still_read_and_write_back_unchanged() {
    let text = r#"
version = 1
[models]
provider = "claude"
[[agents]]
name = "c"
adapter = "codex"
executable = "codex"
confinement = "terrarium"
[branches]
environments = [{ id = "dev", branch = "dev" }]
promotions = [{ from = "feature", to = "dev", strategy = "ff-only" }]
"#;
    let prefs = parse(text).expect("parses");
    assert_eq!(prefs.models.provider, crate::llm::LlmProvider::Claude);
    assert_eq!(prefs.agents[0].adapter, Adapter::Codex);
    assert_eq!(prefs.agents[0].confinement, Confinement::Terrarium);
    assert_eq!(prefs.branches.promotions[0].strategy, Strategy::FfOnly);
    prefs.validate().expect("valid");

    let written = serde_json::to_value(&prefs).unwrap();
    assert_eq!(written["models"]["provider"], "claude");
    assert_eq!(written["agents"][0]["adapter"], "codex");
    assert_eq!(written["agents"][0]["confinement"], "terrarium");
    assert_eq!(written["branches"]["promotions"][0]["strategy"], "ff-only");
    assert_eq!(
        serde_json::to_value(Preferences::default()).unwrap()["models"]["provider"],
        "local"
    );
}

#[test]
fn a_word_outside_the_list_is_refused_saying_what_is_allowed() {
    let agent = |field: &str, word: &str| {
        format!("version = 1\n[[agents]]\nname = \"a\"\nexecutable = \"a\"\n{field} = \"{word}\"\n")
    };
    let err = parse(&agent("adapter", "bogus")).unwrap_err();
    assert!(err.contains("unknown agent adapter bogus"), "{err}");
    let err = parse(&agent("confinement", "jail")).unwrap_err();
    assert!(err.contains("terrarium, agent or direct"), "{err}");
    let err = parse("version = 1\n[models]\nprovider = \"gpt\"\n").unwrap_err();
    assert!(err.contains("local, claude"), "{err}");
    let err = parse(
        "version = 1\n[branches]\npromotions = [{ from = \"feature\", to = \"dev\", strategy = \"rebase\" }]\n",
    )
    .unwrap_err();
    assert!(err.contains("merge, squash or ff-only"), "{err}");
}
