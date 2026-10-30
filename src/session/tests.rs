use super::*;

#[test]
fn a_sandboxed_session_goes_through_terrarium_in_its_own_worktree() {
    let spawn = claude_spawn(Path::new("/dev/lg.worktrees/feat-x"), true, None, None);
    assert_eq!(
        spawn.program,
        crate::terrarium::executable().to_string_lossy()
    );
    assert_eq!(
        spawn.args,
        [
            "sandbox",
            "run",
            "--project",
            "/dev/lg.worktrees/feat-x",
            "--",
            "claude"
        ]
    );
    assert_eq!(spawn.cwd, Path::new("/dev/lg.worktrees/feat-x"));
}

#[test]
fn an_unsandboxed_session_runs_claude_in_auto_mode() {
    let spawn = claude_spawn(Path::new("/dev/lg"), false, None, None);
    assert_eq!(spawn.program, "claude");
    assert_eq!(spawn.args, ["--permission-mode", "auto"]);
    assert_eq!(spawn.cwd, Path::new("/dev/lg"));
}

/// The hook settings are claude's argument. A sandboxed session is started by
/// terrarium, so they have to land after the `--` that separates the two
/// command lines, or terrarium would try to take them itself.
#[test]
fn hook_settings_are_passed_to_claude_and_not_to_terrarium() {
    let settings = Path::new("/dev/lg/.git/lg/sessions/dev-lg/settings.json");

    let sandboxed = claude_spawn(Path::new("/dev/lg"), true, Some(settings), None);
    assert_eq!(
        sandboxed.args,
        [
            "sandbox",
            "run",
            "--project",
            "/dev/lg",
            "--",
            "claude",
            "--settings",
            "/dev/lg/.git/lg/sessions/dev-lg/settings.json"
        ]
    );

    let plain = claude_spawn(Path::new("/dev/lg"), false, Some(settings), None);
    assert_eq!(
        plain.args,
        [
            "--permission-mode",
            "auto",
            "--settings",
            "/dev/lg/.git/lg/sessions/dev-lg/settings.json"
        ]
    );
}

/// Sandboxed, terrarium is the confinement, so codex's own sandbox — which
/// would be nested inside it — is turned off rather than stacked.
#[test]
fn a_sandboxed_codex_leaves_the_confining_to_terrarium() {
    let spawn = codex_spawn(Path::new("/dev/lg.worktrees/feat-x"), true, None);
    assert_eq!(
        spawn.program,
        crate::terrarium::executable().to_string_lossy()
    );
    assert_eq!(
        spawn.args,
        [
            "sandbox",
            "run",
            "--project",
            "/dev/lg.worktrees/feat-x",
            "--",
            "codex",
            "--dangerously-bypass-approvals-and-sandbox",
        ]
    );
    assert_eq!(spawn.cwd, Path::new("/dev/lg.worktrees/feat-x"));
}

/// With nothing else holding it, codex holds itself to the checkout — as
/// far as claude's auto mode goes here.
#[test]
fn an_unsandboxed_codex_confines_itself_to_the_checkout() {
    let spawn = codex_spawn(Path::new("/dev/lg"), false, None);
    assert_eq!(spawn.program, "codex");
    assert_eq!(spawn.args, ["--sandbox", "workspace-write"]);
    assert_eq!(spawn.cwd, Path::new("/dev/lg"));
}

#[test]
fn a_sandboxed_pi_goes_through_terrarium_too() {
    let spawn = pi_spawn(Path::new("/dev/lg.worktrees/feat-x"), true, None);
    assert_eq!(
        spawn.program,
        crate::terrarium::executable().to_string_lossy()
    );
    assert_eq!(
        spawn.args,
        [
            "sandbox",
            "run",
            "--project",
            "/dev/lg.worktrees/feat-x",
            "--",
            "pi"
        ]
    );
}

#[test]
fn an_unsandboxed_pi_runs_with_nothing_added() {
    let spawn = pi_spawn(Path::new("/dev/lg"), false, None);
    assert_eq!(spawn.program, "pi");
    assert!(spawn.args.is_empty(), "pi has no mode to ask for");
}

/// Every agent opens on the conflict lg hands it, and the prompt has to
/// arrive as the first turn rather than as a flag — which is why pi's goes
/// behind a `--`, since a prompt can start with a dash.
#[test]
fn every_agent_opens_on_the_prompt_it_is_given() {
    const PROMPT: &str = "- resolve the merge conflict";

    assert_eq!(
        claude_spawn(Path::new("/dev/lg"), false, None, Some(PROMPT))
            .args
            .last()
            .map(String::as_str),
        Some(PROMPT)
    );
    assert_eq!(
        codex_spawn(Path::new("/dev/lg"), false, Some(PROMPT))
            .args
            .last()
            .map(String::as_str),
        Some(PROMPT)
    );

    let pi = pi_spawn(Path::new("/dev/lg"), false, Some(PROMPT));
    assert_eq!(pi.args, ["--", PROMPT]);
}

#[test]
fn a_blank_prompt_reaches_no_agent_as_an_empty_first_turn() {
    assert!(
        codex_spawn(Path::new("/dev/lg"), false, Some("   "))
            .args
            .iter()
            .all(|arg| arg.starts_with('-') || arg == "workspace-write")
    );
    assert!(
        pi_spawn(Path::new("/dev/lg"), false, Some("   "))
            .args
            .is_empty()
    );
}

/// One of each per checkout, so a worktree can be running claude, codex, pi
/// and a shell at the same time without any of them replacing another.
#[test]
fn a_checkout_holds_one_session_of_every_kind_at_once() {
    let mut sessions = Sessions::default();
    for (id, kind) in [
        SessionKind::Claude,
        SessionKind::Codex,
        SessionKind::Pi,
        SessionKind::Terminal,
    ]
    .into_iter()
    .enumerate()
    {
        sessions.items.push(fake_of(id as u64, "/dev/lg", kind));
    }

    for kind in [
        SessionKind::Claude,
        SessionKind::Codex,
        SessionKind::Pi,
        SessionKind::Terminal,
    ] {
        assert!(
            sessions.for_dir_kind(Path::new("/dev/lg"), kind).is_some(),
            "{} should have its own session here",
            kind.label()
        );
    }
    assert_eq!(sessions.for_dir(Path::new("/dev/lg")).count(), 4);
}

#[test]
fn a_terminal_runs_the_users_shell_with_no_arguments() {
    temp_env("SHELL", "/bin/zsh", || {
        let spawn = shell_spawn(Path::new("/dev/lg"));
        assert_eq!(spawn.program, "/bin/zsh");
        assert!(spawn.args.is_empty(), "an interactive shell needs no flags");
        assert_eq!(spawn.cwd, Path::new("/dev/lg"));
    });
}

#[test]
fn a_terminal_falls_back_to_a_shell_every_machine_has() {
    temp_env("SHELL", "", || {
        assert_eq!(shell_spawn(Path::new("/dev/lg")).program, "/bin/sh");
    });
}

/// A shell is where claude gets started by hand, so it must not inherit the
/// markers that would make that claude think it is nested.
#[test]
fn a_terminal_declares_a_terminal_and_drops_the_nested_claude_markers() {
    let spawn = shell_spawn(Path::new("/dev/lg"));
    assert!(
        spawn
            .env
            .contains(&("TERM".to_string(), "xterm-256color".to_string()))
    );
    assert!(spawn.env_remove.contains(&"CLAUDECODE".to_string()));
}

/// Runs `body` with `SHELL` set to `value`, restoring it afterwards. The
/// shell tests share the process environment, so they also share this lock.
fn temp_env(name: &str, value: &str, body: impl FnOnce()) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let previous = std::env::var_os(name);
    // SAFETY: the lock keeps these tests from racing each other, and the
    // rest of the suite does not read SHELL.
    unsafe { std::env::set_var(name, value) };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    match previous {
        Some(previous) => unsafe { std::env::set_var(name, previous) },
        None => unsafe { std::env::remove_var(name) },
    }
    drop(guard);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// A prompt is claude's first turn, so it has to land after every flag —
/// and after the `--` a sandboxed session goes through, or terrarium would
/// take it for its own.
#[test]
fn an_opening_prompt_is_the_last_argument_claude_gets() {
    let settings = Path::new("/dev/lg/.git/lg/sessions/dev-lg/settings.json");
    let spawn = claude_spawn(
        Path::new("/dev/lg"),
        true,
        Some(settings),
        Some("resolve the conflict"),
    );
    assert_eq!(
        spawn.args.last().map(String::as_str),
        Some("resolve the conflict")
    );
    assert_eq!(
        spawn.args,
        [
            "sandbox",
            "run",
            "--project",
            "/dev/lg",
            "--",
            "claude",
            "--settings",
            "/dev/lg/.git/lg/sessions/dev-lg/settings.json",
            "resolve the conflict"
        ]
    );
}

#[test]
fn a_blank_prompt_is_no_prompt_at_all() {
    let spawn = claude_spawn(Path::new("/dev/lg"), false, None, Some("   "));
    assert_eq!(spawn.args, ["--permission-mode", "auto"]);
}

#[test]
fn sessions_declare_a_terminal_and_drop_the_nested_claude_markers() {
    let spawn = claude_spawn(Path::new("/dev/lg"), false, None, None);
    assert!(
        spawn
            .env
            .contains(&("TERM".to_string(), "xterm-256color".to_string()))
    );
    assert!(spawn.env_remove.contains(&"CLAUDECODE".to_string()));
}

#[test]
fn a_pane_maps_to_the_screen_size_inside_its_border() {
    assert_eq!(
        size_for_pane(ratatui::layout::Rect::new(0, 0, 82, 26)),
        (24, 80)
    );
    assert_eq!(
        size_for_pane(ratatui::layout::Rect::new(0, 0, 1, 1)),
        (1, 1),
        "a pane too small for its border still gets a usable size"
    );
}

/// Sessions that do not need a real process: enough to exercise the
/// registry's bookkeeping.
fn fake(id: u64, cwd: &str) -> Session {
    fake_of(id, cwd, SessionKind::Claude)
}

fn fake_of(id: u64, cwd: &str, kind: SessionKind) -> Session {
    Session {
        id: SessionId(id),
        label: format!("session {id}"),
        seq: 1,
        cwd: PathBuf::from(cwd),
        sandboxed: false,
        kind,
        status: SessionStatus::Running,
        launch: None,
        attention: false,
        activity: SessionActivity::Idle,
        asking: false,
        events: None,
        last_output: None,
        reading: Reading::default(),
        parser: vt100::Parser::new(24, 80, 0),
        process: None,
    }
}

fn registry(dirs: &[&str]) -> Sessions {
    let mut sessions = Sessions::new();
    for (idx, dir) in dirs.iter().enumerate() {
        let id = idx as u64 + 1;
        sessions.items.push(fake(id, dir));
        sessions.next_id = id + 1;
    }
    sessions.focused = sessions.items.first().map(|session| session.id);
    sessions
}

/// Busy or ready is claude's to report, not lg's to guess: the spinner line
/// says nothing lg reads any more. Matching it went wrong twice — once when
/// `esc to interrupt` stopped being printed, once when a turn past a minute
/// grew a `1m ` its clock did not have before.
#[test]
fn the_spinner_line_is_not_read_at_all() {
    for line in [
        "\u{273b} Enchanting\u{2026} (12s \u{b7} still thinking with xhigh effort)",
        "\u{273b} Enchanting\u{2026} (1m 12s \u{b7} esc to interrupt)",
        "\u{273b} Brewed for 4s",
    ] {
        assert!(!is_asking(line), "{line:?} is not a question");
    }
}

/// The prompt every fresh checkout opens on, and the reason the screen is
/// still read: it is up before claude has run a single hook.
#[test]
fn the_trust_prompt_reads_as_needing_input() {
    let screen = "Quick safety check: Is this a project you created or one you trust?\n\
                  \u{276f} 1. Yes, I trust this folder\n  2. No, exit";
    assert!(is_asking(screen));
}

/// An answered question stays on screen with its chosen row ticked. Reading
/// that as a live question is what left a session red after it had been
/// dealt with.
#[test]
fn an_answered_question_stops_reading_as_one() {
    assert!(!is_asking("\u{276f} 1. Yes, I trust this folder \u{2714}"));
}

/// A shell or an agent without hooks has only its screen to be judged by:
/// while it is writing it is working, and once it has been quiet for a
/// moment it is ready again.
#[test]
fn a_program_without_hooks_is_working_while_it_writes() {
    let mut session = fake_of(1, "/a", SessionKind::Terminal);
    assert_eq!(session.activity(), SessionActivity::Idle);
    session.last_output = Some(std::time::Instant::now());
    assert_eq!(session.activity(), SessionActivity::Working);
    session.last_output = Some(
        std::time::Instant::now() - std::time::Duration::from_millis(OUTPUT_ACTIVE_MS as u64 + 500),
    );
    assert_eq!(session.activity(), SessionActivity::Idle);
}

/// codex asks before running a command the way claude does, with a choice
/// list under the question, so it gets the same red dot. Prose that merely
/// mentions approving something does not.
#[test]
fn a_codex_approval_prompt_is_a_question() {
    let screen = "Allow command?\n\u{276f} 1. Yes (y)\n  2. Yes, and don't ask again\n  3. No\n";
    assert!(is_asking(screen));
    assert!(!is_asking(
        "I approve of this change; the reviewer approved it too.\n1. Notes\n"
    ));
}

/// A permission prompt comes up mid-turn, so the hooks have the session down
/// as working when it is really blocked on an answer. The question wins.
#[test]
fn a_question_outranks_what_the_hooks_last_said() {
    let mut session = fake(1, "/a");
    session.activity = SessionActivity::Working;
    session.asking = true;

    assert_eq!(session.activity(), SessionActivity::NeedsInput);
}

#[test]
fn the_ordinary_prompt_caret_is_not_a_question() {
    for line in [
        "\u{276f} Try \"fix lint errors\"",
        "\u{276f} think carefully and write an essay",
        "\u{23f5}\u{23f5} auto mode on (shift+tab to cycle)",
    ] {
        assert!(
            !is_asking(line),
            "{line:?} is claude waiting for a command, not asking one"
        );
    }
}

#[test]
fn prose_that_merely_mentions_a_question_is_not_a_question() {
    assert!(
        !is_asking("I can add the flag if you want. Do you want me to also update the docs?"),
        "an answer that uses the words is not a prompt with options to pick"
    );
}

#[test]
fn a_quiet_screen_reads_as_ready() {
    assert!(!is_asking("Welcome to claude\n\n"));
}

/// Nothing has reported in yet, and nothing is being asked. A session that
/// has said nothing is not busy.
#[test]
fn a_session_that_has_reported_nothing_reads_as_ready() {
    assert_eq!(fake(1, "/a").activity(), SessionActivity::Idle);
}

#[test]
fn an_ended_session_is_never_reported_as_busy() {
    let mut session = fake(1, "/a");
    session.activity = SessionActivity::Working;
    assert_eq!(session.activity(), SessionActivity::Working);

    session.status = SessionStatus::Ended("stopped".into());
    assert_eq!(
        session.activity(),
        SessionActivity::Idle,
        "a dead session is not still working"
    );
}

#[test]
fn a_checkout_has_at_most_one_session_of_each_kind() {
    let mut sessions = registry(&["/a", "/b"]);
    sessions.items.push(fake_of(3, "/b", SessionKind::Terminal));

    assert_eq!(
        sessions.for_dir_kind(Path::new("/b"), SessionKind::Claude),
        Some(SessionId(2))
    );
    assert_eq!(
        sessions.for_dir_kind(Path::new("/b"), SessionKind::Terminal),
        Some(SessionId(3)),
        "a terminal alongside claude is a second session, not the same one"
    );
    assert_eq!(
        sessions.for_dir_kind(Path::new("/c"), SessionKind::Claude),
        None
    );
}

#[test]
fn a_checkout_lists_every_session_running_in_it() {
    let mut sessions = registry(&["/a", "/b"]);
    sessions.items.push(fake_of(3, "/b", SessionKind::Terminal));

    let ids: Vec<SessionId> = sessions
        .for_dir(Path::new("/b"))
        .map(|session| session.id)
        .collect();
    assert_eq!(ids, [SessionId(2), SessionId(3)]);
    assert!(sessions.any_in_dir(Path::new("/a")));
    assert!(!sessions.any_in_dir(Path::new("/c")));
}

/// The header reports these, and it must not cry wolf: a session that has
/// only printed something is busy, not blocked.
#[test]
fn unread_output_is_not_counted_as_a_session_needing_input() {
    let mut sessions = registry(&["/a", "/b", "/c"]);
    sessions.get_mut(SessionId(1)).unwrap().activity = SessionActivity::Working;
    sessions.get_mut(SessionId(1)).unwrap().attention = true;
    sessions.get_mut(SessionId(2)).unwrap().asking = true;
    sessions.get_mut(SessionId(2)).unwrap().attention = true;

    assert_eq!(sessions.attention_count(), 2);
    assert_eq!(
        sessions.activity_counts(),
        (1, 1),
        "one blocked, one busy, one idle"
    );
}

#[test]
fn only_working_sessions_request_animation_frames() {
    let mut state = crate::state::AppState::new();
    state.sessions = registry(&["/a"]);
    assert!(!state.wants_animation());
    state.sessions.get_mut(SessionId(1)).unwrap().activity = SessionActivity::Working;
    assert!(state.wants_animation());
    state.sessions.get_mut(SessionId(1)).unwrap().asking = true;
    assert!(
        !state.wants_animation(),
        "waiting for input should hold steady"
    );
}

/// Whatever a session last reported, a dead one is not waiting on anybody.
#[test]
fn an_ended_session_is_counted_as_neither_blocked_nor_busy() {
    let mut sessions = registry(&["/a"]);
    let session = sessions.get_mut(SessionId(1)).unwrap();
    session.asking = true;
    session.status = SessionStatus::Ended("exited".into());

    assert_eq!(sessions.activity_counts(), (0, 0));
}

#[test]
fn focusing_a_session_clears_its_attention_mark() {
    let mut sessions = registry(&["/a", "/b"]);
    sessions.get_mut(SessionId(2)).unwrap().attention = true;
    assert_eq!(sessions.attention_count(), 1);
    sessions.focus(SessionId(2));
    assert_eq!(sessions.focused(), Some(SessionId(2)));
    assert_eq!(sessions.attention_count(), 0);
}

#[test]
fn closing_the_shown_session_moves_focus_to_a_neighbour() {
    let mut sessions = registry(&["/a", "/b", "/c"]);
    sessions.focus(SessionId(2));
    sessions.close(SessionId(2));
    assert_eq!(sessions.focused(), Some(SessionId(3)));

    sessions.close(SessionId(3));
    assert_eq!(sessions.focused(), Some(SessionId(1)));

    sessions.close(SessionId(1));
    assert_eq!(sessions.focused(), None);
    assert!(sessions.is_empty());
}

#[test]
fn closing_a_background_session_leaves_the_shown_one_alone() {
    let mut sessions = registry(&["/a", "/b"]);
    sessions.focus(SessionId(1));
    sessions.close(SessionId(2));
    assert_eq!(sessions.focused(), Some(SessionId(1)));
}

#[test]
fn a_session_title_says_where_it_runs_and_how_it_ended() {
    let mut session = fake(1, "/a");
    session.label = "feat/x".to_string();
    session.sandboxed = true;
    assert_eq!(session.title(), "claude \u{b7} feat/x \u{b7} sandboxed");
    session.status = SessionStatus::Ended("exited".to_string());
    assert_eq!(
        session.title(),
        "claude \u{b7} feat/x \u{b7} sandboxed \u{b7} exited"
    );
}

#[test]
fn resizing_moves_the_parsed_screen_too() {
    let mut session = fake(1, "/a");
    session.resize(10, 40);
    assert_eq!(session.screen().size(), (10, 40));
}
