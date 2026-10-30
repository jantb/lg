//! Reading gh's JSON answers into the types the panel shows.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{
    Checks, MergeMethod, Mergeable, Owners, PrState, PullRequest, RepoInfo, Repository, Review,
    ReviewDecision,
};

#[derive(Deserialize, Default)]
struct RawActor {
    #[serde(default)]
    login: String,
}

/// A check run carries a status and, once completed, a conclusion; a commit
/// status carries a single state. `statusCheckRollup` mixes the two.
#[derive(Deserialize)]
struct RawCheck {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize)]
struct RawReview {
    #[serde(default)]
    author: Option<RawActor>,
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequest {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author: Option<RawActor>,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    head_repository_owner: Option<RawActor>,
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    status_check_rollup: Option<Vec<RawCheck>>,
    #[serde(default)]
    latest_reviews: Option<Vec<RawReview>>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    #[serde(default)]
    changed_files: u64,
    #[serde(default)]
    updated_at: String,
}

#[derive(Deserialize)]
struct RawBranchRef {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepoInfo {
    name_with_owner: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    default_branch_ref: Option<RawBranchRef>,
    #[serde(default)]
    merge_commit_allowed: bool,
    #[serde(default)]
    squash_merge_allowed: bool,
    #[serde(default)]
    rebase_merge_allowed: bool,
    #[serde(default)]
    viewer_default_merge_method: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawRepository {
    name_with_owner: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_private: bool,
    #[serde(default)]
    is_fork: bool,
    #[serde(default)]
    is_archived: bool,
}

#[derive(Deserialize)]
struct RawOwnersAnswer {
    data: RawOwnersData,
}

#[derive(Deserialize)]
struct RawOwnersData {
    viewer: RawViewer,
}

#[derive(Deserialize)]
struct RawViewer {
    login: String,
    #[serde(default)]
    organizations: Option<RawOrganizations>,
}

#[derive(Deserialize)]
struct RawOrganizations {
    #[serde(default)]
    nodes: Vec<Option<RawActor>>,
}

enum Outcome {
    Passed,
    Pending,
    Failed,
}

fn check_outcome(check: &RawCheck) -> Outcome {
    if let Some(state) = check.state.as_deref() {
        return match state {
            "SUCCESS" => Outcome::Passed,
            "FAILURE" | "ERROR" => Outcome::Failed,
            _ => Outcome::Pending,
        };
    }
    if check.status.as_deref() != Some("COMPLETED") {
        return Outcome::Pending;
    }
    match check.conclusion.as_deref() {
        Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => Outcome::Passed,
        _ => Outcome::Failed,
    }
}

fn summarize_checks(checks: &[RawCheck]) -> Checks {
    let mut summary = Checks::default();
    for check in checks {
        match check_outcome(check) {
            Outcome::Passed => summary.passed += 1,
            Outcome::Pending => summary.pending += 1,
            Outcome::Failed => summary.failed.push(
                check
                    .name
                    .clone()
                    .or_else(|| check.context.clone())
                    .unwrap_or_else(|| "unnamed check".to_string()),
            ),
        }
    }
    summary
}

pub fn parse_pull_requests(json: &str) -> Result<Vec<PullRequest>> {
    let raw: Vec<RawPullRequest> =
        serde_json::from_str(json).context("unexpected pull request list from gh")?;
    Ok(raw
        .into_iter()
        .map(|pr| PullRequest {
            number: pr.number,
            title: pr.title,
            author: pr.author.map(|a| a.login).unwrap_or_default(),
            head: pr.head_ref_name,
            head_owner: pr
                .is_cross_repository
                .then(|| pr.head_repository_owner.map(|owner| owner.login))
                .flatten()
                .filter(|login| !login.is_empty()),
            base: pr.base_ref_name,
            state: match pr.state.as_str() {
                "MERGED" => PrState::Merged,
                "CLOSED" => PrState::Closed,
                _ => PrState::Open,
            },
            draft: pr.is_draft,
            review: match pr.review_decision.as_deref() {
                Some("APPROVED") => ReviewDecision::Approved,
                Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
                Some("REVIEW_REQUIRED") => ReviewDecision::ReviewRequired,
                _ => ReviewDecision::None,
            },
            reviews: pr
                .latest_reviews
                .unwrap_or_default()
                .into_iter()
                .map(|review| Review {
                    author: review.author.map(|a| a.login).unwrap_or_default(),
                    state: review.state,
                })
                .collect(),
            checks: summarize_checks(&pr.status_check_rollup.unwrap_or_default()),
            mergeable: match pr.mergeable.as_deref() {
                Some("MERGEABLE") => Mergeable::Yes,
                Some("CONFLICTING") => Mergeable::Conflicting,
                _ => Mergeable::Unknown,
            },
            url: pr.url,
            body: pr.body.unwrap_or_default(),
            additions: pr.additions,
            deletions: pr.deletions,
            changed_files: pr.changed_files,
            updated_at: pr.updated_at,
        })
        .collect())
}

pub fn parse_repo_info(json: &str) -> Result<RepoInfo> {
    let raw: RawRepoInfo =
        serde_json::from_str(json).context("unexpected repository details from gh")?;
    let merge_methods: Vec<MergeMethod> = [
        (MergeMethod::Merge, raw.merge_commit_allowed),
        (MergeMethod::Squash, raw.squash_merge_allowed),
        (MergeMethod::Rebase, raw.rebase_merge_allowed),
    ]
    .into_iter()
    .filter_map(|(method, allowed)| allowed.then_some(method))
    .collect();
    let preferred_merge = raw
        .viewer_default_merge_method
        .as_deref()
        .and_then(MergeMethod::from_github)
        .filter(|method| merge_methods.contains(method))
        .or_else(|| merge_methods.first().copied())
        .unwrap_or(MergeMethod::Merge);
    Ok(RepoInfo {
        name: raw.name_with_owner,
        url: raw.url,
        default_branch: raw
            .default_branch_ref
            .map(|branch| branch.name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "main".to_string()),
        merge_methods,
        preferred_merge,
    })
}

pub fn parse_repositories(json: &str) -> Result<Vec<Repository>> {
    let raw: Vec<RawRepository> =
        serde_json::from_str(json).context("unexpected repository list from gh")?;
    Ok(raw
        .into_iter()
        .map(|repo| Repository {
            name: repo.name_with_owner,
            description: repo.description.unwrap_or_default(),
            private: repo.is_private,
            fork: repo.is_fork,
            archived: repo.is_archived,
        })
        .collect())
}

pub fn parse_owners(json: &str) -> Result<Owners> {
    let raw: RawOwnersAnswer =
        serde_json::from_str(json).context("unexpected account details from gh")?;
    let mut orgs: Vec<String> = raw
        .data
        .viewer
        .organizations
        .map(|orgs| orgs.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .map(|org| org.login)
        .filter(|login| !login.is_empty())
        .collect();
    orgs.sort_by_key(|login| login.to_lowercase());
    Ok(Owners {
        login: raw.data.viewer.login,
        orgs,
    })
}
