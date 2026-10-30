//! What is walked: the branch, what is not pushed, or a pull request.

use crate::state::{AppState, Modal, PendingAction};
use ratatui::crossterm::event::{MouseButton, MouseEventKind};

use crate::panel::guided::load::switch_scope;

use super::*;

fn opened_in_browser(state: &AppState) -> Option<&str> {
    match &state.pending_action {
        Some(PendingAction::GitHub(crate::state::GitHubAction::OpenInBrowser { url })) => Some(url),
        _ => None,
    }
}

/// A pull request being walked can be looked at on GitHub too.
#[test]
fn a_pull_request_under_review_opens_in_the_browser() {
    use crate::github::{Checks, Mergeable, PrState, PullRequest, ReviewDecision};
    let pr = PullRequest {
        number: 7,
        title: "Add a cache".into(),
        author: "alice".into(),
        head: "cache".into(),
        head_owner: None,
        base: "main".into(),
        state: PrState::Open,
        draft: false,
        review: ReviewDecision::ReviewRequired,
        reviews: Vec::new(),
        checks: Checks::default(),
        mergeable: Mergeable::Yes,
        url: "https://github.com/acme/app/pull/7".into(),
        body: String::new(),
        additions: 0,
        deletions: 0,
        changed_files: 0,
        updated_at: String::new(),
    };
    let mut guided = Guided::new(Source::PullRequest {
        number: pr.number,
        title: pr.title.clone(),
        head: pr.head.clone(),
        base: pr.base.clone(),
        commit: String::new(),
        local: false,
    });
    guided.pull_request = Some(pr);
    let mut state = AppState::new();
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('O'));

    assert_eq!(
        opened_in_browser(&state),
        Some("https://github.com/acme/app/pull/7")
    );
}

/// A walk over local changes has no page, so nothing is opened and the
/// reason is given instead.
#[test]
fn a_walk_over_local_changes_has_nothing_to_open() {
    let mut state = AppState::new();
    state.guided = Some(Box::new(Guided::new(Source::Unpushed {
        branch: "feature".into(),
        base_ref: String::new(),
    })));
    state.modal = Modal::GuidedReview;

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('O'));

    assert_eq!(opened_in_browser(&state), None);
    assert!(state.status.is_some());
}

/// A read whose worker died must end the loading state and say why,
/// rather than leave the walk waiting on it.
#[test]
fn a_read_of_the_change_that_dies_is_reported() {
    let mut guided = Guided::new(Source::Unpushed {
        branch: "feature".into(),
        base_ref: String::new(),
    });
    let (tx, rx) = std::sync::mpsc::channel();
    guided.loading = Some(rx);
    drop(tx);
    let mut state = AppState::new();
    state.guided = Some(Box::new(guided));

    poll(&mut state);

    assert!(state.guided.as_ref().unwrap().loading.is_none());
    let status = state.status.expect("the failure is reported");
    assert!(status.is_error);
    assert!(
        status.text.contains("stopped unexpectedly"),
        "{}",
        status.text
    );
}

#[test]
fn b_switches_between_the_whole_branch_and_what_is_not_pushed() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE);
    let mut state = walk_on_a_long_hunk();

    handle_key(&mut state, b).unwrap();
    assert_eq!(
        state.guided.as_ref().unwrap().source,
        Source::Unpushed {
            branch: "guided-test-branch".into(),
            base_ref: String::new(),
        }
    );

    handle_key(&mut state, b).unwrap();
    assert_eq!(
        state.guided.as_ref().unwrap().source,
        Source::Branch {
            branch: "guided-test-branch".into(),
            base_ref: crate::preferences::base_branch(),
        }
    );
}

/// On the base branch there is no branch against it to walk, so the walk
/// over what is not pushed stays put and says why.
#[test]
fn the_base_branch_has_only_what_is_not_pushed_to_walk() {
    let mut state = AppState::new();
    let branch = crate::preferences::base_branch();
    let unpushed = Source::Unpushed {
        branch,
        base_ref: String::new(),
    };
    state.guided = Some(Box::new(Guided::new(unpushed.clone())));
    state.modal = Modal::GuidedReview;

    switch_scope(&mut state);

    assert_eq!(state.guided.as_ref().unwrap().source, unpushed);
    assert!(state.status.is_some_and(|status| status.is_error));
}

#[test]
fn the_walk_says_whether_it_covers_the_branch_or_only_what_is_not_pushed() {
    let shows = |source: Source| {
        let mut state = AppState::new();
        state.guided = Some(Box::new(Guided::new(source)));
        state.modal = Modal::GuidedReview;
        let rows = screen(&state);
        (
            rows.iter().any(|row| row.contains("BRANCH")),
            rows.iter().any(|row| row.contains("NOT PUSHED")),
        )
    };

    assert_eq!(
        shows(Source::Branch {
            branch: "guided-test-scope".into(),
            base_ref: "main".into(),
        }),
        (true, false)
    );
    assert_eq!(
        shows(Source::Unpushed {
            branch: "guided-test-scope".into(),
            base_ref: "origin/guided-test-scope".into(),
        }),
        (false, true)
    );
}

#[test]
fn a_click_on_the_scope_badge_switches_scope() {
    let mut state = walk_on_a_long_hunk();
    let badge = draw::scope_badge_area(state.guided.as_deref().unwrap(), AREA).unwrap();

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        badge.x + 1,
        badge.y,
    );

    assert!(matches!(
        state.guided.as_ref().unwrap().source,
        Source::Unpushed { .. }
    ));
}
