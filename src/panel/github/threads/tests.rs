use std::cell::RefCell;
use std::sync::mpsc::Sender;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::github::{
    Checks, Mergeable, Owners, PrState, PullRequest, ReviewDecision, ReviewThread, ThreadComment,
    ThreadCount,
};
use crate::panel::github::{Mode, handle_key, handle_paste, poll};
use crate::state::{AppState, Modal};

use super::{Pending, ThreadsView, Written};

/// A write the view sent, and the line to answer it on.
struct Sent {
    id: String,
    reply: Option<String>,
    resolve: bool,
    answer: Sender<Result<Written, String>>,
}

thread_local! {
    static SENT: RefCell<Vec<Sent>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn record_write(
    _number: u64,
    id: String,
    reply: Option<String>,
    resolve: bool,
) -> Pending<Written> {
    let (answer, rx) = std::sync::mpsc::channel();
    SENT.with(|sent| {
        sent.borrow_mut().push(Sent {
            id,
            reply,
            resolve,
            answer,
        })
    });
    rx
}

fn sent() -> Vec<(String, Option<String>, bool)> {
    SENT.with(|sent| {
        sent.borrow()
            .iter()
            .map(|s| (s.id.clone(), s.reply.clone(), s.resolve))
            .collect()
    })
}

/// Answer every write sent so far, as GitHub would once it has them.
fn answer_all(number: u64, said: &str) {
    SENT.with(|sent| {
        for s in sent.borrow_mut().drain(..) {
            let _ = s.answer.send(Ok((number, said.to_string())));
        }
    });
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn pr(number: u64) -> PullRequest {
    PullRequest {
        number,
        title: format!("PR {number}"),
        author: "alice".into(),
        head: "feat".into(),
        head_owner: None,
        base: "main".into(),
        state: PrState::Open,
        draft: false,
        review: ReviewDecision::ReviewRequired,
        reviews: Vec::new(),
        checks: Checks::default(),
        mergeable: Mergeable::Yes,
        url: String::new(),
        body: String::new(),
        additions: 0,
        deletions: 0,
        changed_files: 0,
        updated_at: String::new(),
    }
}

fn thread(id: &str, path: &str, resolved: bool) -> ReviewThread {
    ReviewThread {
        id: id.into(),
        path: path.into(),
        line: Some(10),
        resolved,
        outdated: false,
        resolved_by: None,
        url: format!("https://github.com/acme/app/pull/7#{id}"),
        diff_hunk: "@@ -1,2 +1,2 @@\n-old\n+new".into(),
        comments: vec![ThreadComment {
            author: "bob".into(),
            body: format!("about {path}"),
            created_at: "2026-11-01T10:00:00Z".into(),
        }],
    }
}

/// The modal open on pull request #7, its conversations just read.
fn with_threads(threads: Vec<ReviewThread>) -> AppState {
    SENT.with(|sent| sent.borrow_mut().clear());
    let mut state = AppState::new();
    state.repo_root = Some("/work/app".into());
    state.modal = Modal::GitHub;
    state.github.prs = vec![pr(7)];
    state.github.owners = Some(Owners {
        login: "me".into(),
        orgs: Vec::new(),
    });
    handle_key(&mut state, key(KeyCode::Char('t'))).unwrap();
    arrive(&mut state, threads);
    state
}

fn arrive(state: &mut AppState, threads: Vec<ReviewThread>) {
    let (tx, rx) = std::sync::mpsc::channel();
    state.github.threads_loading = Some(rx);
    tx.send(Ok((7, threads))).unwrap();
    poll(state);
}

fn view(state: &AppState) -> &ThreadsView {
    match &state.github.mode {
        Mode::Threads(view) => view,
        mode => panic!("not on the conversations: {mode:?}"),
    }
}

fn shown(state: &AppState) -> Vec<String> {
    let view = view(state);
    view.visible()
        .into_iter()
        .map(|i| view.threads[i].id.clone())
        .collect()
}

fn selected(state: &AppState) -> Option<String> {
    view(state).selected().map(|t| t.id.clone())
}

#[test]
fn t_opens_the_conversations_of_the_selected_pull_request() {
    let state = with_threads(vec![thread("A", "src/a.rs", false)]);
    assert_eq!(view(&state).number, 7);
    assert_eq!(selected(&state).as_deref(), Some("A"));
}

#[test]
fn only_what_is_left_to_resolve_is_listed_until_asked_for_the_rest() {
    let mut state = with_threads(vec![
        thread("A", "a.rs", false),
        thread("B", "b.rs", true),
        thread("C", "c.rs", false),
    ]);
    assert_eq!(shown(&state), ["A", "C"]);
    assert_eq!(
        view(&state).count(),
        ThreadCount {
            unresolved: 2,
            total: 3
        }
    );
    assert_eq!(
        state.github.thread_counts.get(&7).copied(),
        Some(view(&state).count()),
        "the pull request list counts what was read"
    );

    handle_key(&mut state, key(KeyCode::Char('h'))).unwrap();
    assert_eq!(shown(&state), ["A", "B", "C"]);
}

#[test]
fn resolving_moves_on_to_the_next_conversation_at_once() {
    let mut state = with_threads(vec![
        thread("A", "a.rs", false),
        thread("B", "b.rs", false),
        thread("C", "c.rs", false),
    ]);

    handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    assert_eq!(sent(), [("A".to_string(), None, true)]);
    assert_eq!(shown(&state), ["B", "C"]);
    assert_eq!(selected(&state).as_deref(), Some("B"));
    assert_eq!(state.github.thread_counts[&7].unresolved, 2);

    // The next one goes without waiting for GitHub to answer the first.
    handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    assert_eq!(sent().len(), 2);
    assert_eq!(selected(&state).as_deref(), Some("C"));
    assert_eq!(state.github.thread_counts[&7].unresolved, 1);
}

#[test]
fn a_resolved_conversation_can_be_opened_again() {
    let mut state = with_threads(vec![thread("A", "a.rs", true)]);
    handle_key(&mut state, key(KeyCode::Char('h'))).unwrap();
    handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    assert_eq!(sent(), [("A".to_string(), None, false)]);
    assert!(!view(&state).threads[0].resolved);
}

#[test]
fn a_reply_shows_under_the_conversation_and_is_sent() {
    let mut state = with_threads(vec![thread("A", "a.rs", false)]);
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    for c in "done".chars() {
        handle_key(&mut state, key(KeyCode::Char(c))).unwrap();
    }
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(sent(), [("A".to_string(), Some("done".to_string()), false)]);
    let comments = &view(&state).threads[0].comments;
    assert_eq!(comments.last().map(|c| c.author.as_str()), Some("me"));
    assert_eq!(comments.last().map(|c| c.body.as_str()), Some("done"));
    assert!(view(&state).reply.is_none());
    assert_eq!(shown(&state), ["A"], "a plain reply leaves it open");
}

#[test]
fn alt_enter_replies_and_resolves_in_one_go() {
    let mut state = with_threads(vec![thread("A", "a.rs", false), thread("B", "b.rs", false)]);
    handle_key(&mut state, key(KeyCode::Char('r'))).unwrap();
    assert!(handle_paste(&mut state, "fixed in\nabc123"));
    handle_key(&mut state, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)).unwrap();

    assert_eq!(
        sent(),
        [("A".to_string(), Some("fixed in abc123".to_string()), true)]
    );
    assert_eq!(selected(&state).as_deref(), Some("B"));
}

#[test]
fn an_empty_reply_is_not_sent() {
    let mut state = with_threads(vec![thread("A", "a.rs", false)]);
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(sent().is_empty());
    assert!(view(&state).reply.is_some(), "still writing");
    handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    assert!(view(&state).reply.is_none());
    assert!(matches!(state.github.mode, Mode::Threads(_)));
}

#[test]
fn a_read_that_lands_before_the_writes_does_not_undo_them() {
    let mut state = with_threads(vec![thread("A", "a.rs", false), thread("B", "b.rs", false)]);
    handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();

    // GitHub's older answer, from before the resolution reached it.
    arrive(
        &mut state,
        vec![thread("A", "a.rs", false), thread("B", "b.rs", false)],
    );
    assert_eq!(shown(&state), ["B"]);

    answer_all(7, "resolved");
    poll(&mut state);
    assert!(state.github.thread_writes.is_empty());
    assert!(
        state.github.threads_loading.is_some(),
        "read again once every write is in"
    );
    let status = state
        .status
        .as_ref()
        .map(|s| s.text.clone())
        .unwrap_or_default();
    assert!(status.contains("1 left to resolve"), "{status}");
}

#[test]
fn escape_goes_back_to_the_pull_requests() {
    let mut state = with_threads(vec![thread("A", "a.rs", false)]);
    handle_key(&mut state, key(KeyCode::Esc)).unwrap();
    assert_eq!(state.github.mode, Mode::Browse);
    assert_eq!(state.modal, Modal::GitHub);
}

/// The conversations of #7, with its branch checked out in a checkout that
/// has `files` in it.
fn checked_out(files: &[&str], threads: Vec<ReviewThread>) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    for file in files {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "fn main() {}\n").unwrap();
    }
    let mut state = with_threads(threads);
    state.repo_root = Some(dir.path().to_string_lossy().into_owned());
    state.branch = Some("feat".into());
    (dir, state)
}

#[test]
fn e_opens_the_code_of_the_conversation_at_its_line() {
    let mut a = thread("A", "src/a.rs", false);
    a.line = Some(42);
    let (_dir, mut state) = checked_out(&["src/a.rs"], vec![a]);
    handle_key(&mut state, key(KeyCode::Char('e'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(crate::state::PendingAction::EditFile {
            path: "src/a.rs".into(),
            line: 42,
        })
    );
    assert_eq!(view(&state).number, 7, "the conversations stay open");
}

#[test]
fn i_opens_the_code_of_the_conversation_in_the_ide() {
    let (_dir, mut state) = checked_out(&["src/a.rs"], vec![thread("A", "src/a.rs", false)]);
    handle_key(&mut state, key(KeyCode::Char('i'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(crate::state::PendingAction::OpenFileAt {
            path: "src/a.rs".into(),
            line: 10,
        })
    );
}

#[test]
fn the_code_is_not_opened_when_another_branch_is_checked_out() {
    let (_dir, mut state) = checked_out(&["src/a.rs"], vec![thread("A", "src/a.rs", false)]);
    state.branch = Some("main".into());
    handle_key(&mut state, key(KeyCode::Char('e'))).unwrap();
    assert_eq!(state.pending_action, None);
    let status = state
        .status
        .as_ref()
        .map(|s| s.text.as_str())
        .unwrap_or_default();
    assert!(status.contains("not checked out"), "{status}");
}

#[test]
fn a_file_gone_from_the_checkout_is_not_opened() {
    let (_dir, mut state) = checked_out(&[], vec![thread("A", "src/gone.rs", false)]);
    handle_key(&mut state, key(KeyCode::Char('e'))).unwrap();
    assert_eq!(state.pending_action, None);
    let status = state
        .status
        .as_ref()
        .map(|s| s.text.as_str())
        .unwrap_or_default();
    assert!(status.contains("src/gone.rs"), "{status}");
}

#[test]
fn shift_c_starts_a_claude_chat_on_the_selected_conversation() {
    let (_dir, mut state) = checked_out(&["src/a.rs"], vec![thread("A", "src/a.rs", false)]);
    handle_key(&mut state, key(KeyCode::Char('C'))).unwrap();
    assert_eq!(
        state.pending_action,
        Some(crate::state::PendingAction::ThreadsChat)
    );
}

#[test]
fn no_chat_is_started_where_claude_cannot_read_the_pull_requests_code() {
    let (_dir, mut state) = checked_out(&["src/a.rs"], vec![thread("A", "src/a.rs", false)]);
    state.branch = Some("main".into());
    handle_key(&mut state, key(KeyCode::Char('C'))).unwrap();
    assert_eq!(state.pending_action, None);
    let status = state
        .status
        .as_ref()
        .map(|s| s.text.as_str())
        .unwrap_or_default();
    assert!(status.contains("not checked out"), "{status}");
}

#[test]
fn claude_is_handed_where_the_conversation_is_its_diff_and_every_comment() {
    let mut a = thread("A", "src/a.rs", false);
    a.outdated = true;
    a.comments.push(ThreadComment {
        author: "me".into(),
        body: "it is on purpose".into(),
        created_at: String::new(),
    });
    let brief = super::chat::brief(&a);
    assert!(
        brief.starts_with("Conversation on src/a.rs:10 (outdated"),
        "{brief}"
    );
    assert!(brief.contains("+new"), "{brief}");
    assert!(brief.contains("bob wrote:\nabout src/a.rs"), "{brief}");
    assert!(brief.contains("me wrote:\nit is on purpose"), "{brief}");
}

#[test]
fn without_a_chat_nothing_takes_the_keyboard_from_the_conversations() {
    let mut state = with_threads(vec![thread("A", "src/a.rs", false)]);
    assert!(!crate::panel::github::threads_chat_has_keyboard(&state));
    handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    assert_eq!(selected(&state).as_deref(), Some("A"));
}

mod mouse {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    use super::*;
    use crate::panel::github::draw::{HeaderClick, header_click, thread_panes};
    use crate::panel::github::{Tab, handle_mouse};

    const AREA: Rect = Rect::new(0, 0, 160, 40);

    fn at(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn three() -> AppState {
        with_threads(vec![
            thread("A", "a.rs", false),
            thread("B", "b.rs", false),
            thread("C", "c.rs", false),
        ])
    }

    #[test]
    fn a_click_on_a_conversation_selects_it() {
        let mut state = three();
        let list = thread_panes(AREA, false).unwrap().list;
        let click = MouseEventKind::Down(MouseButton::Left);
        handle_mouse(&mut state, AREA, &at(click, list.x + 2, list.y + 2));
        assert_eq!(selected(&state).as_deref(), Some("C"));
        handle_mouse(&mut state, AREA, &at(click, list.x + 2, list.y + 9));
        assert_eq!(selected(&state).as_deref(), Some("C"), "below the last row");
    }

    #[test]
    fn the_wheel_moves_through_the_list_and_scrolls_the_conversation() {
        let mut state = three();
        let panes = thread_panes(AREA, false).unwrap();
        let (list, talk) = (panes.list, panes.conversation);
        handle_mouse(
            &mut state,
            AREA,
            &at(MouseEventKind::ScrollDown, list.x + 1, list.y),
        );
        assert_eq!(selected(&state).as_deref(), Some("B"));
        handle_mouse(
            &mut state,
            AREA,
            &at(MouseEventKind::ScrollDown, talk.x + 1, talk.y + 1),
        );
        assert_eq!(view(&state).scroll, 3);
        assert_eq!(selected(&state).as_deref(), Some("B"));
        handle_mouse(
            &mut state,
            AREA,
            &at(MouseEventKind::ScrollUp, talk.x + 1, talk.y + 1),
        );
        assert_eq!(view(&state).scroll, 0);
    }

    #[test]
    fn a_tab_in_the_header_leaves_the_conversations_for_it() {
        let mut state = three();
        let (column, row) = (0..AREA.height)
            .flat_map(|row| (0..AREA.width).map(move |column| (column, row)))
            .find(|&(column, row)| {
                header_click(&state, AREA, column, row) == Some(HeaderClick::Tab(Tab::Inbox))
            })
            .expect("the inbox tab is in the header");
        handle_mouse(
            &mut state,
            AREA,
            &at(MouseEventKind::Down(MouseButton::Left), column, row),
        );
        assert_eq!(state.github.mode, Mode::Browse);
        assert_eq!(state.github.tab, Tab::Inbox);
    }

    #[test]
    fn a_drag_over_the_conversation_selects_its_text() {
        let mut state = three();
        let talk = thread_panes(AREA, false).unwrap().conversation;
        let (x, y) = (talk.x + 1, talk.y);
        handle_mouse(
            &mut state,
            AREA,
            &at(MouseEventKind::Down(MouseButton::Left), x, y),
        );
        handle_mouse(
            &mut state,
            AREA,
            &at(MouseEventKind::Drag(MouseButton::Left), x + 6, y + 1),
        );
        assert!(state.selection.as_ref().is_some_and(|s| s.dragging));
        handle_mouse(
            &mut state,
            AREA,
            &at(MouseEventKind::Up(MouseButton::Left), x + 6, y + 1),
        );
        assert!(state.selection.is_some(), "a drag let go is kept to copy");
    }
}
