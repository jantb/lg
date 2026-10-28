//! What on GitHub is waiting on the user in the repositories of this
//! workspace: the unread notifications, and every pull request whose review is
//! still asked of them after its notification was read.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::run;

/// Why something is in the inbox, most pressing first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// A review asked of the user by name.
    ReviewRequested,
    Mentioned,
    /// A review asked of a team the user is on, not of them.
    TeamReviewRequested,
    TeamMentioned,
    Assigned,
    /// A deployment waiting on the user's approval.
    ApprovalRequested,
    SecurityAlert,
    /// Activity on a thread the user opened.
    Authored,
    /// Activity on a thread the user commented on.
    Commented,
    StateChanged,
    /// A workflow run the user started has finished.
    Ci,
    /// Anything else GitHub notifies about, by GitHub's name for it.
    Other(String),
}

impl Reason {
    fn from_github(reason: &str) -> Self {
        match reason {
            "review_requested" => Self::ReviewRequested,
            "mention" => Self::Mentioned,
            "team_mention" => Self::TeamMentioned,
            "assign" => Self::Assigned,
            "approval_requested" => Self::ApprovalRequested,
            "security_alert" => Self::SecurityAlert,
            "author" => Self::Authored,
            "comment" => Self::Commented,
            "state_change" => Self::StateChanged,
            "ci_activity" => Self::Ci,
            other => Self::Other(other.replace('_', " ")),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::ReviewRequested => "review requested",
            Self::Mentioned => "mentioned",
            Self::TeamReviewRequested => "team review requested",
            Self::TeamMentioned => "team mentioned",
            Self::Assigned => "assigned",
            Self::ApprovalRequested => "approval requested",
            Self::SecurityAlert => "security alert",
            Self::Authored => "on yours",
            Self::Commented => "commented on",
            Self::StateChanged => "state changed",
            Self::Ci => "CI",
            Self::Other(reason) => reason,
        }
    }

    /// The label at its shortest, for a column in a list.
    pub fn short(&self) -> &str {
        match self {
            Self::ReviewRequested => "review",
            Self::Mentioned => "mention",
            Self::TeamReviewRequested => "review",
            Self::TeamMentioned => "mention",
            Self::Assigned => "assigned",
            Self::ApprovalRequested => "approval",
            Self::SecurityAlert => "security",
            Self::Authored => "yours",
            Self::Commented => "comment",
            Self::StateChanged => "state",
            Self::Ci => "CI",
            Self::Other(reason) => reason,
        }
    }

    /// Who is being waited on, if anyone.
    pub fn section(&self) -> Section {
        match self {
            Self::ReviewRequested
            | Self::Mentioned
            | Self::Assigned
            | Self::ApprovalRequested
            | Self::SecurityAlert => Section::You,
            Self::TeamReviewRequested | Self::TeamMentioned => Section::Teams,
            Self::Authored | Self::Commented | Self::StateChanged | Self::Ci | Self::Other(_) => {
                Section::Updates
            }
        }
    }
}

/// The parts of the inbox, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// Waiting on the user by name.
    You,
    /// Waiting on a team the user is on.
    Teams,
    /// Telling the user something happened.
    Updates,
}

impl Section {
    pub const ALL: [Self; 3] = [Self::You, Self::Teams, Self::Updates];

    pub fn title(self) -> &'static str {
        match self {
            Self::You => "Waiting on you",
            Self::Teams => "Your teams",
            Self::Updates => "Updates",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subject {
    PullRequest(u64),
    Issue(u64),
    /// A release, discussion, check suite or the like, by GitHub's name for
    /// its kind.
    Other(String),
}

impl Subject {
    pub fn number(&self) -> Option<u64> {
        match self {
            Self::PullRequest(number) | Self::Issue(number) => Some(*number),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxItem {
    /// `owner/name`.
    pub repo: String,
    pub subject: Subject,
    pub title: String,
    pub reason: Reason,
    /// The page to open in a browser.
    pub url: String,
    /// The notification thread, while there is one to mark done. A review
    /// request whose notification was already read has none.
    pub thread: Option<String>,
    /// When it last changed, as GitHub writes it (RFC 3339); empty if unknown.
    pub updated_at: String,
}

impl InboxItem {
    /// What the item is about, the same whichever way it was found.
    pub fn key(&self) -> String {
        match &self.subject {
            Subject::PullRequest(number) | Subject::Issue(number) => {
                format!("{}#{number}", self.repo.to_lowercase())
            }
            Subject::Other(_) => format!("{} {}", self.repo.to_lowercase(), self.title),
        }
    }

    /// How long ago it last changed, shortest form: `5m`, `3h`, `2d`, `6w`.
    pub fn age(&self, now: DateTime<Utc>) -> Option<String> {
        let then = DateTime::parse_from_rfc3339(&self.updated_at).ok()?;
        let minutes = (now - then.with_timezone(&Utc)).num_minutes().max(0);
        Some(match minutes {
            0..60 => format!("{minutes}m"),
            60..1440 => format!("{}h", minutes / 60),
            1440..20160 => format!("{}d", minutes / 1440),
            _ => format!("{}w", minutes / 10080),
        })
    }

    /// `name#12` without the owner, or just the name for what has no number.
    pub fn short_place(&self) -> String {
        let name = self.repo.rsplit('/').next().unwrap_or(&self.repo);
        match self.subject.number() {
            Some(number) => format!("{name}#{number}"),
            None => name.to_string(),
        }
    }

    /// `owner/name#12`, or just the repository for what has no number.
    pub fn place(&self) -> String {
        match self.subject.number() {
            Some(number) => format!("{}#{number}", self.repo),
            None => self.repo.clone(),
        }
    }
}

/// What a read of the inbox found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inbox {
    /// What waits on the user, on their teams, then the updates, each newest
    /// first.
    pub items: Vec<InboxItem>,
    /// The checkout of each repository the workspace holds, by lowercased
    /// `owner/name`.
    pub checkouts: HashMap<String, PathBuf>,
}

impl Inbox {
    pub fn checkout(&self, repo: &str) -> Option<&PathBuf> {
        self.checkouts.get(&repo.to_lowercase())
    }
}

/// Read what is waiting in the GitHub repositories `dirs` are checkouts of.
/// Earlier directories win when two are checkouts of the same repository. With
/// no GitHub repository among them, GitHub is not asked at all.
pub fn read_inbox(dirs: &[PathBuf]) -> Result<Inbox> {
    let checkouts = checkouts(dirs);
    if checkouts.is_empty() {
        return Ok(Inbox::default());
    }
    let notifications =
        parse_notifications(&run(&["api", "--paginate", "notifications?per_page=50"])?)?;
    let search = |query: &str| -> Result<Vec<InboxItem>> {
        parse_review_requests(&run(&[
            "search",
            "prs",
            "--state=open",
            "--limit",
            "100",
            "--json",
            "number,title,url,repository,updatedAt",
            "--",
            query,
        ])?)
    };
    // `review-requested` counts the user's teams too; `user-review-requested`
    // only the user, which is what tells the two apart.
    let review_requests = search("review-requested:@me")?;
    let personal = search("user-review-requested:@me")?
        .iter()
        .map(InboxItem::key)
        .collect();
    let items = gather(notifications, review_requests, &personal, |repo| {
        checkouts.contains_key(&repo.to_lowercase())
    });
    Ok(Inbox { items, checkouts })
}

/// Mark a notification thread done, which takes it out of the GitHub inbox
/// until something new happens on it.
pub fn mark_done(thread: &str) -> Result<()> {
    run(&[
        "api",
        "-X",
        "DELETE",
        &format!("notifications/threads/{thread}"),
    ])?;
    Ok(())
}

/// Mark every one of `threads` done, stopping at the first GitHub refuses.
pub fn mark_all_done(threads: &[String]) -> Result<()> {
    threads.iter().try_for_each(|thread| mark_done(thread))
}

fn checkouts(dirs: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut checkouts = HashMap::new();
    for dir in dirs {
        let urls = crate::git::with_repo(dir, crate::git::remote_urls).unwrap_or_default();
        for name in urls.iter().filter_map(|url| repo_name(url)) {
            checkouts
                .entry(name.to_lowercase())
                .or_insert_with(|| dir.clone());
        }
    }
    checkouts
}

/// `owner/name` of the repository a remote URL points at, in any of the forms
/// git accepts for a hosted one. A local path, written as a `file://` URL or
/// not, names no hosted repository.
pub fn repo_name(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let path = match url.split_once("://") {
        Some(("file", _)) => return None,
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.trim_end_matches(".git");
    let mut segments = path.rsplit('/').filter(|segment| !segment.is_empty());
    let name = segments.next()?;
    let owner = segments.next()?;
    Some(format!("{owner}/{name}"))
}

/// The notifications in `watched` repositories, then every review request
/// they do not already carry. A pull request that is both is shown once, as
/// the review request it still is: the user's own when its key is in
/// `personal`, a team's otherwise. A review request notification for a pull
/// request no longer asking for one is only an update.
fn gather(
    notifications: Vec<InboxItem>,
    review_requests: Vec<InboxItem>,
    personal: &HashSet<String>,
    watched: impl Fn(&str) -> bool,
) -> Vec<InboxItem> {
    let mut items: Vec<InboxItem> = notifications
        .into_iter()
        .filter(|item| watched(&item.repo))
        .collect();
    let requested: HashSet<String> = review_requests.iter().map(InboxItem::key).collect();
    for request in review_requests
        .into_iter()
        .filter(|item| watched(&item.repo))
    {
        match items.iter_mut().find(|item| item.key() == request.key()) {
            Some(item) => item.reason = Reason::ReviewRequested,
            None => items.push(request),
        }
    }
    for item in &mut items {
        if item.reason != Reason::ReviewRequested {
            continue;
        }
        let key = item.key();
        if !personal.contains(&key) {
            item.reason = if requested.contains(&key) {
                Reason::TeamReviewRequested
            } else {
                Reason::Other("review requested earlier".into())
            };
        }
    }
    // RFC 3339 times in UTC order as text; one GitHub gave none goes last.
    items.sort_by(|a, b| {
        a.reason
            .section()
            .cmp(&b.reason.section())
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    items
}

#[derive(Deserialize)]
struct RawNotification {
    id: String,
    reason: String,
    subject: RawSubject,
    repository: RawRepository,
    #[serde(default)]
    updated_at: String,
}

#[derive(Deserialize)]
struct RawSubject {
    title: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct RawRepository {
    full_name: String,
    html_url: String,
}

/// Every page `gh api --paginate` printed, one JSON array after another.
fn parse_notifications(json: &str) -> Result<Vec<InboxItem>> {
    let mut items = Vec::new();
    for page in serde_json::Deserializer::from_str(json).into_iter::<Vec<RawNotification>>() {
        let page = page.context("unexpected notifications from gh")?;
        items.extend(page.into_iter().map(notification_item));
    }
    Ok(items)
}

fn notification_item(raw: RawNotification) -> InboxItem {
    let number = raw
        .subject
        .url
        .as_deref()
        .and_then(|url| url.rsplit('/').next())
        .and_then(|last| last.parse().ok());
    let (subject, url) = match (raw.subject.kind.as_str(), number) {
        ("PullRequest", Some(number)) => (
            Subject::PullRequest(number),
            format!("{}/pull/{number}", raw.repository.html_url),
        ),
        ("Issue", Some(number)) => (
            Subject::Issue(number),
            format!("{}/issues/{number}", raw.repository.html_url),
        ),
        (kind, _) => (Subject::Other(kind.to_string()), raw.repository.html_url),
    };
    InboxItem {
        repo: raw.repository.full_name,
        subject,
        title: raw.subject.title,
        reason: Reason::from_github(&raw.reason),
        url,
        thread: Some(raw.id),
        updated_at: raw.updated_at,
    }
}

#[derive(Deserialize)]
struct RawSearchHit {
    number: u64,
    title: String,
    url: String,
    repository: RawSearchRepository,
    #[serde(default, rename = "updatedAt")]
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSearchRepository {
    name_with_owner: String,
}

fn parse_review_requests(json: &str) -> Result<Vec<InboxItem>> {
    let hits: Vec<RawSearchHit> =
        serde_json::from_str(json).context("unexpected review requests from gh")?;
    Ok(hits
        .into_iter()
        .map(|hit| InboxItem {
            repo: hit.repository.name_with_owner,
            subject: Subject::PullRequest(hit.number),
            title: hit.title,
            reason: Reason::ReviewRequested,
            url: hit.url,
            thread: None,
            updated_at: hit.updated_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(repo: &str, number: u64, reason: &str) -> String {
        format!(
            r#"{{"id":"t{number}","reason":"{reason}",
                "subject":{{"title":"Change {number}","type":"PullRequest",
                            "url":"https://api.github.com/repos/{repo}/pulls/{number}"}},
                "repository":{{"full_name":"{repo}","html_url":"https://github.com/{repo}"}}}}"#
        )
    }

    /// The pull requests whose review is asked of the user by name.
    fn mine(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn a_review_asked_of_a_team_is_told_from_one_asked_of_the_user() {
        let requests = parse_review_requests(
            r#"[{"number":1,"title":"Mine","url":"u","repository":{"nameWithOwner":"acme/app"}},
                {"number":2,"title":"Team","url":"u","repository":{"nameWithOwner":"acme/app"}}]"#,
        )
        .unwrap();
        let stale = parse_notifications(&format!(
            "[{}]",
            notification("acme/app", 3, "review_requested")
        ))
        .unwrap();

        let items = gather(stale, requests, &mine(&["acme/app#1"]), |_| true);

        let reason = |number| {
            let item = items
                .iter()
                .find(|item| item.subject == Subject::PullRequest(number));
            item.unwrap().reason.section()
        };
        assert_eq!(reason(1), Section::You);
        assert_eq!(reason(2), Section::Teams);
        assert_eq!(reason(3), Section::Updates, "no longer asking for a review");
    }

    #[test]
    fn an_ssh_remote_names_its_repository() {
        assert_eq!(
            repo_name("git@github.com:acme/app.git").as_deref(),
            Some("acme/app")
        );
    }

    #[test]
    fn an_https_remote_names_its_repository() {
        assert_eq!(
            repo_name("https://me@github.com/acme/app.git/").as_deref(),
            Some("acme/app")
        );
    }

    #[test]
    fn a_remote_on_this_machine_names_no_repository() {
        assert_eq!(repo_name("file:///srv/git/app.git"), None);
    }

    #[test]
    fn every_page_of_notifications_is_read_and_opens_on_its_page() {
        let json = format!(
            "[{}]\n[{}]",
            notification("acme/app", 1, "mention"),
            notification("acme/lib", 2, "author")
        );

        let items = parse_notifications(&json).unwrap();

        let urls: Vec<&str> = items.iter().map(|item| item.url.as_str()).collect();
        assert!(
            urls.contains(&"https://github.com/acme/app/pull/1"),
            "{urls:?}"
        );
        assert!(
            urls.contains(&"https://github.com/acme/lib/pull/2"),
            "{urls:?}"
        );
        assert!(items.iter().all(|item| item.thread.is_some()));
    }

    #[test]
    fn only_the_workspaces_repositories_are_in_the_inbox() {
        let notifications = parse_notifications(&format!(
            "[{}]",
            notification("other/elsewhere", 3, "mention")
        ))
        .unwrap();
        let requests = parse_review_requests(
            r#"[{"number":9,"title":"Elsewhere","url":"https://github.com/other/x/pull/9",
                 "repository":{"name":"x","nameWithOwner":"other/x"}}]"#,
        )
        .unwrap();

        let items = gather(notifications, requests, &mine(&[]), |repo| {
            repo == "acme/app"
        });

        assert!(items.is_empty(), "{items:?}");
    }

    #[test]
    fn a_review_still_asked_for_stays_after_its_notification_is_read() {
        let requests = parse_review_requests(
            r#"[{"number":7,"title":"Add login","url":"https://github.com/acme/app/pull/7",
                 "repository":{"name":"app","nameWithOwner":"acme/app"}}]"#,
        )
        .unwrap();

        let items = gather(Vec::new(), requests, &mine(&["acme/app#7"]), |_| true);

        assert_eq!(items[0].reason, Reason::ReviewRequested);
        assert_eq!(items[0].url, "https://github.com/acme/app/pull/7");
    }

    #[test]
    fn a_pull_request_both_notified_and_awaiting_review_is_one_review_request() {
        let notifications =
            parse_notifications(&format!("[{}]", notification("acme/app", 7, "comment"))).unwrap();
        let requests = parse_review_requests(
            r#"[{"number":7,"title":"Change 7","url":"https://github.com/acme/app/pull/7",
                 "repository":{"name":"app","nameWithOwner":"Acme/App"}}]"#,
        )
        .unwrap();

        let items = gather(notifications, requests, &mine(&["acme/app#7"]), |_| true);

        let sevens: Vec<_> = items
            .iter()
            .filter(|item| item.subject == Subject::PullRequest(7))
            .collect();
        assert_eq!(sevens.len(), 1, "{items:?}");
        assert_eq!(sevens[0].reason, Reason::ReviewRequested);
        assert!(sevens[0].thread.is_some(), "it can still be marked done");
    }

    #[test]
    fn an_item_says_how_long_ago_it_changed() {
        let mut item = parse_review_requests(
            r#"[{"number":7,"title":"Add login","url":"https://github.com/acme/app/pull/7",
                 "repository":{"name":"app","nameWithOwner":"acme/app"},
                 "updatedAt":"2026-10-20T12:00:00Z"}]"#,
        )
        .unwrap()
        .remove(0);
        let now = |at: &str| {
            DateTime::parse_from_rfc3339(at)
                .unwrap()
                .with_timezone(&Utc)
        };

        assert_eq!(item.age(now("2026-10-20T12:05:00Z")).as_deref(), Some("5m"));
        assert_eq!(item.age(now("2026-10-20T15:00:00Z")).as_deref(), Some("3h"));
        assert_eq!(item.age(now("2026-10-22T12:00:00Z")).as_deref(), Some("2d"));
        assert_eq!(item.age(now("2026-12-01T12:00:00Z")).as_deref(), Some("6w"));
        item.updated_at.clear();
        assert_eq!(item.age(now("2026-10-20T12:00:00Z")), None);
    }

    #[test]
    fn the_newest_comes_first_within_what_waits_on_the_user() {
        let requests = parse_review_requests(
            r#"[{"number":1,"title":"Old","url":"u","repository":{"nameWithOwner":"acme/app"},
                 "updatedAt":"2026-10-01T10:00:00Z"},
                {"number":2,"title":"New","url":"u","repository":{"nameWithOwner":"acme/app"},
                 "updatedAt":"2026-10-20T10:00:00Z"}]"#,
        )
        .unwrap();

        let items = gather(Vec::new(), requests, &mine(&[]), |_| true);

        assert_eq!(items[0].subject, Subject::PullRequest(2));
    }

    #[test]
    fn what_asks_for_the_user_comes_before_what_only_tells_them() {
        let json = format!(
            "[{},{}]",
            notification("acme/app", 1, "ci_activity"),
            notification("acme/app", 2, "review_requested")
        );

        let items = gather(
            parse_notifications(&json).unwrap(),
            Vec::new(),
            &mine(&["acme/app#2"]),
            |_| true,
        );

        assert_eq!(items[0].subject, Subject::PullRequest(2));
    }
}
