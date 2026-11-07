//! The conversations on a pull request: the review threads left on lines of
//! its diff, how many are still open, and replying to and resolving them.
//!
//! `gh pr view` says nothing about review threads, so these go through
//! GitHub's GraphQL API, still by way of `gh` and its login. One read takes
//! in the first 100 threads of a pull request and 100 comments of each; one
//! with more is one to read on GitHub.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::gh::run;

/// One comment in a review thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadComment {
    pub author: String,
    pub body: String,
    /// When it was written, as GitHub spells it; empty for a reply sent from
    /// here that GitHub has not echoed back yet.
    pub created_at: String,
}

/// One conversation on a line of a pull request's diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewThread {
    /// The node id GitHub's mutations take.
    pub id: String,
    pub path: String,
    /// The line it is on, or was on before the code under it changed.
    pub line: Option<u64>,
    pub resolved: bool,
    /// The code it was left on has changed since.
    pub outdated: bool,
    pub resolved_by: Option<String>,
    /// Where it is on GitHub: its first comment.
    pub url: String,
    /// The diff around the line, as the first comment saw it.
    pub diff_hunk: String,
    pub comments: Vec<ThreadComment>,
}

impl ReviewThread {
    /// Who started it.
    pub fn author(&self) -> &str {
        self.comments.first().map_or("", |c| c.author.as_str())
    }

    /// `path:line`, or the path alone for a comment on the whole file.
    pub fn location(&self) -> String {
        match self.line {
            Some(line) => format!("{}:{line}", self.path),
            None => self.path.clone(),
        }
    }
}

/// How many of a pull request's conversations are still open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThreadCount {
    pub unresolved: usize,
    pub total: usize,
}

impl ThreadCount {
    pub fn of(threads: &[ReviewThread]) -> Self {
        Self {
            unresolved: threads.iter().filter(|t| !t.resolved).count(),
            total: threads.len(),
        }
    }
}

const THREADS_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){\
    repository(owner:$owner,name:$name){pullRequest(number:$number){\
    reviewThreads(first:100){nodes{id isResolved isOutdated path line originalLine \
    resolvedBy{login} comments(first:100){nodes{author{login} body createdAt url diffHunk}}}}}}}";

const REPLY_MUTATION: &str = "mutation($id:ID!,$body:String!){\
    addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$id,body:$body}){comment{id}}}";

const RESOLVE_MUTATION: &str =
    "mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{isResolved}}}";

const UNRESOLVE_MUTATION: &str =
    "mutation($id:ID!){unresolveReviewThread(input:{threadId:$id}){thread{isResolved}}}";

/// `gh api graphql` with the repository of the checkout lg is pointed at as
/// `$owner` and `$name`; gh fills those placeholders in itself.
fn graphql(query: &str, fields: &[String]) -> Result<String> {
    let query = format!("query={query}");
    let mut args = vec!["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}"];
    for field in fields {
        args.extend(["-F", field.as_str()]);
    }
    args.extend(["-f", query.as_str()]);
    run(&args)
}

/// A mutation, with every variable sent as a plain string so nothing in a
/// reply is read as a number or a file to load.
fn mutate(mutation: &str, vars: &[(&str, &str)]) -> Result<()> {
    let query = format!("query={mutation}");
    let vars: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let mut args = vec!["api", "graphql"];
    for var in &vars {
        args.extend(["-f", var.as_str()]);
    }
    args.extend(["-f", query.as_str()]);
    run(&args).map(drop)
}

/// Every conversation on pull request `number`, in the order GitHub lists
/// them: the order they were started in.
pub fn review_threads(number: u64) -> Result<Vec<ReviewThread>> {
    parse_threads(&graphql(THREADS_QUERY, &[format!("number={number}")])?)
}

/// How many conversations each of `numbers` has open, in one call.
pub fn thread_counts(numbers: &[u64]) -> Result<HashMap<u64, ThreadCount>> {
    if numbers.is_empty() {
        return Ok(HashMap::new());
    }
    parse_thread_counts(&graphql(&counts_query(numbers), &[])?)
}

pub fn reply_to_thread(thread_id: &str, body: &str) -> Result<()> {
    mutate(REPLY_MUTATION, &[("id", thread_id), ("body", body)])
}

/// Resolve a conversation, or open it again.
pub fn resolve_thread(thread_id: &str, resolve: bool) -> Result<()> {
    let mutation = if resolve {
        RESOLVE_MUTATION
    } else {
        UNRESOLVE_MUTATION
    };
    mutate(mutation, &[("id", thread_id)])
}

/// One query asking after every pull request in `numbers`, each under an
/// alias of its own number.
pub(super) fn counts_query(numbers: &[u64]) -> String {
    let fields: String = numbers
        .iter()
        .map(|n| {
            format!(
                "p{n}:pullRequest(number:{n}){{reviewThreads(first:100){{nodes{{isResolved}}}}}} "
            )
        })
        .collect();
    format!(
        "query($owner:String!,$name:String!){{repository(owner:$owner,name:$name){{{fields}}}}}"
    )
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: Option<RawRepository<T>>,
}

#[derive(Deserialize)]
struct RawRepository<T> {
    repository: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequest {
    pull_request: Option<RawThreads>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThreads {
    review_threads: Nodes<RawThread>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThread {
    #[serde(default)]
    id: String,
    #[serde(default)]
    is_resolved: bool,
    #[serde(default)]
    is_outdated: bool,
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: Option<u64>,
    #[serde(default)]
    original_line: Option<u64>,
    #[serde(default)]
    resolved_by: Option<Login>,
    comments: Nodes<RawComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawComment {
    #[serde(default)]
    author: Option<Login>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    diff_hunk: String,
}

#[derive(Deserialize)]
struct Login {
    #[serde(default)]
    login: String,
}

/// An author GitHub no longer knows — a deleted account — reads as `ghost`,
/// the way GitHub itself shows it.
fn login(actor: Option<Login>) -> String {
    actor
        .map(|a| a.login)
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "ghost".into())
}

pub(super) fn parse_threads(json: &str) -> Result<Vec<ReviewThread>> {
    let envelope: Envelope<RawPullRequest> =
        serde_json::from_str(json).context("read the review threads gh sent")?;
    let threads = envelope
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .context("GitHub did not find the pull request")?
        .review_threads
        .nodes;
    Ok(threads
        .into_iter()
        .map(|raw| {
            let first = raw.comments.nodes.first();
            let url = first.map(|c| c.url.clone()).unwrap_or_default();
            let diff_hunk = first.map(|c| c.diff_hunk.clone()).unwrap_or_default();
            ReviewThread {
                id: raw.id,
                path: raw.path,
                line: raw.line.or(raw.original_line),
                resolved: raw.is_resolved,
                outdated: raw.is_outdated,
                resolved_by: raw.resolved_by.map(|l| l.login).filter(|l| !l.is_empty()),
                url,
                diff_hunk,
                comments: raw
                    .comments
                    .nodes
                    .into_iter()
                    .map(|c| ThreadComment {
                        author: login(c.author),
                        body: c.body.replace("\r\n", "\n"),
                        created_at: c.created_at,
                    })
                    .collect(),
            }
        })
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawResolved {
    #[serde(default)]
    is_resolved: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCounted {
    review_threads: Nodes<RawResolved>,
}

pub(super) fn parse_thread_counts(json: &str) -> Result<HashMap<u64, ThreadCount>> {
    let envelope: Envelope<HashMap<String, Option<RawCounted>>> =
        serde_json::from_str(json).context("read the conversation counts gh sent")?;
    let repository = envelope
        .data
        .and_then(|d| d.repository)
        .context("GitHub did not find the repository")?;
    Ok(repository
        .into_iter()
        .filter_map(|(alias, pr)| {
            let number = alias.strip_prefix('p')?.parse().ok()?;
            let threads = pr?.review_threads.nodes;
            Some((
                number,
                ThreadCount {
                    unresolved: threads.iter().filter(|t| !t.is_resolved).count(),
                    total: threads.len(),
                },
            ))
        })
        .collect())
}
