use super::common::*;
use lg::session::{SessionKind, SessionSpec};
use lg::state::ReviewAgentJob;
use lg::term::Spawn;
use std::time::{Duration, Instant};

fn review_with_agent_node() -> AssistedReview {
    AssistedReview {
        report: "report".into(),
        nodes: vec![
            ReviewNode {
                id: "checklist".into(),
                parent: None,
                depth: 0,
                title: "Review checklist".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
            ReviewNode {
                id: lg::git::REVIEW_AGENT_NODE_ID.into(),
                parent: Some("checklist".into()),
                depth: 1,
                title: "Agent review".into(),
                body: Vec::new(),
                context: Vec::new(),
            },
        ],
    }
}

/// A "claude" that writes its findings and leaves, standing in for a session
/// so the test controls what gets written and when.
fn review_session(
    app: &mut lg::app::HeadlessApp<TestBackend>,
    script: &str,
) -> lg::session::SessionId {
    let spawn = Spawn {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: std::env::temp_dir(),
        env: vec![("TERM".into(), "xterm-256color".into())],
        env_remove: Vec::new(),
    };
    let spec = SessionSpec {
        label: "review · feat/x".into(),
        cwd: std::env::temp_dir(),
        sandboxed: false,
        kind: SessionKind::Claude,
        prompt: None,
    };
    app.state
        .sessions
        .start_with(spec, &spawn, (24, 80))
        .expect("start session")
}

fn poll_until(
    app: &mut lg::app::HeadlessApp<TestBackend>,
    mut done: impl FnMut(&lg::app::HeadlessApp<TestBackend>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the agent review"
        );
        app.state.sessions.pump();
        app.drain_review_agent();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn findings_the_session_writes_appear_under_the_agent_review_row() {
    let dir = tempfile::tempdir().unwrap();
    let findings = dir.path().join("review-findings.md");
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.review = Some(review_with_agent_node());
    let session = review_session(
        &mut app,
        &format!(
            "printf '## Verdict\\n- approve\\n## Findings\\n- src/lib.rs:3 fine\\n' > '{}'; sleep 0.3",
            findings.display()
        ),
    );
    app.state.review_agent_job = Some(ReviewAgentJob {
        session,
        findings: findings.clone(),
        seen: String::new(),
    });

    poll_until(&mut app, |app| {
        app.state
            .review_assists
            .get(lg::git::REVIEW_AGENT_NODE_ID)
            .is_some_and(|text| text.contains("src/lib.rs:3 fine"))
    });
    assert!(
        app.state.review_agent_job.is_some(),
        "the job outlives the first findings while the session is still running"
    );

    poll_until(&mut app, |app| app.state.review_agent_job.is_none());
    let text = &app.state.review_assists[lg::git::REVIEW_AGENT_NODE_ID];
    assert!(text.contains("## Verdict"), "{text}");
    assert!(
        app.state
            .status
            .as_ref()
            .is_some_and(|m| m.text.contains("finished")),
        "{:?}",
        app.state.status.as_ref().map(|m| m.text.clone())
    );
}

#[test]
fn a_session_that_leaves_without_writing_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let findings = dir.path().join("review-findings.md");
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(120, 30)).unwrap();
    app.state.review = Some(review_with_agent_node());
    let session = review_session(&mut app, "exit 0");
    app.state.review_agent_job = Some(ReviewAgentJob {
        session,
        findings: findings.clone(),
        seen: String::new(),
    });

    poll_until(&mut app, |app| app.state.review_agent_job.is_none());

    let text = &app.state.review_assists[lg::git::REVIEW_AGENT_NODE_ID];
    assert!(text.contains("without writing"), "{text}");
    assert!(text.contains(&findings.display().to_string()), "{text}");
}

#[test]
fn a_in_review_mode_asks_for_an_agent_review() {
    // Wide enough for the whole review footer; a narrow one is cut short.
    let mut app = lg::app::HeadlessApp::new(TestBackend::new(240, 30)).unwrap();
    app.state.focus = Pane::Main;
    app.state.diff_source = lg::state::DiffSource::Review;
    app.state.review = Some(review_with_agent_node());

    app.render().unwrap();
    let footer = buffer_text(&app);
    assert!(footer.contains("A agent"), "{footer}");

    app.state.pending_action = None;
    lg::panel::main::handle_key(&mut app.state, key(KeyCode::Char('A'))).unwrap();
    assert_eq!(app.state.pending_action, Some(PendingAction::ReviewAgent));
}
