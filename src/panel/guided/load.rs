//! Opening a walk over the branch, what is not pushed or a pull request, and
//! reading the change into it.

use crate::git::guided::GuidedFile;
use crate::state::{AppState, Modal, PendingAction};

use super::{Guided, Loaded, OVERVIEW, Overview, Source, work};

/// Walk the branch on screen against main, or on main itself, what has not
/// been pushed yet.
pub fn open_branch(state: &mut AppState) {
    if state.repo_root.is_none() {
        state.set_status("no repository open to review", true);
        return;
    }
    let branch = state.branch.clone().unwrap_or_else(|| "HEAD".to_string());
    // On main the branch has nothing against itself, so what there is to
    // review is what has not been pushed yet.
    let base = crate::preferences::base_branch();
    let source = if branch == base {
        Source::Unpushed {
            branch,
            base_ref: String::new(),
        }
    } else {
        Source::Branch {
            branch,
            base_ref: base,
        }
    };
    open_local(state, source);
}

/// Walk the checkout as `source` has it, or go back to that walk if it is the
/// one already under way.
fn open_local(state: &mut AppState, source: Source) {
    if let Some(guided) = state.guided.as_ref()
        && guided.source.progress_key() == source.progress_key()
    {
        // Back to the walk already under way, rebuilt in case the code moved.
        state.modal = Modal::GuidedReview;
        reload(state);
        return;
    }
    let what = match &source {
        Source::Unpushed { .. } => "what is not pushed",
        _ => "the branch",
    };
    let mut guided = Guided::new(source);
    guided.loading = Some(work::load_local(&guided.source));
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state.set_status(format!("reading {what} for a guided review\u{2026}"), false);
}

/// Switch the walk between the whole branch against main and only what has
/// not been pushed yet. Each keeps its own progress and notes, so switching
/// back resumes where that walk stopped.
pub(super) fn switch_scope(state: &mut AppState) {
    let Some(guided) = state.guided.as_deref() else {
        return;
    };
    if guided.fixing.is_some() {
        state.set_status(
            "claude is still changing the code; switch once it is done",
            true,
        );
        return;
    }
    let base = crate::preferences::base_branch();
    let source = match &guided.source {
        Source::Branch { branch, .. } => Source::Unpushed {
            branch: branch.clone(),
            base_ref: String::new(),
        },
        Source::Unpushed { branch, .. } if *branch == base => {
            state.set_status(
                format!("{branch} is the base branch; there is no branch against it to review"),
                true,
            );
            return;
        }
        Source::Unpushed { branch, .. } => Source::Branch {
            branch: branch.clone(),
            base_ref: base,
        },
        Source::PullRequest { .. } => {
            state.set_status("a pull request is walked as GitHub has it", true);
            return;
        }
    };
    open_local(state, source);
}

/// Walk a pull request as GitHub has it.
pub fn open_pull_request(state: &mut AppState, pr: &crate::github::PullRequest) {
    // The number alone could be another repository's pull request: the inbox
    // opens reviews across the workspace.
    if let Some(guided) = state.guided.as_ref()
        && matches!(&guided.source, Source::PullRequest { number, .. } if *number == pr.number)
        && guided
            .pull_request
            .as_ref()
            .is_none_or(|open| open.url == pr.url)
    {
        state.modal = Modal::GuidedReview;
        return;
    }
    let local = state.branch.as_deref() == Some(pr.head.as_str());
    let mut guided = Guided::new(Source::PullRequest {
        number: pr.number,
        title: pr.title.clone(),
        head: pr.head.clone(),
        base: pr.base.clone(),
        commit: String::new(),
        local,
    });
    guided.loading = Some(work::load_pull_request(pr.clone(), local));
    guided.pull_request = Some(pr.clone());
    state.guided = Some(Box::new(guided));
    state.modal = Modal::GuidedReview;
    state.set_status(
        format!("reading #{} for a guided review\u{2026}", pr.number),
        false,
    );
}

/// Open the pull request under review on GitHub. A walk over local changes has
/// no page to open.
pub(super) fn open_in_browser(state: &mut AppState) {
    let url = state
        .guided
        .as_deref()
        .and_then(|guided| guided.pull_request.as_ref())
        .map(|pr| pr.url.clone())
        .filter(|url| !url.is_empty());
    match url {
        Some(url) => {
            state.pending_action = Some(PendingAction::GitHub(
                crate::state::GitHubAction::OpenInBrowser { url },
            ))
        }
        None => state.set_status("only a pull request has a page to open", false),
    }
}

/// Read the change again, keeping the place, the notes and every answer whose
/// hunk did not change.
pub fn reload(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if guided.loading.is_some() {
        return;
    }
    guided.loading = Some(match (&guided.source, guided.pull_request.clone()) {
        (Source::PullRequest { local, .. }, Some(pr)) => work::load_pull_request(pr, *local),
        (source, _) => work::load_local(source),
    });
}

pub(super) fn overview_of(
    title: &str,
    body: &str,
    commits: &[String],
    steps: &[GuidedFile],
) -> Overview {
    let mut out = format!("{title}\n");
    if !body.trim().is_empty() {
        out.push_str("\nDescription:\n");
        out.push_str(body.trim());
        out.push('\n');
    }
    if !commits.is_empty() {
        out.push_str("\nCommits:\n");
        for commit in commits {
            out.push_str(&format!("- {commit}\n"));
        }
    }
    out.push_str("\nFiles changed:\n");
    let mut files: Vec<(&str, usize, usize, Option<&str>)> = Vec::new();
    for step in steps {
        match files.last_mut() {
            Some((path, added, removed, _)) if *path == step.path => {
                *added += step.added();
                *removed += step.removed();
            }
            _ => files.push((
                &step.path,
                step.added(),
                step.removed(),
                step.file_note.as_deref(),
            )),
        }
    }
    let (mut text, mut brief) = (out.clone(), out);
    for (path, added, removed, note) in files {
        let note = note.map(|n| format!(" ({n})")).unwrap_or_default();
        text.push_str(&format!("- {path} +{added} -{removed}{note}\n"));
        brief.push_str(&format!("- {path}{note}\n"));
    }
    Overview { text, brief }
}

pub(super) fn apply_loaded(guided: &mut Guided, loaded: Loaded, first_load: bool) {
    let previous = guided.step().map(|step| (step.key(), step.path.clone()));
    let was_final = guided.on_final();
    // A fresh read keeps a pull request's commit, which only the load knows.
    guided.source = loaded.source;
    guided.steps = loaded.steps;
    guided.overview = loaded.overview.text;
    guided.brief = loaded.overview.brief;
    if was_final {
        guided.current = guided.final_step();
    } else if let Some((key, path)) = previous {
        // The same change if it is still there, else the same file, else
        // where the walk was.
        let index = guided
            .steps
            .iter()
            .position(|step| step.key() == key)
            .or_else(|| guided.steps.iter().position(|step| step.path == path));
        guided.current = match index {
            Some(index) => index + 1,
            None => guided.current.min(guided.total()),
        };
    } else if first_load && !guided.progress.reviewed.is_empty() {
        // A walk picked up again opens where it stopped; a new one opens on
        // the overview.
        guided.current = first_unreviewed(guided).unwrap_or(OVERVIEW);
    }
    guided.current = guided.current.min(guided.final_step());
    guided.cursor = guided.step().map(GuidedFile::first_changed).unwrap_or(0);
    guided.side_scroll = 0;
}

fn first_unreviewed(guided: &Guided) -> Option<usize> {
    guided
        .walk_order()
        .into_iter()
        .find(|&step| !guided.is_reviewed(&guided.steps[step - 1]))
}
