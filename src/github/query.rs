//! Reading pull requests, repositories and the signed-in user's owners from
//! GitHub.

use anyhow::{Context, Result};

use super::gh::run;
use super::parse::{parse_owners, parse_pull_requests, parse_repo_info, parse_repositories};
use super::{Owners, PrFilter, PullRequest, RepoInfo, Repository};

/// How many pull requests one list asks for. A list longer than this is
/// searched on GitHub, not scrolled through in a terminal.
const PR_LIMIT: &str = "50";

const REPO_LIMIT: &str = "200";

const PR_FIELDS: &str = "number,title,author,headRefName,baseRefName,state,isDraft,\
                         reviewDecision,mergeable,statusCheckRollup,latestReviews,url,body,\
                         additions,deletions,changedFiles,isCrossRepository,\
                         headRepositoryOwner,updatedAt";

const REPO_INFO_FIELDS: &str = "nameWithOwner,url,defaultBranchRef,mergeCommitAllowed,\
                                squashMergeAllowed,rebaseMergeAllowed,viewerDefaultMergeMethod";

const REPO_LIST_FIELDS: &str = "nameWithOwner,description,isPrivate,isFork,isArchived";

/// The signed-in user and the organizations they belong to, in one call.
const OWNERS_QUERY: &str =
    "query=query { viewer { login organizations(first: 100) { nodes { login } } } }";

/// The repository the checkout lg is pointed at belongs to.
pub fn repo_info() -> Result<RepoInfo> {
    parse_repo_info(&run(&["repo", "view", "--json", REPO_INFO_FIELDS])?)
}

/// The pull requests `filter` selects, newest first, drafts left out when
/// `hide_drafts` says so.
pub fn pull_requests(filter: PrFilter, hide_drafts: bool) -> Result<Vec<PullRequest>> {
    let mut args = vec!["pr", "list", "--limit", PR_LIMIT, "--json", PR_FIELDS];
    args.extend(filter.args());
    let search = search_terms(filter, hide_drafts);
    if let Some(search) = &search {
        args.extend(["--search", search]);
    }
    parse_pull_requests(&run(&args)?)
}

/// The search a pull request list needs, if any. A search is ordered by
/// relevance unless told otherwise, so it is told to put the newest first, as
/// a plain list does.
pub(super) fn search_terms(filter: PrFilter, hide_drafts: bool) -> Option<String> {
    let mut terms: Vec<&str> = filter.search().into_iter().collect();
    if hide_drafts {
        terms.push("draft:false");
    }
    if terms.is_empty() {
        return None;
    }
    terms.push("sort:created-desc");
    Some(terms.join(" "))
}

/// One pull request of the checkout lg is pointed at, whatever became of it.
pub fn pull_request(number: u64) -> Result<PullRequest> {
    let n = number.to_string();
    let json = run(&["pr", "view", &n, "--json", PR_FIELDS])?;
    parse_pull_requests(&format!("[{json}]"))?
        .pop()
        .with_context(|| format!("gh said nothing about #{number}"))
}

/// The repositories of `owner` — an organization, or the signed-in user when
/// there is none — most recently touched first.
pub fn repositories(owner: Option<&str>) -> Result<Vec<Repository>> {
    let mut args = vec!["repo", "list"];
    args.extend(owner);
    args.extend(["--limit", REPO_LIMIT, "--json", REPO_LIST_FIELDS]);
    parse_repositories(&run(&args)?)
}

pub fn owners() -> Result<Owners> {
    parse_owners(&run(&["api", "graphql", "-f", OWNERS_QUERY])?)
}

/// The pull request's change as a unified diff, the way GitHub shows it.
pub fn pr_diff(number: u64) -> Result<String> {
    run(&["pr", "diff", &number.to_string()])
}

/// The commit the pull request's head is at, which inline comments are
/// pinned to.
pub fn pr_head(number: u64) -> Result<String> {
    let out = run(&[
        "pr",
        "view",
        &number.to_string(),
        "--json",
        "headRefOid",
        "--jq",
        ".headRefOid",
    ])?;
    let oid = out.trim();
    if oid.is_empty() {
        anyhow::bail!("gh did not report the head commit of #{number}");
    }
    Ok(oid.to_string())
}
