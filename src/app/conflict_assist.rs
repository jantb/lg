//! Trying the local model on a conflict before handing it to claude.
//!
//! A merge conflict is a decision, and most of them are small ones: two
//! branches touched neighbouring lines and both changes belong. Those are worth
//! a round trip to the model already running on this machine. The rest — a file
//! full of conflicts, a hunk the size of a function, two sides that decided
//! opposite things — are not, and the value of trying locally comes entirely
//! from knowing when to stop. So every conflict is measured before it is asked
//! about, every answer is checked before it is written, and anything that falls
//! short leaves the file exactly as git wrote it and goes to claude.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::git::{ConflictHunk, ConflictedFile, holds_conflict_marker};
use crate::state::{AppState, ConflictResolveJob, ConflictResolveMsg};

/// How many conflicts in one file the local model may try. Past this the file
/// is a merge rather than a hunk, and the odds of getting every one of them
/// right are not what they look like per conflict.
const MAX_LOCAL_HUNKS: usize = 6;

/// How many lines either side of a single conflict may span. A disagreement
/// this large is a design decision written down twice, not a merge.
const MAX_LOCAL_HUNK_LINES: usize = 40;

/// How many merged lines either side of a conflict travel with it, so the model
/// can see what the code around it is doing.
const HUNK_CONTEXT_LINES: usize = 12;

/// Hand every conflicted file to the local model, one at a time, in the
/// background.
pub(crate) fn spawn_conflict_resolve(state: &mut AppState) {
    if state.conflict_resolve_job.is_some() {
        state.set_status("the local model is already working on this", false);
        return;
    }
    let Some(root) = state.repo_root.clone() else {
        state.set_status("no repository to resolve conflicts in", true);
        return;
    };
    let paths = state.unresolved_conflicts();
    if paths.is_empty() {
        state.set_status("nothing left for the local model to resolve", false);
        return;
    }

    let total = paths.len();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || resolve_files(&PathBuf::from(root), paths, &tx));
    state.conflict_resolve_job = Some(ConflictResolveJob {
        rx,
        handle: Some(handle),
        spinner: 0,
        active_path: None,
        completed: 0,
        total,
    });
    state.set_status(
        format!("asking the local model to resolve {total} conflicted file(s)\u{2026}"),
        false,
    );
}

/// Walk the conflicted files, reporting each one as it is settled or given up
/// on. Stops early only if the receiver is gone, which means lg no longer cares.
fn resolve_files(root: &Path, paths: Vec<String>, tx: &Sender<ConflictResolveMsg>) {
    let total = paths.len();
    let mut resolved = Vec::new();
    let mut declined = Vec::new();

    for (index, path) in paths.into_iter().enumerate() {
        if tx
            .send(ConflictResolveMsg::Started {
                path: path.clone(),
                index: index + 1,
                total,
            })
            .is_err()
        {
            return;
        }
        let sent = match resolve_file(root, &path) {
            Ok(hunks) => {
                resolved.push(path.clone());
                tx.send(ConflictResolveMsg::Resolved { path, hunks })
            }
            Err(reason) => {
                declined.push(path.clone());
                tx.send(ConflictResolveMsg::Declined { path, reason })
            }
        };
        if sent.is_err() {
            return;
        }
    }

    let _ = tx.send(ConflictResolveMsg::Finished { resolved, declined });
}

/// Settle one file and write it back, or say why it was left alone. The file is
/// only ever written once every conflict in it has an accepted answer, so a
/// partial success leaves nothing half-merged on disk.
fn resolve_file(root: &Path, path: &str) -> Result<usize, String> {
    let full = root.join(path);
    let text =
        std::fs::read_to_string(&full).map_err(|err| format!("cannot read the file: {err}"))?;
    let (resolved, hunks) = resolve_conflicted_text(path, &text, &mut ask_local_model)?;
    std::fs::write(&full, resolved).map_err(|err| format!("cannot write the file back: {err}"))?;
    Ok(hunks)
}

/// What settles one conflict: the file it is in, the conflict, and the merged
/// text either side of it. Returning `Err` gives up on the whole file.
type Ask<'a> = dyn FnMut(&str, &ConflictHunk, String, String) -> Result<String, String> + 'a;

/// Settle every conflict in `text`, or say why the local model is the wrong
/// tool for this file. Reports the merged file and how many conflicts were in
/// it.
///
/// The gates come first and are cheap: a file past them costs nothing to
/// decline, and declining is not a failure — it is the answer that sends the
/// conflict to claude while it is still untouched.
fn resolve_conflicted_text(
    path: &str,
    text: &str,
    ask: &mut Ask<'_>,
) -> Result<(String, usize), String> {
    let Some(file) = ConflictedFile::parse(text) else {
        return Err("the conflict markers are not a shape lg can splice".to_string());
    };
    let count = file.hunk_count();
    if count > MAX_LOCAL_HUNKS {
        return Err(format!(
            "{count} conflicts in one file is more than the local model is trusted with"
        ));
    }

    let mut resolutions = Vec::with_capacity(count);
    for (index, hunk) in file.hunks().enumerate() {
        if let Some(agreed) = hunk.agreed_text() {
            resolutions.push(agreed);
            continue;
        }
        let widest = hunk.widest_side_lines();
        if widest > MAX_LOCAL_HUNK_LINES {
            return Err(format!(
                "conflict {} spans {widest} lines, too much of a decision for the local model",
                index + 1
            ));
        }
        let answer = ask(
            path,
            hunk,
            file.context_before(index, HUNK_CONTEXT_LINES),
            file.context_after(index, HUNK_CONTEXT_LINES),
        )?;
        resolutions.push(accept_resolution(hunk, &answer)?);
    }

    Ok((file.render(&resolutions), count))
}

/// Take the model's answer, or say what is wrong with it.
///
/// Everything here is a way the answer could be worse than no answer: a model
/// that gave up, one that echoed the markers back, one that dropped code
/// nobody asked it to drop, one that kept writing past the end of the merge.
/// A rejected answer costs one round trip; an accepted bad one costs a wrong
/// merge somebody has to find later.
fn accept_resolution(hunk: &ConflictHunk, answer: &str) -> Result<String, String> {
    let trimmed = answer.trim();
    if trimmed
        .to_ascii_uppercase()
        .starts_with(crate::llm::GIVE_UP_PHRASE)
    {
        return Err("the local model said it could not resolve this conflict".to_string());
    }
    if holds_conflict_marker(answer) {
        return Err("the local model left conflict markers in its answer".to_string());
    }
    if trimmed.is_empty() && !(hunk.ours.trim().is_empty() || hunk.theirs.trim().is_empty()) {
        return Err("the local model dropped both sides of the conflict".to_string());
    }
    let budget = hunk.resolution_budget_lines();
    let written = answer.lines().count();
    if written > budget {
        return Err(format!(
            "the local model wrote {written} lines where both sides together are {budget}"
        ));
    }
    Ok(answer.to_string())
}

/// Put one conflict to the local model and wait for the whole answer.
///
/// The stream is consumed here rather than shown, because a half-written merge
/// is not something to put on screen: the file changes only once the answer is
/// complete and has passed [`accept_resolution`].
fn ask_local_model(
    path: &str,
    hunk: &ConflictHunk,
    before: String,
    after: String,
) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::llm::stream_conflict_hunk(path.to_string(), hunk.clone(), before, after, tx);

    let mut answer = None;
    let mut error = None;
    for msg in rx {
        match msg {
            crate::state::GenMsg::Done(text) => answer = Some(text),
            crate::state::GenMsg::Error(message) => error = Some(message),
            crate::state::GenMsg::Thinking(_)
            | crate::state::GenMsg::Output(_)
            | crate::state::GenMsg::Reset => {}
        }
    }
    match (answer, error) {
        (_, Some(message)) => Err(message),
        (Some(answer), None) => Ok(answer),
        (None, None) => Err("the local model answered nothing".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFLICT: &str =
        "fn main() {\n<<<<<<< HEAD\n    a();\n=======\n    b();\n>>>>>>> them\n}\n";

    /// An `ask` that always answers the same thing.
    fn answering(
        answer: &str,
    ) -> impl FnMut(&str, &ConflictHunk, String, String) -> Result<String, String> + '_ {
        move |_, _, _, _| Ok(answer.to_string())
    }

    #[test]
    fn a_settled_conflict_is_spliced_into_the_file_git_wrote() {
        let (resolved, hunks) = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &mut answering("    a();\n    b();\n"),
        )
        .expect("a small conflict the model answered");

        assert_eq!(resolved, "fn main() {\n    a();\n    b();\n}\n");
        assert_eq!(hunks, 1);
    }

    #[test]
    fn the_conflict_and_the_code_around_it_reach_the_model() {
        let mut seen = None;
        let _ =
            resolve_conflicted_text("src/main.rs", CONFLICT, &mut |path, hunk, before, after| {
                seen = Some((
                    path.to_string(),
                    hunk.ours.clone(),
                    hunk.theirs.clone(),
                    before,
                    after,
                ));
                Ok("    a();\n".to_string())
            });

        let (path, ours, theirs, before, after) = seen.expect("the model was asked");
        assert_eq!(path, "src/main.rs");
        assert_eq!(ours, "    a();\n");
        assert_eq!(theirs, "    b();\n");
        assert_eq!(before, "fn main() {\n");
        assert_eq!(after, "}\n");
    }

    #[test]
    fn a_conflict_both_sides_wrote_the_same_way_needs_no_model() {
        let text = "<<<<<<< HEAD\nsame\n=======\nsame\n>>>>>>> them\n";
        let mut asked = false;
        let (resolved, _) = resolve_conflicted_text("doc.md", text, &mut |_, _, _, _| {
            asked = true;
            Ok(String::new())
        })
        .expect("nothing to decide");

        assert_eq!(resolved, "same\n");
        assert!(!asked, "there was no decision to put to a model");
    }

    #[test]
    fn a_file_full_of_conflicts_goes_straight_on_without_being_asked_about() {
        let text = "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> them\n".repeat(MAX_LOCAL_HUNKS + 1);
        let mut asked = false;
        let outcome = resolve_conflicted_text("src/main.rs", &text, &mut |_, _, _, _| {
            asked = true;
            Ok("a\n".to_string())
        });

        assert!(outcome.is_err(), "too many conflicts to try locally");
        assert!(!asked, "declining early is the point of the gate");
    }

    #[test]
    fn a_conflict_too_large_to_be_a_merge_is_declined_before_it_is_asked_about() {
        let side = "line\n".repeat(MAX_LOCAL_HUNK_LINES + 1);
        let text = format!("<<<<<<< HEAD\n{side}=======\nother\n>>>>>>> them\n");
        let mut asked = false;
        let outcome = resolve_conflicted_text("src/main.rs", &text, &mut |_, _, _, _| {
            asked = true;
            Ok("line\n".to_string())
        });

        assert!(outcome.is_err());
        assert!(!asked);
    }

    #[test]
    fn a_model_that_says_it_cannot_resolve_is_believed() {
        let outcome = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &mut answering(crate::llm::GIVE_UP_PHRASE),
        );

        assert!(outcome.is_err(), "an admission is not a resolution");
    }

    #[test]
    fn an_answer_with_a_marker_left_in_it_is_refused() {
        let outcome = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &mut answering("<<<<<<< HEAD\n    a();\n"),
        );

        assert!(outcome.is_err());
    }

    #[test]
    fn an_answer_that_keeps_writing_past_the_merge_is_refused() {
        let outcome = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &mut answering(&"filler\n".repeat(64)),
        );

        assert!(outcome.is_err());
    }

    #[test]
    fn an_answer_that_silently_deletes_both_sides_is_refused() {
        let outcome = resolve_conflicted_text("src/main.rs", CONFLICT, &mut answering(""));

        assert!(outcome.is_err());
    }

    /// One side deleting what the other kept is a real merge result, and the
    /// empty answer is the right one for it.
    #[test]
    fn a_deletion_is_accepted_when_one_side_deleted() {
        let text = "keep\n<<<<<<< HEAD\ngone\n=======\n>>>>>>> them\nkeep\n";
        let (resolved, _) = resolve_conflicted_text("src/main.rs", text, &mut answering(""))
            .expect("deleting what one side deleted");

        assert_eq!(resolved, "keep\nkeep\n");
    }

    #[test]
    fn a_model_that_errors_leaves_the_file_alone() {
        let outcome = resolve_conflicted_text("src/main.rs", CONFLICT, &mut |_, _, _, _| {
            Err("mtplx request: connection refused".to_string())
        });

        assert_eq!(
            outcome.unwrap_err(),
            "mtplx request: connection refused",
            "the reason has to survive so the panel can say why claude was called"
        );
    }

    #[test]
    fn every_conflict_in_a_file_has_to_be_settled_for_any_of_it_to_count() {
        let text = "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> them\nmid\n<<<<<<< HEAD\nc\n=======\nd\n>>>>>>> them\n";
        let mut answers = ["a\n".to_string(), crate::llm::GIVE_UP_PHRASE.to_string()].into_iter();
        let outcome = resolve_conflicted_text("src/main.rs", text, &mut |_, _, _, _| {
            Ok(answers.next().unwrap())
        });

        assert!(
            outcome.is_err(),
            "a file is written whole or not at all, so one refusal declines the file"
        );
    }
}
