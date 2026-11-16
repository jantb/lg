//! A note stays with the line it quotes when the code around it moves.

use crate::git::guided::Side;
use crate::state::{AppState, GitHubAction, PendingAction};

use super::*;

const NOTE: &str = "why keep the old name";

/// `walk_with_findings` with five lines added above `+new one`, which moves
/// from line 2 to line 7 and leaves line 2 to the first of them.
fn lines_added_above(guided: &Guided) -> Loaded {
    let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
                @@ -1,3 +1,8 @@\n keep\n+i1\n+i2\n+i3\n+i4\n+i5\n-old one\n+new one\n tail\n\
                @@ -20,3 +25,3 @@\n keep\n-old two\n+new two\n keep\n\
                diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
    Loaded {
        source: guided.source.clone(),
        steps: crate::git::guided::parse_files(diff),
        overview: Overview {
            text: guided.overview.clone(),
            brief: String::new(),
        },
    }
}

fn with_a_note_on_new_one(mut state: AppState) -> AppState {
    let guided = state.guided.as_mut().unwrap();
    guided.progress.notes = vec![GuidedNote {
        path: "a.rs".into(),
        line: 2,
        side: Side::Right,
        body: NOTE.into(),
        step: guided.steps[0].key(),
        quote: "+new one".into(),
    }];
    state
}

fn cursor_on(state: &mut AppState, text: &str) {
    let guided = state.guided.as_mut().unwrap();
    guided.cursor = guided.steps[0]
        .lines
        .iter()
        .position(|line| line.text == text)
        .unwrap();
}

fn row_with(rows: &[String], text: &str) -> Option<usize> {
    rows.iter().position(|row| row.contains(text))
}

#[test]
fn a_note_follows_its_line_when_lines_are_added_above_it() {
    let mut state = with_a_note_on_new_one(walk_with_findings());
    let guided = state.guided.as_mut().unwrap();
    let loaded = lines_added_above(guided);
    super::load::apply_loaded(guided, loaded, false);

    cursor_on(&mut state, "+i1");
    assert_eq!(
        state.guided.as_ref().unwrap().note_at_cursor(),
        None,
        "the line that took its number is not the note's"
    );
    cursor_on(&mut state, "+new one");
    assert_eq!(state.guided.as_ref().unwrap().note_at_cursor(), Some(0));

    let rows = screen(&state);
    let note = row_with(&rows, NOTE).expect("the note is drawn");
    let code = row_with(&rows, "new one").unwrap();
    assert!(note > code, "drawn under its own line: {rows:#?}");
}

#[test]
fn a_note_whose_line_is_gone_is_shown_as_moved_not_on_another_line() {
    let mut state = with_a_note_on_new_one(walk_with_findings());
    let guided = state.guided.as_mut().unwrap();
    let mut loaded = lines_added_above(guided);
    loaded.steps[0].lines.retain(|line| line.text != "+new one");
    super::load::apply_loaded(guided, loaded, false);

    cursor_on(&mut state, "+i1");
    assert_eq!(state.guided.as_ref().unwrap().note_at_cursor(), None);

    let rows = screen(&state);
    let note = row_with(&rows, NOTE).expect("the note is still in the file");
    assert!(rows[note].contains("moved"), "{}", rows[note]);
}

fn submitted(state: &AppState) -> (String, Vec<crate::github::LineComment>) {
    match &state.pending_action {
        Some(PendingAction::GitHub(GitHubAction::SubmitReview { body, comments, .. })) => {
            (body.clone(), comments.clone())
        }
        other => panic!("no review submitted: {other:?}"),
    }
}

fn pull_request_walk_after(change: impl FnOnce(&mut Loaded)) -> AppState {
    let mut state = with_a_note_on_new_one(walk_with_findings());
    let guided = state.guided.as_mut().unwrap();
    guided.source = Source::PullRequest {
        number: 7,
        title: "Rename".into(),
        head: "rename".into(),
        base: "main".into(),
        commit: "abc".into(),
        local: false,
    };
    let mut loaded = lines_added_above(guided);
    change(&mut loaded);
    super::load::apply_loaded(guided, loaded, false);
    guided.mode = Mode::Summary {
        event: 0,
        body: crate::panel::text_input::TextInput::with_text("Looks close."),
    };
    state
}

#[test]
fn a_submitted_note_is_posted_on_the_line_it_quotes() {
    let mut state = pull_request_walk_after(|_| {});

    super::notes::submit(&mut state);

    let (_, comments) = submitted(&state);
    let posted = comments.iter().find(|c| c.body == NOTE).unwrap();
    assert_eq!((posted.path.as_str(), posted.line), ("a.rs", 7));
}

#[test]
fn a_submitted_note_whose_line_is_gone_goes_in_the_body_not_on_another_line() {
    let mut state = pull_request_walk_after(|loaded| {
        loaded.steps[0].lines.retain(|line| line.text != "+new one");
    });

    super::notes::submit(&mut state);

    let (body, comments) = submitted(&state);
    assert!(comments.iter().all(|c| c.body != NOTE), "{comments:?}");
    assert!(body.contains(NOTE), "{body}");
}
