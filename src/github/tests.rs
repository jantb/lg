use std::path::{Path, PathBuf};

use super::gh::failure;
use super::query::search_terms;
use super::review::review_payload;
use super::*;

#[test]
fn review_requested_asks_only_for_requests_made_of_the_user_by_name() {
    assert_eq!(
        search_terms(PrFilter::ReviewRequested, false).as_deref(),
        Some("user-review-requested:@me sort:created-desc")
    );
}

#[test]
fn the_wider_review_lists_take_in_teams_and_then_everyone_else() {
    assert_eq!(
        search_terms(PrFilter::TeamReviewRequested, false).as_deref(),
        Some("review-requested:@me sort:created-desc")
    );
    assert_eq!(
        search_terms(PrFilter::ToReview, false).as_deref(),
        Some("-author:@me sort:created-desc")
    );
}

#[test]
fn hiding_drafts_searches_any_list_newest_first() {
    assert_eq!(search_terms(PrFilter::Open, false), None);
    assert_eq!(
        search_terms(PrFilter::Mine, true).as_deref(),
        Some("draft:false sort:created-desc")
    );
    assert_eq!(
        search_terms(PrFilter::ReviewRequested, true).as_deref(),
        Some("user-review-requested:@me draft:false sort:created-desc")
    );
}

#[test]
fn a_review_carries_every_inline_comment_and_always_a_body() {
    let payload = review_payload(
        "abc123",
        ReviewEvent::RequestChanges,
        "  ",
        &[LineComment {
            path: "src/a.rs".into(),
            line: 11,
            side: "LEFT",
            body: "why remove this?".into(),
        }],
    );

    assert_eq!(payload["commit_id"], "abc123");
    assert_eq!(payload["event"], "REQUEST_CHANGES");
    assert!(!payload["body"].as_str().unwrap().is_empty());
    assert_eq!(payload["comments"][0]["path"], "src/a.rs");
    assert_eq!(payload["comments"][0]["line"], 11);
    assert_eq!(payload["comments"][0]["side"], "LEFT");
}

const PR_JSON: &str = r#"[
  {
    "number": 42,
    "title": "Add login",
    "author": {"login": "alice", "is_bot": false},
    "headRefName": "feat/login",
    "headRepositoryOwner": {"login": "alice"},
    "isCrossRepository": false,
    "baseRefName": "main",
    "state": "OPEN",
    "isDraft": false,
    "reviewDecision": "CHANGES_REQUESTED",
    "mergeable": "CONFLICTING",
    "statusCheckRollup": [
      {"__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS"},
      {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED", "conclusion": "FAILURE"},
      {"__typename": "CheckRun", "name": "e2e", "status": "IN_PROGRESS", "conclusion": ""},
      {"__typename": "StatusContext", "context": "ci/legacy", "state": "ERROR"},
      {"__typename": "StatusContext", "context": "deploy", "state": "PENDING"},
      {"__typename": "CheckRun", "name": "docs", "status": "COMPLETED", "conclusion": "SKIPPED"}
    ],
    "latestReviews": [{"author": {"login": "bob"}, "state": "CHANGES_REQUESTED"}],
    "url": "https://github.com/acme/app/pull/42",
    "body": "Adds a login form.",
    "additions": 120,
    "deletions": 30,
    "changedFiles": 5,
    "updatedAt": "2026-09-20T10:00:00Z"
  },
  {
    "number": 7,
    "title": "Fix typo",
    "author": {"login": "carol"},
    "headRefName": "main",
    "headRepositoryOwner": {"login": "carol"},
    "isCrossRepository": true,
    "baseRefName": "main",
    "state": "MERGED",
    "isDraft": true,
    "reviewDecision": "",
    "mergeable": "UNKNOWN",
    "statusCheckRollup": null,
    "latestReviews": [],
    "url": "https://github.com/acme/app/pull/7",
    "body": null,
    "additions": 1,
    "deletions": 1,
    "changedFiles": 1,
    "updatedAt": "2026-09-01T10:00:00Z"
  }
]"#;

#[test]
fn a_pull_request_list_reads_back_what_the_panel_shows() {
    let prs = parse_pull_requests(PR_JSON).unwrap();
    let login = prs.iter().find(|pr| pr.number == 42).unwrap();

    assert_eq!(login.title, "Add login");
    assert_eq!(login.author, "alice");
    assert_eq!(
        (login.head.as_str(), login.base.as_str()),
        ("feat/login", "main")
    );
    assert_eq!(login.head_owner, None, "not from a fork");
    assert!(login.is_open());
    assert_eq!(login.review, ReviewDecision::ChangesRequested);
    assert_eq!(login.mergeable, Mergeable::Conflicting);
    assert_eq!(login.reviews[0].author, "bob");
    assert_eq!(login.body, "Adds a login form.");
}

/// Check runs and commit statuses report in different words; both have to
/// land in the same three buckets, and a failure has to say which check.
#[test]
fn checks_are_counted_by_outcome_whichever_way_they_report() {
    let prs = parse_pull_requests(PR_JSON).unwrap();
    let checks = &prs.iter().find(|pr| pr.number == 42).unwrap().checks;

    assert_eq!(checks.passed, 2, "success and skipped both pass");
    assert_eq!(checks.pending, 2, "in progress and pending both wait");
    let mut failed = checks.failed.clone();
    failed.sort();
    assert_eq!(failed, ["ci/legacy", "lint"]);
}

#[test]
fn missing_and_null_fields_read_as_nothing_rather_than_failing() {
    let prs = parse_pull_requests(PR_JSON).unwrap();
    let merged = prs.iter().find(|pr| pr.number == 7).unwrap();

    assert_eq!(merged.state, PrState::Merged);
    assert!(merged.draft);
    assert_eq!(merged.review, ReviewDecision::None);
    assert!(merged.checks.is_empty());
    assert_eq!(merged.body, "");
    assert_eq!(merged.head_owner.as_deref(), Some("carol"), "from a fork");
}

#[test]
fn the_merge_method_offered_first_is_one_the_repository_allows() {
    let info = parse_repo_info(
        r#"{"nameWithOwner":"acme/app","url":"https://github.com/acme/app",
            "defaultBranchRef":{"name":"trunk"},
            "mergeCommitAllowed":false,"squashMergeAllowed":true,"rebaseMergeAllowed":true,
            "deleteBranchOnMerge":true,"viewerDefaultMergeMethod":"MERGE"}"#,
    )
    .unwrap();

    assert_eq!(info.name, "acme/app");
    assert_eq!(info.default_branch, "trunk");
    assert_eq!(
        info.merge_methods,
        [MergeMethod::Squash, MergeMethod::Rebase]
    );
    assert_eq!(
        info.preferred_merge,
        MergeMethod::Squash,
        "the user's usual method is not allowed here"
    );
}

#[test]
fn the_users_usual_merge_method_is_kept_when_allowed() {
    let info = parse_repo_info(
        r#"{"nameWithOwner":"acme/app","defaultBranchRef":null,
            "mergeCommitAllowed":true,"squashMergeAllowed":true,"rebaseMergeAllowed":false,
            "viewerDefaultMergeMethod":"SQUASH"}"#,
    )
    .unwrap();

    assert_eq!(info.preferred_merge, MergeMethod::Squash);
    assert_eq!(info.default_branch, "main");
}

#[test]
fn repositories_read_back_with_their_flags() {
    let repos = parse_repositories(
        r#"[{"nameWithOwner":"me/tool","description":null,"isPrivate":true,"isFork":false,"isArchived":false},
            {"nameWithOwner":"me/fork","description":"a fork","isPrivate":false,"isFork":true,"isArchived":true}]"#,
    )
    .unwrap();

    let tool = repos.iter().find(|r| r.name == "me/tool").unwrap();
    assert!(tool.private);
    assert_eq!(tool.description, "");
    let fork = repos.iter().find(|r| r.name == "me/fork").unwrap();
    assert!(fork.fork && fork.archived);
}

#[test]
fn the_signed_in_user_comes_back_with_their_organizations() {
    let owners = parse_owners(
        r#"{"data":{"viewer":{"login":"me","organizations":{"nodes":[
            {"login":"zeta"},{"login":"Acme"},null]}}}}"#,
    )
    .unwrap();

    assert_eq!(owners.login, "me");
    assert_eq!(owners.orgs, ["Acme", "zeta"], "listed by name");
}

#[test]
fn a_user_in_no_organization_still_has_their_own_repositories() {
    let owners =
        parse_owners(r#"{"data":{"viewer":{"login":"me","organizations":{"nodes":[]}}}}"#).unwrap();

    assert_eq!(owners.login, "me");
    assert!(owners.orgs.is_empty());
}

#[test]
fn a_clone_is_named_after_the_repository_however_it_was_written() {
    let parent = Path::new("/work");
    for repo in [
        "acme/app",
        "https://github.com/acme/app",
        "https://github.com/acme/app.git",
        "git@github.com:acme/app.git",
        "app",
    ] {
        assert_eq!(
            clone_dir(parent, repo),
            Some(PathBuf::from("/work/app")),
            "{repo}"
        );
    }
    assert_eq!(clone_dir(parent, ""), None);
    assert_eq!(clone_dir(parent, "acme/.."), None);
}

#[test]
fn one_commit_speaks_for_the_pull_request() {
    let commits = vec![(
        "Add login form".to_string(),
        "With validation.\n".to_string(),
    )];
    assert_eq!(
        fill_from_commits("feat/login", &commits),
        ("Add login form".to_string(), "With validation.".to_string())
    );
}

#[test]
fn several_commits_are_listed_under_the_branch_name() {
    let commits = vec![
        ("Add form".to_string(), String::new()),
        ("Validate input".to_string(), String::new()),
    ];
    let (title, body) = fill_from_commits("feat/add-login_form", &commits);
    assert_eq!(title, "Add login form");
    assert!(body.contains("- Add form") && body.contains("- Validate input"));
}

#[test]
fn a_failed_call_says_what_gh_said_on_one_line() {
    let err = failure(
        &["pr", "merge", "42"],
        "",
        "X Pull request acme/app#42 is not mergeable\nthe base branch policy prohibits the merge\n",
    );
    let text = err.to_string();
    assert!(!text.contains('\n'), "{text}");
    assert!(text.contains("gh pr merge"), "{text}");
    assert!(text.contains("prohibits the merge"), "{text}");
}
