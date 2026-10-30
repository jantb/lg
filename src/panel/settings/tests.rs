use super::*;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

/// A hub over the configuration detected for a checkout with develop and
/// test branches, with no file read.
fn branches_hub() -> AppState {
    let mut state = AppState::default();
    state.decorative_animations = false;
    let detected = preferences::detect_branches(&["main".into(), "develop".into(), "test".into()]);
    state.settings_hub = Settings {
        category: 5,
        draft: serde_json::to_value(detected).unwrap(),
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    state
}

fn drawn(state: &AppState, area: Rect) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal.draw(|frame| render(state, area, frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn nested_entries_are_shown_under_their_own_heading() {
    let state = branches_hub();
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 120)));
    assert!(screen.contains("#1 Development"), "{screen}");
    assert!(screen.contains("#1 feature \u{2192} dev"), "{screen}");
    assert!(
        !screen.contains("environments #1 \u{b7} branch"),
        "the flat labels are gone: {screen}"
    );
}

#[test]
fn a_long_value_wraps_instead_of_running_off_the_pane() {
    let mut state = branches_hub();
    state.settings_hub.draft["base"] = Value::String("word ".repeat(40).trim().into());
    let screen = text(&drawn(&state, Rect::new(0, 0, 100, 40)));
    let value_rows = screen
        .lines()
        .filter(|line| line.contains("word word"))
        .count();
    assert!(value_rows >= 2, "{screen}");
}

#[test]
fn clicking_a_field_selects_it_and_clicking_again_edits_it() {
    let mut state = branches_hub();
    let area = Rect::new(0, 0, 120, 120);
    let r = regions(area);
    // Draw once so the scroll position is settled, then find `remote` on screen.
    let buf = drawn(&state, area);
    let row = (r.fields.y..r.fields.y + r.fields.height)
        .find(|y| {
            // Past the two marker columns; the top-level field is not indented.
            (r.fields.x + 2..r.fields.x + r.fields.width)
                .map(|x| buf[(x, *y)].symbol().to_string())
                .collect::<String>()
                .starts_with("remote: origin")
        })
        .expect("remote is on screen");
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: r.fields.x + 4,
        row,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(&mut state, area, &click);
    let selected = fields(&state.settings_hub)[state.settings_hub.selected].clone();
    assert_eq!(selected.path, "/remote");
    assert!(!state.settings_hub.editing);
    handle_mouse(&mut state, area, &click);
    assert!(state.settings_hub.editing);
    assert_eq!(state.settings_hub.input.text, "origin");
}

#[test]
fn clicking_a_category_switches_to_it() {
    let mut state = branches_hub();
    let area = Rect::new(0, 0, 120, 40);
    let r = regions(area);
    handle_mouse(
        &mut state,
        area,
        &MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.categories.x + 3,
            row: r.categories.y + 3,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert_eq!(state.settings_hub.key(), "agents");
}

fn press(state: &mut AppState, code: KeyCode) {
    handle_key(state, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

/// A hub over the writing category with the default preferences.
/// A check whose worker died must not leave "Checking…" up for good.
#[test]
fn an_agent_check_that_dies_says_so_instead_of_checking_forever() {
    let mut state = AppState::default();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    state.settings_hub.diagnostics = Some(rx);
    drop(tx);

    poll(&mut state);

    assert!(state.settings_hub.diagnostics.is_none());
    assert_eq!(state.settings_hub.notice_kind, NoticeKind::Error);
    assert!(
        state.settings_hub.notice.contains("stopped unexpectedly"),
        "{}",
        state.settings_hub.notice
    );
}

#[test]
fn a_finished_agent_check_shows_its_report() {
    let mut state = AppState::default();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    state.settings_hub.diagnostics = Some(rx);
    tx.send("claude 1.0".into()).unwrap();

    poll(&mut state);

    assert!(state.settings_hub.diagnostics.is_none());
    assert_eq!(state.settings_hub.notice, "claude 1.0");
}

fn writing_hub() -> AppState {
    let mut state = AppState::default();
    state.decorative_animations = false;
    state.settings_hub = Settings {
        category: 1,
        draft: serde_json::to_value(preferences::Preferences::default()).unwrap(),
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    state
}

#[test]
fn language_is_picked_from_the_supported_languages() {
    let mut state = writing_hub();
    select(&mut state, "/writing/language");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        state.settings_hub.draft["writing"]["language"],
        json!("Norwegian")
    );
    press(&mut state, KeyCode::Enter);
    for c in "Klingon".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    assert!(
        state.settings_hub.editing,
        "an unsupported language is refused"
    );
    assert!(
        state.settings_hub.notice_kind == NoticeKind::Error,
        "{}",
        state.settings_hub.notice
    );
    assert_eq!(
        state.settings_hub.draft["writing"]["language"],
        json!("Norwegian")
    );
}

#[test]
fn enter_on_a_boolean_flips_it_without_opening_the_editor() {
    let mut state = branches_hub();
    state.settings_hub.draft["flag"] = Value::Bool(false);
    state.settings_hub.original = state.settings_hub.draft.clone();
    state.settings_hub.selected = fields(&state.settings_hub)
        .iter()
        .position(|f| f.path == "/flag")
        .unwrap();
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.settings_hub.draft["flag"], Value::Bool(true));
    assert!(!state.settings_hub.editing);
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.settings_hub.draft["flag"], Value::Bool(false));
}

#[test]
fn left_arrow_moves_to_the_categories_and_up_down_switch_them() {
    let mut state = branches_hub();
    let before = state.settings_hub.key();
    press(&mut state, KeyCode::Left);
    press(&mut state, KeyCode::Down);
    assert_ne!(state.settings_hub.key(), before);
    press(&mut state, KeyCode::Up);
    assert_eq!(state.settings_hub.key(), before);
    // Back in the fields, Down moves the field selection, not the category.
    press(&mut state, KeyCode::Right);
    press(&mut state, KeyCode::Down);
    assert_eq!(state.settings_hub.key(), before);
    assert_eq!(state.settings_hub.selected, 1);
}

fn with_local_branches(state: &mut AppState, names: &[&str]) {
    state.branches = names
        .iter()
        .map(|name| crate::git::Branch {
            name: (*name).into(),
            is_current: false,
            upstream: None,
            upstream_gone: false,
            ahead: 0,
            behind: 0,
            behind_main: 0,
            last_commit_unix: None,
        })
        .collect();
}

fn select(state: &mut AppState, path: &str) {
    state.settings_hub.selected = fields(&state.settings_hub)
        .iter()
        .position(|f| f.path == path)
        .unwrap();
}

/// A field that takes one of a few words is chosen from those words, and
/// a word that is not one of them never reaches the draft.
#[test]
fn a_promotion_strategy_is_chosen_from_the_allowed_values() {
    let mut state = branches_hub();
    select(&mut state, "/promotions/0/strategy");
    let before = state.settings_hub.draft["promotions"][0]["strategy"].clone();
    press(&mut state, KeyCode::Enter);
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    for option in preferences::STRATEGIES {
        assert!(screen.contains(option), "{option} is offered: {screen}");
    }
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Enter);
    assert!(!state.settings_hub.editing);
    let after = state.settings_hub.draft["promotions"][0]["strategy"].clone();
    assert_ne!(after, before);
    assert!(preferences::STRATEGIES.contains(&after.as_str().unwrap()));

    press(&mut state, KeyCode::Enter);
    for c in "rebase".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.settings_hub.draft["promotions"][0]["strategy"], after);
    assert!(
        state.settings_hub.notice.contains("one of"),
        "{}",
        state.settings_hub.notice
    );
}

#[test]
fn the_identity_scope_is_chosen_from_repository_or_folder() {
    let mut state = AppState::default();
    state.decorative_animations = false;
    state.settings_hub = Settings {
        category: 0,
        draft: json!({"scope": "Repository", "folder": "/tmp", "name": "", "email": ""}),
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    select(&mut state, "/scope");
    press(&mut state, KeyCode::Enter);
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    for option in IDENTITY_SCOPES {
        assert!(screen.contains(option), "{option} is offered: {screen}");
    }
    for c in "Folder".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.settings_hub.draft["scope"], json!("Folder"));

    press(&mut state, KeyCode::Enter);
    for c in "Everywhere".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.settings_hub.draft["scope"], json!("Folder"));
}

/// Typing the whole word is as good as highlighting it.
#[test]
fn a_typed_allowed_value_is_applied() {
    let mut state = branches_hub();
    select(&mut state, "/promotions/0/strategy");
    press(&mut state, KeyCode::Enter);
    for c in "squash".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        state.settings_hub.draft["promotions"][0]["strategy"],
        Value::String("squash".into())
    );
}

/// An agent's confinement is one of three words, offered as a list.
#[test]
fn confinement_is_chosen_from_a_list() {
    let mut state = AppState::default();
    state.decorative_animations = false;
    let config = preferences::Preferences::default();
    state.settings_hub = Settings {
        category: 3,
        draft: serde_json::to_value(&config.agents).unwrap(),
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    select(&mut state, "/0/confinement");
    press(&mut state, KeyCode::Enter);
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    for option in preferences::CONFINEMENTS {
        assert!(screen.contains(option), "{option} is offered: {screen}");
    }
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Enter);
    let chosen = state.settings_hub.draft[0]["confinement"].clone();
    assert!(preferences::CONFINEMENTS.contains(&chosen.as_str().unwrap()));
}

/// The editor is picked from the programs on this machine, and a program
/// that is not listed may still be typed.
#[test]
fn the_editor_is_offered_from_the_programs_found() {
    let mut state = AppState::default();
    state.decorative_animations = false;
    let config = preferences::Preferences::default();
    state.settings_hub = Settings {
        category: 6,
        draft: serde_json::to_value(&config.tools).unwrap(),
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    state.settings_hub.editors = vec!["hx".into(), "nvim".into()];
    select(&mut state, "/editor");
    press(&mut state, KeyCode::Enter);
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    assert!(screen.contains("nvim"), "{screen}");
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        state.settings_hub.draft["editor"],
        Value::String("hx".into())
    );

    press(&mut state, KeyCode::Enter);
    for c in "mate".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        state.settings_hub.draft["editor"],
        Value::String("mate".into())
    );
}

#[test]
fn an_environment_branch_is_picked_from_the_local_branches() {
    let mut state = branches_hub();
    with_local_branches(&mut state, &["main", "staging", "feature/x"]);
    select(&mut state, "/environments/1/branch");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings_hub.editing);
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    assert!(screen.contains("staging"), "{screen}");
    for c in "stag".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Enter);
    assert!(!state.settings_hub.editing);
    assert_eq!(
        state.settings_hub.draft["environments"][1]["branch"],
        Value::String("staging".into())
    );
}

#[test]
fn the_model_is_picked_from_what_the_endpoint_serves() {
    let mut state = AppState::default();
    state.decorative_animations = false;
    state.settings_hub = Settings {
        category: 2,
        draft: json!({"endpoint": "http://localhost:8000/v1/chat/completions", "model": "old"}),
        models: vec!["Qwen3-27B".into(), "gpt-oss-120b".into()],
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    select(&mut state, "/model");
    press(&mut state, KeyCode::Enter);
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    assert!(screen.contains("Qwen3-27B"), "{screen}");
    assert!(screen.contains("gpt-oss-120b"), "{screen}");
    for c in "gpt".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Enter);
    assert!(!state.settings_hub.editing);
    assert_eq!(
        state.settings_hub.draft["model"],
        Value::String("gpt-oss-120b".into())
    );
}

#[test]
fn a_claude_alias_shows_which_model_it_stands_for() {
    let mut state = AppState::default();
    state.decorative_animations = false;
    state.settings_hub = Settings {
        category: 2,
        draft: json!({"claude_model": ""}),
        claude_models: vec![
            crate::llm::ClaudeModel {
                name: "opus".into(),
                note: "Opus 5.5 \u{b7} claude-opus-5-5".into(),
            },
            crate::llm::ClaudeModel {
                name: "claude-opus-4-8".into(),
                note: "Opus 4.8".into(),
            },
        ],
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    select(&mut state, "/claude_model");
    press(&mut state, KeyCode::Enter);
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    assert!(screen.contains("claude-opus-5-5"), "{screen}");
    assert!(screen.contains("Opus 4.8"), "{screen}");
    for c in "opus".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    // The note is shown, not saved.
    assert_eq!(
        state.settings_hub.draft["claude_model"],
        Value::String("opus".into())
    );
}

/// The settings only one provider reads sit under that provider's
/// heading, and the heading of the provider not chosen says it is unused.
#[test]
fn the_provider_not_chosen_is_marked_unused() {
    let mut state = AppState::default();
    state.decorative_animations = false;
    state.settings_hub = Settings {
        category: 2,
        draft: json!({
            "enabled": true,
            "provider": "claude",
            "endpoint": "http://localhost:8000/v1/chat/completions",
            "model": "Qwen3-27B",
            "claude_model": "opus",
        }),
        ..Settings::default()
    };
    state.settings_hub.original = state.settings_hub.draft.clone();
    let heading = |state: &AppState, name: &str| {
        text(&drawn(state, Rect::new(0, 0, 120, 40)))
            .lines()
            .find(|l| {
                l.trim_start_matches(|c: char| !c.is_alphanumeric())
                    .starts_with(&format!("{name}  "))
            })
            .map(str::to_string)
            .unwrap_or_default()
    };
    assert!(
        heading(&state, "local").contains("not used"),
        "{}",
        heading(&state, "local")
    );
    assert!(
        !heading(&state, "claude").contains("not used"),
        "{}",
        heading(&state, "claude")
    );

    state.settings_hub.draft["provider"] = json!("local");
    assert!(
        heading(&state, "claude").contains("not used"),
        "{}",
        heading(&state, "claude")
    );
    assert!(
        !heading(&state, "local").contains("not used"),
        "{}",
        heading(&state, "local")
    );
}

#[test]
fn a_typed_branch_name_is_kept_when_nothing_is_picked() {
    let mut state = branches_hub();
    with_local_branches(&mut state, &["main"]);
    select(&mut state, "/base");
    press(&mut state, KeyCode::Enter);
    for c in "trunk".chars() {
        press(&mut state, KeyCode::Char(c));
    }
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        state.settings_hub.draft["base"],
        Value::String("trunk".into())
    );
}

#[test]
fn assigning_an_environment_branch_makes_it_a_release_target() {
    let mut state = branches_hub();
    with_local_branches(&mut state, &["staging"]);
    press(&mut state, KeyCode::Char('n'));
    let index = state.settings_hub.draft["environments"]
        .as_array()
        .unwrap()
        .len()
        - 1;
    let id = state.settings_hub.draft["environments"][index]["id"]
        .as_str()
        .unwrap()
        .to_string();
    select(&mut state, &format!("/environments/{index}/branch"));
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Enter);
    let draft = &state.settings_hub.draft;
    assert!(
        draft["protected"]
            .as_array()
            .unwrap()
            .contains(&Value::String("staging".into())),
        "{draft}"
    );
    assert!(
        draft["promotions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["to"].as_str() == Some(id.as_str())),
        "{draft}"
    );
}

#[test]
fn the_selected_field_is_explained_below_the_list() {
    let mut state = branches_hub();
    state.settings_hub.selected = fields(&state.settings_hub)
        .iter()
        .position(|f| f.path == "/environments/0/id")
        .unwrap();
    let screen = text(&drawn(&state, Rect::new(0, 0, 120, 40)));
    assert!(screen.contains("dev and test"), "{screen}");
}

/// A capital L is part of a commit style like "Like Linux", not a shortcut.
#[test]
fn a_capital_l_is_typed_into_the_comment_style() {
    let mut state = writing_hub();
    select(&mut state, "/writing/comment_style");
    press(&mut state, KeyCode::Enter);
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    )
    .unwrap();
    press(&mut state, KeyCode::Char('L'));
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        state.settings_hub.draft["writing"]["comment_style"],
        json!("L")
    );
    assert_eq!(state.modal, Modal::None);
}

#[test]
fn ctrl_l_derives_the_style_from_inside_the_comment_style_field() {
    let mut state = writing_hub();
    select(&mut state, "/writing/comment_style");
    press(&mut state, KeyCode::Enter);
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(state.modal, Modal::Model);
}
