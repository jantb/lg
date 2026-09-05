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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::git::{ConflictHunk, ConflictSides, ConflictedFile, holds_conflict_marker};
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
    let handle = std::thread::spawn(move || {
        resolve_files(&PathBuf::from(root), paths, &tx, &mut ask_local_model);
    });
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

/// Why a file was left for claude.
///
/// A refusal is about this one conflict; an outage is about every conflict
/// still in the queue. Keeping them apart is what stops a stopped model server
/// from being asked once per conflicted file, each time to wait out the same
/// refused connection.
#[derive(Debug)]
enum Decline {
    /// The local model is the wrong tool for this file.
    Refused(String),
    /// The model server is not answering, so nothing else in the queue will
    /// fare any better.
    Unavailable(String),
}

impl Decline {
    fn reason(&self) -> &str {
        match self {
            Self::Refused(reason) | Self::Unavailable(reason) => reason,
        }
    }
}

/// Walk the conflicted files, reporting each one as it is settled or given up
/// on.
///
/// Stops early if the receiver is gone, which means lg no longer cares, or if
/// the model server stops answering — the files left over are handed on
/// untouched, which is where they were going anyway.
fn resolve_files(
    root: &Path,
    paths: Vec<String>,
    tx: &Sender<ConflictResolveMsg>,
    ask: &mut Ask<'_>,
) {
    let total = paths.len();
    let mut resolved = Vec::new();
    let mut declined: Vec<(String, String)> = Vec::new();
    let mut remaining = paths.into_iter().enumerate();

    while let Some((index, path)) = remaining.next() {
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
        let outcome = resolve_file(root, &path, ask);
        let unavailable = matches!(outcome, Err(Decline::Unavailable(_)));
        let sent = match outcome {
            Ok(hunks) => {
                log_attempt(&format!(
                    "file {path}: resolved, {hunks} conflict(s) written back"
                ));
                resolved.push(path.clone());
                tx.send(ConflictResolveMsg::Resolved { path, hunks })
            }
            Err(decline) => {
                let reason = decline.reason().to_string();
                log_attempt(&format!("file {path}: declined: {reason}"));
                declined.push((path.clone(), reason.clone()));
                tx.send(ConflictResolveMsg::Declined { path, reason })
            }
        };
        if sent.is_err() {
            return;
        }
        if unavailable {
            for (_, path) in remaining {
                let reason = "the model server stopped answering".to_string();
                declined.push((path.clone(), reason.clone()));
                if tx
                    .send(ConflictResolveMsg::Declined { path, reason })
                    .is_err()
                {
                    return;
                }
            }
            break;
        }
    }

    let _ = tx.send(ConflictResolveMsg::Finished { resolved, declined });
}

/// Settle one file and write it back, or say why it was left alone. The file is
/// only ever written once every conflict in it has an accepted answer, so a
/// partial success leaves nothing half-merged on disk.
fn resolve_file(root: &Path, path: &str, ask: &mut Ask<'_>) -> Result<usize, Decline> {
    let full = root.join(path);
    let text = std::fs::read_to_string(&full)
        .map_err(|err| Decline::Refused(format!("cannot read the file: {err}")))?;
    let sides = crate::git::conflict_sides(root, path);
    let (resolved, hunks) = resolve_conflicted_text(path, &text, &sides, ask)?;
    std::fs::write(&full, resolved)
        .map_err(|err| Decline::Refused(format!("cannot write the file back: {err}")))?;
    Ok(hunks)
}

/// What the model sent back for one conflict, and whether it got to the end of
/// it. A merge the server stopped at the budget is text that looks finished and
/// is not, so the two travel together.
struct Answer {
    text: String,
    truncated: bool,
}

/// Everything the model is shown about one conflict: the file it is in, the
/// conflict, where each side came from, and the merged text either side of it.
pub(crate) struct HunkQuestion<'a> {
    pub path: &'a str,
    pub hunk: &'a ConflictHunk,
    pub sides: &'a ConflictSides,
    pub before: String,
    pub after: String,
}

/// What settles one conflict. Returning `Err` gives up on the whole file.
type Ask<'a> = dyn FnMut(&HunkQuestion<'_>) -> Result<Answer, Decline> + 'a;

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
    sides: &ConflictSides,
    ask: &mut Ask<'_>,
) -> Result<(String, usize), Decline> {
    let Some(file) = ConflictedFile::parse(text) else {
        return Err(Decline::Refused(
            "the conflict markers are not a shape lg can splice".to_string(),
        ));
    };
    let count = file.hunk_count();
    if count > MAX_LOCAL_HUNKS {
        return Err(Decline::Refused(format!(
            "{count} conflicts in one file is more than the local model is trusted with"
        )));
    }

    let mut resolutions = Vec::with_capacity(count);
    for (index, hunk) in file.hunks().enumerate() {
        if let Some(agreed) = hunk.agreed_text() {
            resolutions.push(agreed);
            continue;
        }
        let widest = hunk.widest_side_lines();
        if widest > MAX_LOCAL_HUNK_LINES {
            return Err(Decline::Refused(format!(
                "conflict {} spans {widest} lines, too much of a decision for the local model",
                index + 1
            )));
        }
        let answer = ask(&HunkQuestion {
            path,
            hunk,
            sides,
            before: file.context_before(index, HUNK_CONTEXT_LINES),
            after: file.context_after(index, HUNK_CONTEXT_LINES),
        })?;
        let verdict = accept_resolution(hunk, &answer);
        log_attempt(&format!(
            "file {path} conflict {}: {}",
            index + 1,
            match &verdict {
                Ok(_) => "answer accepted".to_string(),
                Err(decline) => format!("answer refused: {}", decline.reason()),
            }
        ));
        resolutions.push(verdict?);
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
fn accept_resolution(hunk: &ConflictHunk, answer: &Answer) -> Result<String, Decline> {
    let text = answer.text.as_str();
    let trimmed = text.trim();
    // A truncated merge is checked for first because every other gate here is
    // shaped to catch an answer that is too much, and this is the one that is
    // too little: the model stopped mid-merge because it ran out of budget, so
    // what it wrote is the first half of a resolution. It passes the length
    // gate precisely because it is short, and nothing in the text says it is
    // unfinished — only the server does.
    if answer.truncated {
        return Err(Decline::Refused(
            "the local model ran out of budget part-way through the merge".to_string(),
        ));
    }
    if trimmed
        .to_ascii_uppercase()
        .starts_with(crate::llm::GIVE_UP_PHRASE)
    {
        return Err(Decline::Refused(
            "the local model said it could not resolve this conflict".to_string(),
        ));
    }
    if holds_conflict_marker(text) {
        return Err(Decline::Refused(
            "the local model left conflict markers in its answer".to_string(),
        ));
    }
    if trimmed.is_empty() && !(hunk.ours.trim().is_empty() || hunk.theirs.trim().is_empty()) {
        return Err(Decline::Refused(
            "the local model dropped both sides of the conflict".to_string(),
        ));
    }
    let budget = hunk.resolution_budget_lines();
    let written = text.lines().count();
    if written > budget {
        return Err(Decline::Refused(format!(
            "the local model wrote {written} lines where both sides together are {budget}"
        )));
    }
    Ok(hunk.restore_indent(text))
}

/// Put one conflict to the local model and wait for the whole answer.
///
/// The stream is consumed here rather than shown, because a half-written merge
/// is not something to put on screen: the file changes only once the answer is
/// complete and has passed [`accept_resolution`].
fn ask_local_model(question: &HunkQuestion<'_>) -> Result<Answer, Decline> {
    let prompt = crate::llm::build_conflict_hunk_prompt(
        question.path,
        question.hunk,
        question.sides,
        &question.before,
        &question.after,
    );
    log_attempt(&format!(
        "file {}: asking the local model\n--- prompt ---\n{prompt}--- end prompt ---",
        question.path
    ));
    let (tx, rx) = std::sync::mpsc::channel();
    crate::llm::stream_conflict_hunk(prompt, tx);

    let mut text = None;
    let mut truncated = false;
    let mut error = None;
    for msg in rx {
        match msg {
            crate::state::GenMsg::Done { text: done, stats } => {
                text = Some(done);
                truncated = stats.truncated;
            }
            crate::state::GenMsg::Error(message) => error = Some(message),
            crate::state::GenMsg::Thinking(_)
            | crate::state::GenMsg::Output(_)
            | crate::state::GenMsg::Reset => {}
        }
    }
    log_attempt(&format!(
        "file {}: model answered (truncated: {truncated}, error: {})\n--- answer ---\n{}--- end answer ---",
        question.path,
        error.as_deref().unwrap_or("none"),
        text.as_deref().unwrap_or("")
    ));
    match (text, error) {
        // A server that could not be reached will not be reached for the next
        // file either. A server that answered and turned this request down —
        // a prompt too large, a session still busy — says nothing about the
        // next one, so only this file is declined.
        (_, Some(message)) if crate::llm::error_means_unreachable(&message) => {
            Err(Decline::Unavailable(message))
        }
        (_, Some(message)) => Err(Decline::Refused(message)),
        (Some(text), None) => Ok(Answer { text, truncated }),
        (None, None) => Err(Decline::Refused(
            "the local model answered nothing".to_string(),
        )),
    }
}

/// Write one line of what the local model was asked and what it said to the
/// conflict log, so a merge that went wrong can be read back afterwards.
///
/// The status line shows a decline for a moment and the panel shows the reason,
/// but neither keeps the prompt or the answer, and those are what say whether
/// the model, the gates, or the prompt were at fault. Failing to write the log
/// is not worth stopping a merge over, so errors are dropped.
fn log_attempt(entry: &str) {
    // The tests drive the resolver with made-up conflicts, and a merge that
    // never happened has no place in the log a person reads to find out what
    // went wrong with one that did.
    if cfg!(test) {
        return;
    }
    let Ok(path) = crate::settings::conflict_resolve_log_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // One write per entry, so entries from files resolved back to back do not
    // interleave.
    let _ = file.write_all(format!("[{stamp}] {entry}\n").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFLICT: &str =
        "fn main() {\n<<<<<<< HEAD\n    a();\n=======\n    b();\n>>>>>>> them\n}\n";

    /// An `ask` that always answers the same thing, and got to finish.
    fn no_sides() -> ConflictSides {
        ConflictSides {
            ours: None,
            theirs: None,
        }
    }

    fn answering(answer: &str) -> impl FnMut(&HunkQuestion<'_>) -> Result<Answer, Decline> + '_ {
        move |_| {
            Ok(Answer {
                text: answer.to_string(),
                truncated: false,
            })
        }
    }

    /// An `ask` whose answer the server stopped at the token budget.
    fn answering_truncated(
        answer: &str,
    ) -> impl FnMut(&HunkQuestion<'_>) -> Result<Answer, Decline> + '_ {
        move |_| {
            Ok(Answer {
                text: answer.to_string(),
                truncated: true,
            })
        }
    }

    fn finished(text: &str) -> Result<Answer, Decline> {
        Ok(Answer {
            text: text.to_string(),
            truncated: false,
        })
    }

    #[test]
    fn a_settled_conflict_is_spliced_into_the_file_git_wrote() {
        let (resolved, hunks) = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &no_sides(),
            &mut answering("    a();\n    b();\n"),
        )
        .expect("a small conflict the model answered");

        assert_eq!(resolved, "fn main() {\n    a();\n    b();\n}\n");
        assert_eq!(hunks, 1);
    }

    #[test]
    fn the_conflict_and_the_code_around_it_reach_the_model() {
        let mut seen = None;
        let _ = resolve_conflicted_text("src/main.rs", CONFLICT, &no_sides(), &mut |q| {
            seen = Some((
                q.path.to_string(),
                q.hunk.ours.clone(),
                q.hunk.theirs.clone(),
                q.before.clone(),
                q.after.clone(),
            ));
            finished("    a();\n")
        });

        let (path, ours, theirs, before, after) = seen.expect("the model was asked");
        assert_eq!(path, "src/main.rs");
        assert_eq!(ours, "    a();\n");
        assert_eq!(theirs, "    b();\n");
        assert_eq!(before, "fn main() {\n");
        assert_eq!(after, "}\n");
    }

    /// What the local model actually did with alv-no's image tag: it chose a
    /// side and returned the line flush left. The file has to come back with
    /// the tag where git had it.
    #[test]
    fn a_side_returned_without_its_indent_is_spliced_in_with_it() {
        let text = "    image:\n<<<<<<< HEAD\n      tag: \"sha-0e337ed\"\n=======\n      tag: \"sha-1685629\"\n>>>>>>> origin/main\n    config:\n";
        let (resolved, _) = resolve_conflicted_text(
            ".halvnais/app.yaml",
            text,
            &no_sides(),
            &mut answering("tag: \"sha-1685629\"\n"),
        )
        .expect("a picked side is a resolution");

        assert_eq!(
            resolved,
            "    image:\n      tag: \"sha-1685629\"\n    config:\n"
        );
    }

    #[test]
    fn a_conflict_both_sides_wrote_the_same_way_needs_no_model() {
        let text = "<<<<<<< HEAD\nsame\n=======\nsame\n>>>>>>> them\n";
        let mut asked = false;
        let (resolved, _) = resolve_conflicted_text("doc.md", text, &no_sides(), &mut |_| {
            asked = true;
            finished("")
        })
        .expect("nothing to decide");

        assert_eq!(resolved, "same\n");
        assert!(!asked, "there was no decision to put to a model");
    }

    #[test]
    fn a_file_full_of_conflicts_goes_straight_on_without_being_asked_about() {
        let text = "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> them\n".repeat(MAX_LOCAL_HUNKS + 1);
        let mut asked = false;
        let outcome = resolve_conflicted_text("src/main.rs", &text, &no_sides(), &mut |_| {
            asked = true;
            finished("a\n")
        });

        assert!(outcome.is_err(), "too many conflicts to try locally");
        assert!(!asked, "declining early is the point of the gate");
    }

    #[test]
    fn a_conflict_too_large_to_be_a_merge_is_declined_before_it_is_asked_about() {
        let side = "line\n".repeat(MAX_LOCAL_HUNK_LINES + 1);
        let text = format!("<<<<<<< HEAD\n{side}=======\nother\n>>>>>>> them\n");
        let mut asked = false;
        let outcome = resolve_conflicted_text("src/main.rs", &text, &no_sides(), &mut |_| {
            asked = true;
            finished("line\n")
        });

        assert!(outcome.is_err());
        assert!(!asked);
    }

    #[test]
    fn a_model_that_says_it_cannot_resolve_is_believed() {
        let outcome = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &no_sides(),
            &mut answering(crate::llm::GIVE_UP_PHRASE),
        );

        assert!(outcome.is_err(), "an admission is not a resolution");
    }

    #[test]
    fn an_answer_with_a_marker_left_in_it_is_refused() {
        let outcome = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &no_sides(),
            &mut answering("<<<<<<< HEAD\n    a();\n"),
        );

        assert!(outcome.is_err());
    }

    #[test]
    fn an_answer_that_keeps_writing_past_the_merge_is_refused() {
        let outcome = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &no_sides(),
            &mut answering(&"filler\n".repeat(64)),
        );

        assert!(outcome.is_err());
    }

    #[test]
    fn an_answer_that_silently_deletes_both_sides_is_refused() {
        let outcome =
            resolve_conflicted_text("src/main.rs", CONFLICT, &no_sides(), &mut answering(""));

        assert!(outcome.is_err());
    }

    /// The gates all catch an answer that is too much. This one is too little:
    /// the model stopped mid-merge because it ran out of budget, so the text is
    /// the first half of a resolution — short enough to pass the length gate,
    /// with nothing in it that says it is unfinished.
    #[test]
    fn a_merge_the_server_cut_off_is_never_written_to_the_file() {
        let outcome = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &no_sides(),
            &mut answering_truncated("    a();\n    b("),
        );

        assert!(
            outcome.is_err(),
            "half a merge on disk is worse than a conflict claude can still see"
        );
    }

    /// The same text, arriving complete, is a resolution. Truncation is the
    /// only difference between the two.
    #[test]
    fn the_same_answer_is_accepted_when_the_model_got_to_finish() {
        let (resolved, _) = resolve_conflicted_text(
            "src/main.rs",
            CONFLICT,
            &no_sides(),
            &mut answering("    a();\n"),
        )
        .expect("a finished answer is a resolution");

        assert_eq!(resolved, "fn main() {\n    a();\n}\n");
    }

    /// A stopped model server is not a verdict about one file, so asking it
    /// again for each of the others only spends the same refused connection
    /// over and over. Everything left goes to claude, which is where a declined
    /// file was going anyway.
    #[test]
    fn a_server_that_stops_answering_ends_the_pass_instead_of_being_asked_again() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ["a.rs", "b.rs", "c.rs"];
        for path in paths {
            std::fs::write(dir.path().join(path), CONFLICT).unwrap();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut asked = 0usize;

        resolve_files(
            dir.path(),
            paths.iter().map(|path| (*path).to_string()).collect(),
            &tx,
            &mut |_| {
                asked += 1;
                Err(Decline::Unavailable("connection refused".to_string()))
            },
        );
        drop(tx);

        assert_eq!(asked, 1, "the server was asked once, not once per file");
        let declined = rx
            .iter()
            .find_map(|msg| match msg {
                ConflictResolveMsg::Finished { declined, .. } => Some(declined),
                _ => None,
            })
            .expect("the pass reported what it came to");
        assert_eq!(
            declined.len(),
            paths.len(),
            "every file still has to reach claude"
        );
    }

    /// One file the model is the wrong tool for says nothing about the next.
    #[test]
    fn a_refused_file_does_not_stop_the_ones_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ["a.rs", "b.rs"];
        for path in paths {
            std::fs::write(dir.path().join(path), CONFLICT).unwrap();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut asked = 0usize;

        resolve_files(
            dir.path(),
            paths.iter().map(|path| (*path).to_string()).collect(),
            &tx,
            &mut |_| {
                asked += 1;
                Err(Decline::Refused("not a merge".to_string()))
            },
        );
        drop(tx);

        assert_eq!(asked, paths.len());
        assert!(rx.iter().any(|msg| matches!(
            msg,
            ConflictResolveMsg::Finished { ref declined, .. } if declined.len() == paths.len()
        )));
    }

    /// One side deleting what the other kept is a real merge result, and the
    /// empty answer is the right one for it.
    #[test]
    fn a_deletion_is_accepted_when_one_side_deleted() {
        let text = "keep\n<<<<<<< HEAD\ngone\n=======\n>>>>>>> them\nkeep\n";
        let (resolved, _) =
            resolve_conflicted_text("src/main.rs", text, &no_sides(), &mut answering(""))
                .expect("deleting what one side deleted");

        assert_eq!(resolved, "keep\nkeep\n");
    }

    #[test]
    fn a_model_that_errors_leaves_the_file_alone() {
        let outcome = resolve_conflicted_text("src/main.rs", CONFLICT, &no_sides(), &mut |_| {
            Err(Decline::Unavailable(
                "mtplx request: connection refused".to_string(),
            ))
        });

        let decline = outcome.expect_err("a file nothing answered for");
        assert_eq!(
            decline.reason(),
            "mtplx request: connection refused",
            "the reason has to survive so the panel can say why claude was called"
        );
        assert!(
            matches!(decline, Decline::Unavailable(_)),
            "a server that is not answering is not a verdict about this file"
        );
    }

    #[test]
    fn every_conflict_in_a_file_has_to_be_settled_for_any_of_it_to_count() {
        let text = "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> them\nmid\n<<<<<<< HEAD\nc\n=======\nd\n>>>>>>> them\n";
        let mut answers = ["a\n".to_string(), crate::llm::GIVE_UP_PHRASE.to_string()].into_iter();
        let outcome = resolve_conflicted_text("src/main.rs", text, &no_sides(), &mut |_| {
            finished(&answers.next().unwrap())
        });

        assert!(
            outcome.is_err(),
            "a file is written whole or not at all, so one refusal declines the file"
        );
    }
}
