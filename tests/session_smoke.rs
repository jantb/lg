//! Manual smoke test: run the real claude binary in a session.
//!
//! Ignored by default — it needs claude installed and signed in. Run with
//! `cargo test --test session_smoke -- --ignored --nocapture`.

use lg::session::{SessionSpec, Sessions};
use std::time::{Duration, Instant};

#[test]
#[ignore = "needs the claude binary and an interactive login"]
fn a_real_claude_session_starts_and_draws_something() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sessions = Sessions::new();
    let id = sessions
        .start(
            SessionSpec {
                label: "smoke".into(),
                cwd: dir.path().to_path_buf(),
                sandboxed: false,
            },
            (30, 100),
        )
        .expect("start claude");

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut contents = String::new();
    while Instant::now() < deadline {
        sessions.pump();
        contents = sessions
            .get(id)
            .map(|session| session.screen().contents())
            .unwrap_or_default();
        if contents.trim().len() > 20 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    println!("--- claude screen ---\n{contents}\n---------------------");
    assert!(
        contents.trim().len() > 20,
        "claude drew nothing in 30s: {contents:?}"
    );
    sessions.close_all();
}

/// The sandboxed path, end to end: a repository with a terrarium profile, a
/// worktree of it, a profile derived for that worktree, and claude running
/// inside the sandbox.
#[test]
#[ignore = "needs terrarium, the claude binary and an interactive login"]
fn a_sandboxed_worktree_session_starts_inside_terrarium() {
    let repo = tempfile::tempdir().expect("tempdir");
    let run = |dir: &std::path::Path, program: &str, args: &[&str]| {
        let out = std::process::Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
        assert!(
            out.status.success(),
            "{program} {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run(repo.path(), "git", &["init", "-b", "main"]);
    run(
        repo.path(),
        "git",
        &["config", "user.email", "t@example.com"],
    );
    run(repo.path(), "git", &["config", "user.name", "T"]);
    std::fs::write(repo.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    run(repo.path(), "git", &["add", "."]);
    run(repo.path(), "git", &["commit", "-m", "init"]);
    run(repo.path(), "terrarium", &["init", "--preset", "rust"]);

    let worktree = repo.path().join(".worktrees-smoke/feat-x");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&worktree, "feat/x", "main")
    })
    .expect("add worktree");

    let (main, git_dir) = lg::git::with_repo(&worktree, || {
        (
            lg::git::main_worktree().expect("main worktree"),
            lg::git::common_git_dir().expect("git dir"),
        )
    });
    let note = lg::terrarium::ensure_profile(&worktree, &main, &git_dir).expect("profile");
    println!("profile: {note:?}");
    let profile_path = lg::terrarium::profile_path(&worktree).expect("profile path");
    println!("{}", std::fs::read_to_string(&profile_path).expect("read"));

    let mut sessions = Sessions::new();
    let id = sessions
        .start(
            SessionSpec {
                label: "feat/x".into(),
                cwd: worktree.clone(),
                sandboxed: true,
            },
            (30, 100),
        )
        .expect("start sandboxed claude");

    let deadline = Instant::now() + Duration::from_secs(40);
    let mut contents = String::new();
    while Instant::now() < deadline {
        sessions.pump();
        contents = sessions
            .get(id)
            .map(|session| session.screen().contents())
            .unwrap_or_default();
        if contents.trim().len() > 20 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("--- sandboxed claude ---\n{contents}\n------------------------");
    sessions.close_all();

    assert!(
        contents.trim().len() > 20,
        "sandboxed claude drew nothing: {contents:?}"
    );
    assert!(
        !contents.contains("no profile found"),
        "terrarium did not find the profile lg wrote: {contents}"
    );
    assert!(
        !contents.contains("Enable network proxy"),
        "a session must never land on an interactive terrarium prompt: {contents}"
    );
}

/// What the workspace view actually looks like with claude running in a
/// worktree. Prints the frame for a human to look at.
#[test]
#[ignore = "needs the claude binary and an interactive login"]
fn workspace_mode_with_a_real_session() {
    use lg::state::AppMode;
    use ratatui::backend::TestBackend;

    let repo = tempfile::tempdir().expect("tempdir");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    std::fs::write(repo.path().join("readme.md"), "hello\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "init"]);

    // Inside the temporary repository, so nothing is left behind if this test
    // stops early.
    let worktree = repo.path().join("smoke-worktrees/feat-x");
    lg::git::with_repo(repo.path(), || {
        lg::git::worktree_add(&worktree, "feat/x", "main")
    })
    .expect("add worktree");

    let mut app = lg::app::HeadlessApp::new(TestBackend::new(140, 34)).unwrap();
    app.state.repo_root = Some(repo.path().to_string_lossy().into_owned());
    app.state.workspace_root = Some(repo.path().to_string_lossy().into_owned());
    app.state.branch = Some("main".into());
    app.state.worktrees =
        lg::git::with_repo(repo.path(), lg::git::worktrees).expect("list worktrees");
    app.state.mode = AppMode::Workspace;

    let id = app
        .state
        .sessions
        .start(
            SessionSpec {
                label: "feat/x".into(),
                cwd: worktree.clone(),
                sandboxed: false,
            },
            (30, 100),
        )
        .expect("start claude");
    app.state.show_session(id);
    app.state.session_capture = true;

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        app.state.sessions.pump();
        app.render().unwrap();
        let drawn = app
            .state
            .sessions
            .get(id)
            .map(|s| s.screen().contents())
            .unwrap_or_default();
        if drawn.trim().len() > 40 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    app.render().unwrap();

    let buf = app.terminal.backend().buffer().clone();
    let mut out = String::new();
    for row in 0..buf.area.height {
        for col in 0..buf.area.width {
            out.push_str(buf[(col, row)].symbol());
        }
        out.push('\n');
    }
    println!("{out}");
    app.state.sessions.close_all();
    let _ = lg::git::with_repo(repo.path(), || lg::git::worktree_remove(&worktree, true));
}

// ── What a session looks like it is doing ─────────────────────────────────────
//
// Busy or ready is reported by claude itself, through the hooks lg starts it
// with. This is the probe for that whole chain — settings file, hook commands,
// the file they append to, and lg reading it back — because none of it can be
// checked without a real claude running a real turn.

use lg::session::{SessionActivity, SessionId};

/// Pump for `millis`, printing the activity and the status line whenever either
/// changes.
fn watch(sessions: &mut Sessions, id: SessionId, millis: u64, label: &str) -> SessionActivity {
    let deadline = Instant::now() + Duration::from_millis(millis);
    let mut last = None;
    let mut activity = SessionActivity::Idle;
    while Instant::now() < deadline {
        sessions.pump();
        if let Some(session) = sessions.get(id) {
            activity = session.activity();
            let contents = session.screen().contents();
            let status = contents
                .lines()
                .map(str::trim)
                .rfind(|l| {
                    !l.is_empty()
                        && (l.contains('\u{2026}')
                            || l.contains(" for ")
                            || l.starts_with('\u{276f}'))
                })
                .unwrap_or("")
                .chars()
                .take(78)
                .collect::<String>();
            let now = (activity, status.clone());
            if last.as_ref() != Some(&now) {
                println!("[{label:^18}] {activity:?}  | {status}");
                last = Some(now);
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    activity
}

fn submit(sessions: &mut Sessions, id: SessionId, text: &[u8]) {
    sessions.get_mut(id).unwrap().send(text);
    watch(sessions, id, 700, "typing");
    sessions.get_mut(id).unwrap().send(b"\r");
}

#[test]
#[ignore = "needs the claude binary and an interactive login"]
fn activity_tracks_a_real_claude_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("hello.txt"), "hello\n").unwrap();
    // Hooks report into the repository's git directory, so the probe needs a
    // repository — a bare directory would run claude with nothing to report to.
    let git = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir.path())
        .output()
        .expect("spawn git");
    assert!(git.status.success(), "git init failed");

    let mut sessions = Sessions::new();
    let id = sessions
        .start(
            SessionSpec {
                label: "probe".into(),
                cwd: dir.path().to_path_buf(),
                sandboxed: false,
            },
            (40, 120),
        )
        .expect("start claude");

    // The trust prompt is a real question: this must read NeedsInput.
    let asking = watch(&mut sessions, id, 8000, "startup");
    assert_eq!(
        asking,
        SessionActivity::NeedsInput,
        "the trust prompt is a question and should show red"
    );

    sessions.get_mut(id).unwrap().send(b"\r");
    let after_answer = watch(&mut sessions, id, 5000, "answered");
    assert_eq!(
        after_answer,
        SessionActivity::Idle,
        "an answered prompt must stop reading as a question"
    );

    submit(
        &mut sessions,
        id,
        b"think carefully and then write a 200 word essay about rust lifetimes",
    );

    let mut saw_working = false;
    for step in 0..12 {
        let activity = watch(&mut sessions, id, 2500, &format!("turn {step}"));
        saw_working |= activity == SessionActivity::Working;
        if saw_working && activity == SessionActivity::Idle {
            break;
        }
    }
    assert!(
        saw_working,
        "a working turn should have read as Working — check the hook settings lg \
         wrote under <git dir>/lg/sessions, and that claude ran them"
    );

    let settled = watch(&mut sessions, id, 4000, "settled");
    assert_eq!(
        settled,
        SessionActivity::Idle,
        "a finished turn should fall back to Idle"
    );

    sessions.close_all();
}
