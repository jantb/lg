use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::github::{Checks, Mergeable, NewPullRequest, PrState, ReviewDecision, Verdict};
use crate::state::{AppState, GitHubAction, Modal, PendingAction};

use super::browse::select_pr;
use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_text(state: &mut AppState, text: &str) {
    for c in text.chars() {
        handle_key(state, key(KeyCode::Char(c))).unwrap();
    }
}

fn pr(number: u64, head: &str) -> PullRequest {
    PullRequest {
        number,
        title: format!("PR {number}"),
        author: "alice".into(),
        head: head.into(),
        head_owner: None,
        base: "main".into(),
        state: PrState::Open,
        draft: false,
        review: ReviewDecision::ReviewRequired,
        reviews: Vec::new(),
        checks: Checks::default(),
        mergeable: Mergeable::Yes,
        url: format!("https://github.com/acme/app/pull/{number}"),
        body: String::new(),
        additions: 0,
        deletions: 0,
        changed_files: 0,
        updated_at: String::new(),
    }
}

fn repo(name: &str, description: &str) -> Repository {
    Repository {
        name: name.into(),
        description: description.into(),
        private: false,
        fork: false,
        archived: false,
    }
}

/// The modal open on the pull requests of a checkout, as if the list had
/// just come back from GitHub.
fn with_prs(prs: Vec<PullRequest>) -> AppState {
    let mut state = AppState::new();
    state.repo_root = Some("/work/app".into());
    state.branch = Some("feat/login".into());
    state.modal = Modal::GitHub;
    state.github.prs = prs;
    state.github.repo = Some(RepoInfo {
        name: "acme/app".into(),
        url: "https://github.com/acme/app".into(),
        default_branch: "main".into(),
        merge_methods: vec![MergeMethod::Squash, MergeMethod::Rebase],
        preferred_merge: MergeMethod::Squash,
    });
    state
}

fn github_action(state: &AppState) -> Option<GitHubAction> {
    match &state.pending_action {
        Some(PendingAction::GitHub(action)) => Some(action.clone()),
        _ => None,
    }
}

#[test]
fn enter_checks_the_selected_pull_request_out_here() {
    let mut state = with_prs(vec![pr(1, "a"), pr(2, "b")]);
    handle_key(&mut state, key(KeyCode::Char('j'))).unwrap();
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Checkout { number: 2 })
    );
    assert_eq!(
        state.modal,
        Modal::None,
        "the checkout is what to look at next"
    );
}

#[test]
fn a_pull_request_can_be_checked_out_into_its_own_worktree() {
    let mut state = with_prs(vec![pr(9, "feat/x")]);
    handle_key(&mut state, key(KeyCode::Char('w'))).unwrap();

    assert_eq!(
        github_action(&state),
        Some(GitHubAction::CheckoutWorktree {
            number: 9,
            path: "/work/app.worktrees/feat-x".into(),
        })
    );
}

#[test]
fn approving_goes_out_with_the_words_typed_and_keeps_the_list_open() {
    let mut state = with_prs(vec![pr(5, "a")]);
    handle_key(&mut state, key(KeyCode::Char('a'))).unwrap();
    assert!(
        github_action(&state).is_none(),
        "nothing is sent before Enter"
    );
    type_text(&mut state, "lgtm");
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Review {
            number: 5,
            verdict: Verdict::Approve,
            body: "lgtm".into(),
        })
    );
    assert_eq!(state.modal, Modal::GitHub);
    assert_eq!(state.github.mode, Mode::Browse);
}

#[test]
fn an_approval_needs_no_words() {
    let mut state = with_prs(vec![pr(5, "a")]);
    handle_key(&mut state, key(KeyCode::Char('a'))).unwrap();
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert!(matches!(
        github_action(&state),
        Some(GitHubAction::Review {
            verdict: Verdict::Approve,
            ..
        })
    ));
}

#[test]
fn requesting_changes_waits_until_it_says_what_to_change() {
    let mut state = with_prs(vec![pr(5, "a")]);
    handle_key(&mut state, key(KeyCode::Char('x'))).unwrap();
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert!(github_action(&state).is_none());

    type_text(&mut state, "needs tests");
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Review {
            number: 5,
            verdict: Verdict::RequestChanges,
            body: "needs tests".into(),
        })
    );
}

#[test]
fn escape_abandons_a_comment_without_sending_it() {
    let mut state = with_prs(vec![pr(5, "a")]);
    handle_key(&mut state, key(KeyCode::Char('c'))).unwrap();
    type_text(&mut state, "half a thought");
    handle_key(&mut state, key(KeyCode::Esc)).unwrap();

    assert!(github_action(&state).is_none());
    assert_eq!(state.github.mode, Mode::Browse);
    assert_eq!(
        state.modal,
        Modal::GitHub,
        "Esc backs out of the comment only"
    );
}

#[test]
fn merging_offers_only_what_the_repository_allows() {
    let mut state = with_prs(vec![pr(3, "a")]);
    handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();
    // Step the method past the end of the list and back round to the start.
    handle_key(&mut state, key(KeyCode::Right)).unwrap();
    handle_key(&mut state, key(KeyCode::Right)).unwrap();
    // Keep the branch, and merge.
    handle_key(&mut state, key(KeyCode::Down)).unwrap();
    handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Merge {
            number: 3,
            method: MergeMethod::Squash,
            delete_branch: false,
            auto: false,
        })
    );
}

#[test]
fn a_merged_pull_request_is_not_offered_for_merging_again() {
    let mut merged = pr(3, "a");
    merged.state = PrState::Merged;
    let mut state = with_prs(vec![merged]);
    handle_key(&mut state, key(KeyCode::Char('m'))).unwrap();

    assert_eq!(state.github.mode, Mode::Browse);
    assert!(github_action(&state).is_none());
    assert!(state.status.is_some(), "the user is told why");
}

#[test]
fn closing_asks_first_and_a_closed_one_reopens() {
    let mut state = with_prs(vec![pr(4, "a")]);
    handle_key(&mut state, key(KeyCode::Char('X'))).unwrap();
    assert!(github_action(&state).is_none());
    handle_key(&mut state, key(KeyCode::Char('y'))).unwrap();
    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Close { number: 4 })
    );

    let mut closed = pr(4, "a");
    closed.state = PrState::Closed;
    let mut state = with_prs(vec![closed]);
    handle_key(&mut state, key(KeyCode::Char('X'))).unwrap();
    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Reopen { number: 4 })
    );
}

#[test]
fn a_new_pull_request_goes_from_the_current_branch_to_the_default_one() {
    let mut state = with_prs(Vec::new());
    handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();
    let Mode::Create(form) = &mut state.github.mode else {
        panic!("the form opens");
    };
    form.title.clear();
    type_text(&mut state, "Add login");
    // Title, Base, Body.
    handle_key(&mut state, key(KeyCode::Tab)).unwrap();
    handle_key(&mut state, key(KeyCode::Tab)).unwrap();
    if let Mode::Create(form) = &mut state.github.mode {
        form.body.clear();
    }
    type_text(&mut state, "first");
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    type_text(&mut state, "second");
    handle_key(&mut state, key(KeyCode::Tab)).unwrap();
    handle_key(&mut state, key(KeyCode::Char(' '))).unwrap();
    handle_key(&mut state, ctrl('s')).unwrap();

    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Create(NewPullRequest {
            branch: "feat/login".into(),
            base: "main".into(),
            title: "Add login".into(),
            body: "first\nsecond".into(),
            draft: true,
        }))
    );
}

#[test]
fn a_branch_that_already_has_a_pull_request_goes_to_it_instead() {
    let mut state = with_prs(vec![pr(1, "other"), pr(8, "feat/login")]);
    handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();

    assert_eq!(state.github.mode, Mode::Browse);
    assert_eq!(selected_pr(&state).map(|pr| pr.number), Some(8));
}

#[test]
fn no_pull_request_is_opened_from_the_base_branch() {
    let mut state = with_prs(Vec::new());
    state.branch = Some("main".into());
    handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();

    assert_eq!(state.github.mode, Mode::Browse);
}

#[test]
fn typing_narrows_the_repositories_and_enter_clones_into_the_workspace() {
    let mut state = AppState::new();
    state.workspace_root = Some("/nonexistent-lg-workspace".into());
    state.modal = Modal::GitHub;
    state.github.tab = Tab::Repositories;
    state.github.repos = vec![
        repo("me/lg", "a git tui"),
        repo("me/notes", "plain text"),
        repo("me/blog", ""),
    ];

    type_text(&mut state, "text");
    assert_eq!(
        visible_repos(&state.github)
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["me/notes"],
        "descriptions are searched too"
    );
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Clone {
            repo: "me/notes".into(),
            dir: "/nonexistent-lg-workspace/notes".into(),
        })
    );
}

#[test]
fn a_typed_owner_and_name_can_be_cloned_without_being_listed() {
    let mut state = AppState::new();
    state.repo_root = Some("/nonexistent-lg-parent/current".into());
    state.modal = Modal::GitHub;
    state.github.tab = Tab::Repositories;

    type_text(&mut state, "rust-lang/rust");
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Clone {
            repo: "rust-lang/rust".into(),
            dir: "/nonexistent-lg-parent/rust".into(),
        }),
        "a clone goes beside the checkout, never inside it"
    );
}

/// The repositories tab listing the user's own, with the organizations
/// they belong to already read.
fn with_owners(orgs: &[&str]) -> AppState {
    let mut state = AppState::new();
    state.workspace_root = Some("/nonexistent-lg-workspace".into());
    state.modal = Modal::GitHub;
    state.github.tab = Tab::Repositories;
    state.github.owners = Some(Owners {
        login: "me".into(),
        orgs: orgs.iter().map(|org| org.to_string()).collect(),
    });
    state.github.repos = vec![repo("me/lg", "")];
    state
}

#[test]
fn the_arrows_step_through_the_users_organizations_and_back_to_the_user() {
    let mut state = with_owners(&["acme", "zeta"]);

    handle_key(&mut state, key(KeyCode::Right)).unwrap();
    assert_eq!(state.github.owner.as_deref(), Some("acme"));
    handle_key(&mut state, key(KeyCode::Right)).unwrap();
    assert_eq!(state.github.owner.as_deref(), Some("zeta"));
    handle_key(&mut state, key(KeyCode::Right)).unwrap();
    assert_eq!(
        state.github.owner, None,
        "past the last, the user's own again"
    );
    handle_key(&mut state, key(KeyCode::Left)).unwrap();
    assert_eq!(state.github.owner.as_deref(), Some("zeta"));
}

#[test]
fn another_owners_list_never_clones_from_the_last_one() {
    let mut state = with_owners(&["acme"]);

    handle_key(&mut state, key(KeyCode::Right)).unwrap();
    assert!(
        visible_repos(&state.github).is_empty(),
        "me/lg is not acme's to offer"
    );

    state.github.repos = vec![repo("acme/api", "")];
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();
    assert_eq!(
        github_action(&state),
        Some(GitHubAction::Clone {
            repo: "acme/api".into(),
            dir: "/nonexistent-lg-workspace/api".into(),
        })
    );
}

#[test]
fn without_organizations_the_arrows_say_why_nothing_changed() {
    let mut state = with_owners(&[]);

    handle_key(&mut state, key(KeyCode::Right)).unwrap();

    assert_eq!(state.github.owner, None);
    assert_eq!(visible_repos(&state.github).len(), 1, "the list is kept");
    assert!(state.status.is_some());
}

#[test]
fn organizations_that_could_not_be_read_leave_the_users_list_standing() {
    let mut state = with_owners(&[]);
    state.github.owners = None;
    let (tx, rx) = std::sync::mpsc::channel();
    state.github.owners_loading = Some(rx);
    tx.send(Err("missing read:org scope".into())).unwrap();

    poll(&mut state);

    assert!(!state.github.loading());
    assert!(state.status.as_ref().is_some_and(|status| status.is_error));
    assert_eq!(
        visible_repos(&state.github)
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["me/lg"]
    );
}

#[test]
fn a_repository_already_cloned_is_opened_rather_than_cloned_again() {
    let workspace = tempfile::tempdir().unwrap();
    let existing = workspace.path().join("lg");
    std::fs::create_dir(&existing).unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&existing)
        .status()
        .unwrap();
    assert!(init.success());

    let mut state = AppState::new();
    state.workspace_root = Some(workspace.path().to_string_lossy().into_owned());
    state.modal = Modal::GitHub;
    state.github.tab = Tab::Repositories;
    state.github.repos = vec![repo("me/lg", "")];
    handle_key(&mut state, key(KeyCode::Enter)).unwrap();

    assert_eq!(
        state.pending_action,
        Some(PendingAction::SwitchRepository {
            target: crate::state::RepoTarget::Path(existing),
        })
    );
}

#[test]
fn a_pasted_body_keeps_its_lines_and_a_pasted_title_does_not() {
    let mut state = with_prs(Vec::new());
    handle_key(&mut state, key(KeyCode::Char('n'))).unwrap();
    if let Mode::Create(form) = &mut state.github.mode {
        form.title.clear();
        form.body.clear();
    }
    assert!(handle_paste(&mut state, "Fix\nlogin"));
    handle_key(&mut state, key(KeyCode::Tab)).unwrap();
    handle_key(&mut state, key(KeyCode::Tab)).unwrap();
    assert!(handle_paste(&mut state, "line one\nline two"));

    let Mode::Create(form) = &state.github.mode else {
        panic!("the form is still open");
    };
    assert_eq!(form.title, "Fix login");
    assert_eq!(form.body, "line one\nline two");
}

#[test]
fn a_finished_list_replaces_what_was_shown() {
    let mut state = with_prs(vec![pr(1, "a"), pr(2, "b"), pr(3, "c")]);
    state.github.pr_idx = 2;
    let (tx, rx) = std::sync::mpsc::channel();
    state.github.prs_loading = Some(rx);
    tx.send(Ok((None, vec![pr(10, "x")]))).unwrap();

    poll(&mut state);

    assert!(!state.github.loading());
    assert_eq!(selected_pr(&state).map(|pr| pr.number), Some(10));
    assert!(state.github.repo.is_some(), "repository details are kept");
}

#[test]
fn a_list_whose_worker_died_says_so_and_stops_loading() {
    let mut state = with_prs(Vec::new());
    let (tx, rx) = std::sync::mpsc::channel();
    state.github.prs_loading = Some(rx);
    drop(tx);

    poll(&mut state);

    assert!(!state.github.loading());
    assert!(state.github.prs_error.is_some());
}

#[test]
fn a_failed_list_says_why() {
    let mut state = with_prs(Vec::new());
    let (tx, rx) = std::sync::mpsc::channel();
    state.github.prs_loading = Some(rx);
    tx.send(Err("gh auth login required".into())).unwrap();

    poll(&mut state);

    assert_eq!(
        state.github.prs_error.as_deref(),
        Some("gh auth login required")
    );
}

fn by(author: &str, number: u64) -> PullRequest {
    PullRequest {
        author: author.into(),
        ..pr(number, &format!("branch-{number}"))
    }
}

fn signed_in_as(state: &mut AppState, login: &str) {
    state.github.owners = Some(Owners {
        login: login.into(),
        orgs: Vec::new(),
    });
}

fn arrive(state: &mut AppState, prs: Vec<PullRequest>) {
    let (tx, rx) = std::sync::mpsc::channel();
    state.github.prs_loading = Some(rx);
    tx.send(Ok((None, prs))).unwrap();
    poll(state);
}

fn numbers(state: &AppState) -> Vec<u64> {
    state.github.prs.iter().map(|pr| pr.number).collect()
}

#[test]
fn the_users_own_pull_requests_come_first() {
    let mut state = with_prs(Vec::new());
    signed_in_as(&mut state, "me");
    arrive(
        &mut state,
        vec![by("alice", 1), by("me", 2), by("bob", 3), by("me", 4)],
    );

    let order = numbers(&state);
    let last_mine = order.iter().rposition(|n| [2, 4].contains(n)).unwrap();
    let first_other = order.iter().position(|n| [1, 3].contains(n)).unwrap();
    assert!(last_mine < first_other, "{order:?}");
}

#[test]
fn learning_who_the_user_is_moves_their_pull_requests_up_and_keeps_the_selection() {
    let mut state = with_prs(Vec::new());
    arrive(&mut state, vec![by("alice", 1), by("bob", 3), by("me", 2)]);
    select_pr(&mut state, 1);
    let (tx, rx) = std::sync::mpsc::channel();
    state.github.owners_loading = Some(rx);
    tx.send(Ok(Owners {
        login: "me".into(),
        orgs: Vec::new(),
    }))
    .unwrap();

    poll(&mut state);

    assert_eq!(state.github.prs[0].number, 2);
    assert_eq!(selected_pr(&state).map(|pr| pr.number), Some(3));
}

#[test]
fn asking_for_a_new_pull_request_opens_the_form_once_the_list_is_in() {
    let mut state = with_prs(Vec::new());
    state.github.create_on_load = true;

    arrive(&mut state, vec![pr(1, "other")]);

    let Mode::Create(form) = &state.github.mode else {
        panic!("the form opens");
    };
    assert_eq!(form.branch, "feat/login");
}

#[test]
fn asking_for_a_new_pull_request_on_a_branch_that_has_one_shows_it() {
    let mut state = with_prs(Vec::new());
    state.github.create_on_load = true;

    arrive(&mut state, vec![pr(1, "other"), pr(8, "feat/login")]);

    assert_eq!(state.github.mode, Mode::Browse);
    assert_eq!(selected_pr(&state).map(|pr| pr.number), Some(8));
}

#[test]
fn a_narrow_header_steps_through_the_filters_from_the_one_shown() {
    let state = with_prs(Vec::new());
    let area = ratatui::layout::Rect::new(0, 0, 90, 30);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(90, 30)).unwrap();
    terminal.draw(|frame| render(&state, area, frame)).unwrap();
    let buf = terminal.backend().buffer();
    let (column, row) = (0..buf.area.height)
        .find_map(|y| {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            let byte = line.find("showing")?;
            Some((line[..byte].chars().count() as u16, y))
        })
        .expect("only the filter shown fits");

    assert_eq!(
        draw::header_click(&state, area, column, row),
        Some(draw::HeaderClick::NextFilter)
    );
}

#[test]
fn the_header_takes_clicks_on_its_tabs_filter_and_drafts() {
    let state = with_prs(Vec::new());
    let area = ratatui::layout::Rect::new(0, 0, 120, 30);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| render(&state, area, frame)).unwrap();
    let buf = terminal.backend().buffer();
    // Where `text` starts on screen, as the user would aim at it.
    let at = |text: &str| {
        (0..buf.area.height)
            .find_map(|y| {
                let row: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                let line = row.concat();
                let byte = line.find(text)?;
                let column = line[..byte].chars().count();
                Some((column as u16, y))
            })
            .unwrap_or_else(|| panic!("{text} is on screen"))
    };
    let click = |text: &str| {
        let (column, row) = at(text);
        draw::header_click(&state, area, column + 1, row)
    };

    assert_eq!(
        click("Repositories"),
        Some(draw::HeaderClick::Tab(Tab::Repositories))
    );
    assert_eq!(click("Inbox"), Some(draw::HeaderClick::Tab(Tab::Inbox)));
    assert_eq!(
        click("my teams"),
        Some(draw::HeaderClick::Filter(PrFilter::TeamReviewRequested))
    );
    assert_eq!(
        click("to review"),
        Some(draw::HeaderClick::Filter(PrFilter::ToReview))
    );
    assert_eq!(click("(s)"), Some(draw::HeaderClick::NextFilter));
    assert_eq!(click("h hides drafts"), Some(draw::HeaderClick::Drafts));
    let (column, row) = at("h hides drafts");
    assert_eq!(draw::header_click(&state, area, column, row + 1), None);
}

#[test]
fn a_click_in_the_list_selects_the_row_it_lands_on() {
    let mut state = with_prs(vec![pr(4, "a"), pr(5, "b"), pr(6, "c")]);
    state.github.tab = Tab::PullRequests;
    let area = ratatui::layout::Rect::new(0, 0, 120, 30);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| render(&state, area, frame)).unwrap();
    let buf = terminal.backend().buffer();
    let (column, row) = (0..buf.area.height)
        .find_map(|y| {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            let byte = line.find("#6")?;
            Some((line[..byte].chars().count() as u16, y))
        })
        .expect("#6 is on screen");
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };

    handle_mouse(&mut state, area, &click);

    assert_eq!(selected_pr(&state).map(|pr| pr.number), Some(6));
}

#[test]
fn the_list_names_each_author_and_the_user_as_you() {
    let mut state = with_prs(Vec::new());
    signed_in_as(&mut state, "me");
    arrive(&mut state, vec![by("alice", 1), by("me", 2)]);
    let area = ratatui::layout::Rect::new(0, 0, 120, 30);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| render(&state, area, frame)).unwrap();
    let buf = terminal.backend().buffer();
    // The list is the first column inside the frame; the details beside
    // it name the selected pull request too.
    let rows: Vec<String> = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .filter_map(|row: String| row.split('\u{2502}').nth(1).map(str::to_string))
        .collect();

    let row = |title: &str| rows.iter().find(|row| row.contains(title)).cloned();
    assert!(
        row("PR 1").is_some_and(|row| row.contains("alice")),
        "{rows:#?}"
    );
    assert!(
        row("PR 2").is_some_and(|row| row.contains("you")),
        "{rows:#?}"
    );
}
