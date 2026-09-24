//! GitHub, through the `gh` command-line tool.
//!
//! lg does not speak the GitHub API itself. `gh` already holds the user's
//! login, knows which remote of a checkout is the GitHub one, and sets up the
//! fork's remote when a pull request from a fork is checked out, so every call
//! here is a `gh` invocation against the checkout lg is pointed at, read back
//! as JSON where there is anything to read.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Which pull requests the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrFilter {
    #[default]
    Open,
    /// Open pull requests waiting on the user's review.
    ReviewRequested,
    /// The user's own, whatever became of them.
    Mine,
    /// Everything, merged and closed included.
    All,
}

impl PrFilter {
    pub const ALL: [Self; 4] = [Self::Open, Self::ReviewRequested, Self::Mine, Self::All];

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ReviewRequested => "review requested",
            Self::Mine => "mine",
            Self::All => "all",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Open => &["--state", "open"],
            Self::ReviewRequested => &["--state", "open", "--search", "review-requested:@me"],
            Self::Mine => &["--state", "all", "--author", "@me"],
            Self::All => &["--state", "all"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }
}

/// Where the reviews of a pull request have left it, as GitHub sums them up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    /// The repository asks for no reviews.
    None,
}

impl ReviewDecision {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Approved => Some("approved"),
            Self::ChangesRequested => Some("changes requested"),
            Self::ReviewRequired => Some("review required"),
            Self::None => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    Yes,
    Conflicting,
    /// GitHub has not worked it out yet; it does so lazily.
    Unknown,
}

/// The checks on a pull request's head commit, counted by outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checks {
    pub passed: usize,
    pub pending: usize,
    /// Names of the checks that failed, since those are the ones to go and read.
    pub failed: Vec<String>,
}

impl Checks {
    pub fn is_empty(&self) -> bool {
        self.passed == 0 && self.pending == 0 && self.failed.is_empty()
    }
}

/// One reviewer's latest word on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub author: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, … as GitHub spells it.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    /// The branch the changes are on.
    pub head: String,
    /// Who owns the head branch's repository, for a pull request from a fork.
    pub head_owner: Option<String>,
    /// The branch they are to be merged into.
    pub base: String,
    pub state: PrState,
    pub draft: bool,
    pub review: ReviewDecision,
    pub reviews: Vec<Review>,
    pub checks: Checks,
    pub mergeable: Mergeable,
    pub url: String,
    pub body: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub updated_at: String,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == PrState::Open
    }
}

/// A repository offered for cloning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// `owner/name`.
    pub name: String,
    pub description: String,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [Self; 3] = [Self::Merge, Self::Squash, Self::Rebase];

    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "merge commit",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Merge => "--merge",
            Self::Squash => "--squash",
            Self::Rebase => "--rebase",
        }
    }

    fn from_github(name: &str) -> Option<Self> {
        match name {
            "MERGE" => Some(Self::Merge),
            "SQUASH" => Some(Self::Squash),
            "REBASE" => Some(Self::Rebase),
            _ => None,
        }
    }
}

/// Whose repositories there are to list: the signed-in user's own, and those
/// of each organization they belong to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Owners {
    pub login: String,
    /// Sorted by name, so stepping through them goes in the order shown.
    pub orgs: Vec<String>,
}

/// What the repository a checkout belongs to allows and prefers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// `owner/name`.
    pub name: String,
    pub url: String,
    pub default_branch: String,
    /// The merge methods the repository accepts, in [`MergeMethod::ALL`] order.
    pub merge_methods: Vec<MergeMethod>,
    /// The method the user last merged with here, as GitHub remembers it.
    pub preferred_merge: MergeMethod,
}

/// A review that settles something, as opposed to a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    RequestChanges,
}

impl Verdict {
    fn flag(self) -> &'static str {
        match self {
            Self::Approve => "--approve",
            Self::RequestChanges => "--request-changes",
        }
    }
}

// ─── Reading ─────────────────────────────────────────────────────────────────

/// The repository the checkout lg is pointed at belongs to.
pub fn repo_info() -> Result<RepoInfo> {
    parse_repo_info(&run(&["repo", "view", "--json", REPO_INFO_FIELDS])?)
}

pub fn pull_requests(filter: PrFilter) -> Result<Vec<PullRequest>> {
    let mut args = vec!["pr", "list", "--limit", PR_LIMIT, "--json", PR_FIELDS];
    args.extend(filter.args());
    parse_pull_requests(&run(&args)?)
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

// ─── Acting ──────────────────────────────────────────────────────────────────

/// Check a pull request out in the checkout lg is pointed at.
pub fn checkout(number: u64) -> Result<String> {
    let n = number.to_string();
    let out = run_combined(None, &["pr", "checkout", &n])?;
    Ok(last_line(&out).unwrap_or_else(|| format!("checked out #{number}")))
}

/// Check a pull request out in a new worktree at `path`, leaving the checkout
/// lg is pointed at as it was.
///
/// The worktree starts detached and `gh` checks the pull request out inside
/// it, so a pull request from a fork gets its remote set up the same way it
/// would in place.
pub fn checkout_in_worktree(number: u64, path: &Path) -> Result<String> {
    crate::git::worktree_add_detached(path)?;
    let n = number.to_string();
    match run_combined(Some(path), &["pr", "checkout", &n]) {
        Ok(_) => Ok(format!("checked out #{number} in {}", path.display())),
        Err(err) => {
            // A worktree parked on a detached HEAD is clutter nobody asked for.
            let _ = crate::git::worktree_remove(path, true);
            Err(err)
        }
    }
}

/// A flag and its value as one argument. Written apart, gh takes whatever
/// follows a flag as its value, but a value that starts with a dash — a title
/// such as `--draft` — reads as a flag of its own.
fn flag_value(flag: &str, value: &str) -> String {
    format!("{flag}={value}")
}

pub fn review(number: u64, verdict: Verdict, body: &str) -> Result<String> {
    let n = number.to_string();
    let body_arg = flag_value("--body", body);
    let mut args = vec!["pr", "review", &n, verdict.flag()];
    if !body.trim().is_empty() {
        args.push(&body_arg);
    }
    run_combined(None, &args)?;
    Ok(match verdict {
        Verdict::Approve => format!("approved #{number}"),
        Verdict::RequestChanges => format!("requested changes on #{number}"),
    })
}

pub fn comment(number: u64, body: &str) -> Result<String> {
    let n = number.to_string();
    run_combined(None, &["pr", "comment", &n, &flag_value("--body", body)])?;
    Ok(format!("commented on #{number}"))
}

/// Merge a pull request. `auto` queues the merge for when the branch
/// protection rules are met, which is the only way through while required
/// checks are still running.
pub fn merge(number: u64, method: MergeMethod, delete_branch: bool, auto: bool) -> Result<String> {
    let n = number.to_string();
    let mut args = vec!["pr", "merge", &n, method.flag()];
    if delete_branch {
        args.push("--delete-branch");
    }
    if auto {
        args.push("--auto");
    }
    run_combined(None, &args)?;
    Ok(if auto {
        format!("#{number} will merge once its requirements are met")
    } else {
        format!("merged #{number} ({})", method.label())
    })
}

/// Mark a draft ready for review, or turn a pull request back into a draft.
pub fn set_draft(number: u64, draft: bool) -> Result<String> {
    let n = number.to_string();
    let mut args = vec!["pr", "ready", &n];
    if draft {
        args.push("--undo");
    }
    run_combined(None, &args)?;
    Ok(if draft {
        format!("#{number} is a draft again")
    } else {
        format!("#{number} is ready for review")
    })
}

pub fn close(number: u64) -> Result<String> {
    let n = number.to_string();
    run_combined(None, &["pr", "close", &n])?;
    Ok(format!("closed #{number}"))
}

pub fn reopen(number: u64) -> Result<String> {
    let n = number.to_string();
    run_combined(None, &["pr", "reopen", &n])?;
    Ok(format!("reopened #{number}"))
}

/// What a new pull request says and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullRequest {
    pub branch: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// Push `pr.branch` to `remote` and open a pull request for it.
///
/// `gh` cannot open one for a branch the remote has not seen, and asks where
/// to push it only when a terminal is there to answer. lg answers the way it
/// pushes everywhere else: to the configured remote, which the branch then
/// tracks.
pub fn create(remote: &str, pr: &NewPullRequest) -> Result<String> {
    crate::git::push_with_upstream(remote, &pr.branch)?;
    let (head, base) = (
        flag_value("--head", &pr.branch),
        flag_value("--base", &pr.base),
    );
    let (title, body) = (
        flag_value("--title", &pr.title),
        flag_value("--body", &pr.body),
    );
    let mut args = vec!["pr", "create", &head, &base, &title, &body];
    if pr.draft {
        args.push("--draft");
    }
    let url = run(&args)?;
    Ok(match last_line(&url) {
        Some(url) => format!("opened {url}"),
        None => format!("opened a pull request for {}", pr.branch),
    })
}

/// Clone `repo` — `owner/name` or a URL — into `dir`.
pub fn clone(repo: &str, dir: &Path) -> Result<String> {
    // gh would read a leading dash as an option, and no repository has one.
    if repo.starts_with('-') {
        anyhow::bail!("not a repository: {repo}");
    }
    if dir.exists() {
        anyhow::bail!("{} already exists", dir.display());
    }
    let target = dir.to_string_lossy().into_owned();
    run_combined(dir.parent(), &["repo", "clone", repo, &target])?;
    Ok(format!("cloned {repo} into {}", dir.display()))
}

/// Open `url` in the browser. Not waited on beyond starting it.
pub fn open_in_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start the browser")?;
    // Reaped off the UI thread so the opener does not linger as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

// ─── Pure helpers ────────────────────────────────────────────────────────────

/// The directory a clone of `repo` gets under `parent`: the repository's own
/// name, as `gh repo clone` would pick it.
pub fn clone_dir(parent: &Path, repo: &str) -> Option<PathBuf> {
    let name = repo
        .trim()
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()?
        .trim_end_matches(".git");
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(parent.join(name))
}

/// A title and body for a new pull request, read off the commits it carries:
/// one commit speaks for itself, several are listed under the branch name.
/// The same choice `gh pr create --fill` makes.
pub fn fill_from_commits(branch: &str, commits: &[(String, String)]) -> (String, String) {
    match commits {
        [(subject, body)] => (subject.clone(), body.trim().to_string()),
        _ => {
            let body = commits
                .iter()
                .map(|(subject, _)| format!("- {subject}"))
                .collect::<Vec<_>>()
                .join("\n");
            (humanize_branch(branch), body)
        }
    }
}

/// `feat/add-login` reads as `Add login`: the last path segment, with its
/// separators turned back into spaces.
fn humanize_branch(branch: &str) -> String {
    let leaf = branch.rsplit('/').next().unwrap_or(branch);
    let words = leaf.replace(['-', '_'], " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => branch.to_string(),
    }
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rfind(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
}

// ─── Parsing ─────────────────────────────────────────────────────────────────

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
struct RawRepository {
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

// ─── Running gh ──────────────────────────────────────────────────────────────

/// A `gh` invocation in `dir`, or in the checkout this thread's git commands
/// run in.
fn command(dir: Option<&Path>, args: &[&str]) -> Command {
    let mut command = Command::new("gh");
    command
        .args(args)
        .stdin(Stdio::null())
        // Never wait on a question nobody is there to answer, and keep the
        // output free of colour, spinners and upgrade notices so it parses.
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_SPINNER_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env("GH_PAGER", "cat");
    if let Some(dir) = dir
        .map(Path::to_path_buf)
        .or_else(crate::git::repo_dir)
        .filter(|dir| dir.is_dir())
    {
        command.current_dir(dir);
    }
    command
}

fn spawn_failure(err: std::io::Error) -> anyhow::Error {
    if err.kind() == std::io::ErrorKind::NotFound {
        anyhow::anyhow!(
            "the GitHub CLI is not installed: get gh from https://cli.github.com, then run `gh auth login`"
        )
    } else {
        anyhow::anyhow!("failed to start gh: {err}")
    }
}

/// What a failed `gh` call said, on one line: the status bar has room for
/// that and no more, and gh spreads its errors over several.
fn failure(args: &[&str], stdout: &str, stderr: &str) -> anyhow::Error {
    let said = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let said = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let what = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
    anyhow::anyhow!("gh {what} failed: {said}")
}

/// Run `gh` and hand back what it printed on stdout.
fn run(args: &[&str]) -> Result<String> {
    let out = command(None, args).output().map_err(spawn_failure)?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(failure(
            args,
            &stdout,
            &String::from_utf8_lossy(&out.stderr),
        ))
    }
}

/// Run `gh` and hand back everything it printed. The commands that drive git
/// report on stderr, and that report is the result.
fn run_combined(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let out = command(dir, args).output().map_err(spawn_failure)?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(format!("{stdout}{stderr}"))
    } else {
        Err(failure(args, &stdout, &stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            parse_owners(r#"{"data":{"viewer":{"login":"me","organizations":{"nodes":[]}}}}"#)
                .unwrap();

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
}
