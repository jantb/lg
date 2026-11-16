//! A walk belongs to the repository it was opened in, and gives way with its
//! chat when another walk takes its place.

use std::path::Path;

use crate::git::guided::{Side, load_progress};
use crate::state::AppState;

use super::*;

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    dir
}

fn note(body: &str) -> GuidedNote {
    GuidedNote {
        path: "a.rs".into(),
        line: 1,
        side: Side::Right,
        body: body.into(),
        step: "a.rs#1".into(),
        quote: "+a".into(),
    }
}

fn unpushed_main() -> Source {
    Source::Unpushed {
        branch: "main".into(),
        base_ref: String::new(),
    }
}

/// The notes of a review in one repository are written there, whichever
/// repository lg has moved on to by the time they are saved.
#[test]
fn a_note_is_saved_with_the_repository_the_review_was_opened_in() {
    let (a, b) = (repository(), repository());
    let mut guided = crate::git::with_repo(a.path(), || Guided::new(unpushed_main()));
    guided.progress.notes.push(note("kept in a"));

    crate::git::with_repo(b.path(), || guided.save()).unwrap();

    let key = unpushed_main().progress_key();
    let in_a = crate::git::with_repo(a.path(), || load_progress(&key));
    let in_b = crate::git::with_repo(b.path(), || load_progress(&key));
    assert!(in_a.notes.iter().any(|n| n.body == "kept in a"), "{in_a:?}");
    assert!(in_b.notes.is_empty(), "{in_b:?}");
}

/// Worktrees of one repository review the same change, so they share notes.
#[test]
fn worktrees_of_one_repository_share_their_notes() {
    let a = repository();
    std::fs::write(a.path().join("a.rs"), "a\n").unwrap();
    git(a.path(), &["add", "a.rs"]);
    git(a.path(), &["commit", "-q", "-m", "first"]);
    let linked = tempfile::tempdir().unwrap();
    let worktree = linked.path().join("other");
    git(
        a.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            worktree.to_str().unwrap(),
        ],
    );
    let source = Source::PullRequest {
        number: 7,
        title: "Shared".into(),
        head: "other".into(),
        base: "main".into(),
        commit: String::new(),
        local: false,
    };
    let mut guided = crate::git::with_repo(&worktree, || Guided::new(source.clone()));
    guided.progress.notes.push(note("seen from both"));
    guided.save().unwrap();

    let in_main = crate::git::with_repo(a.path(), || Guided::new(source));
    assert!(
        in_main
            .progress
            .notes
            .iter()
            .any(|n| n.body == "seen from both")
    );
}

/// The same branch name in another repository is another review: it does
/// not pick up the walk, notes and reads of the one left behind.
#[test]
fn opening_the_review_in_another_repository_starts_that_repositorys_walk() {
    let (a, b) = (repository(), repository());
    let mut state = AppState::new();
    state.branch = Some(crate::preferences::base_branch());
    state.repo_root = Some(a.path().display().to_string());
    crate::git::with_repo(a.path(), || open_branch(&mut state));
    let guided = state.guided.as_mut().unwrap();
    guided.progress.notes.push(note("about a"));
    guided
        .commentary
        .insert(OVERVIEW_KEY.into(), commentary("a's plan"));

    state.repo_root = Some(b.path().display().to_string());
    crate::git::with_repo(b.path(), || open_branch(&mut state));

    let guided = state.guided.as_ref().unwrap();
    assert!(guided.progress.notes.is_empty(), "{:?}", guided.progress);
    assert!(!guided.commentary.contains_key(OVERVIEW_KEY));
}

/// The chat beside a walk is about that walk: a switch of scope leaves no
/// chat running that nothing tracks any more.
#[test]
fn switching_scope_closes_the_chat_of_the_walk_left_behind() {
    use ratatui::crossterm::event::KeyCode::Char;
    let mut state = walk_on_a_long_hunk();
    chatting(&mut state, "echo ready; sleep 30", "ready");
    let old = state.guided.as_ref().unwrap().chat.unwrap();

    press(&mut state, Char('b'));

    let still_running = state.sessions.get(old).is_some();
    state.sessions.close_all();
    assert!(!still_running, "the old walk's chat is still running");
}

/// Leaving the repository leaves its review: the walk is not carried into
/// the next repository, and its chat stops.
#[test]
fn leaving_the_repository_closes_its_review_and_chat() {
    let mut state = walk_on_a_long_hunk();
    chatting(&mut state, "echo ready; sleep 30", "ready");
    let old = state.guided.as_ref().unwrap().chat.unwrap();

    leave_repository(&mut state);

    let still_running = state.sessions.get(old).is_some();
    state.sessions.close_all();
    assert!(!still_running, "the old walk's chat is still running");
    assert!(state.guided.is_none());
    assert_ne!(state.modal, crate::state::Modal::GuidedReview);
}
