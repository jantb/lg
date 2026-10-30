//! The work a guided review hands to other threads: reading the change, the
//! model's commentary and answers, and Claude's fixes. Each answers on a
//! channel of its own, which [`super::poll::poll`] reads.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use super::{Loaded, Source};
use crate::llm::GenStats;
use crate::state::{GenMsg, stopped_unexpectedly};

/// Said to the model about a walk over the whole codebase, where the diff
/// shows every file as added although none of it is new.
const CODEBASE_DESCRIPTION: &str = "This is not a change: it is the whole codebase as it \
    stands, every file shown as added. Review it as existing code: correctness, design, tests \
    and what is risky to touch, not what a change did.";

/// Read a walk over the checkout itself: the branch against main, what is
/// not pushed yet, or the whole codebase.
pub(super) fn load_local(source: &Source) -> Receiver<Result<Loaded, String>> {
    let source = source.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let diff = match source {
            Source::Unpushed { .. } => crate::git::unpushed_diff(),
            Source::Codebase { .. } => crate::git::codebase_diff(),
            _ => crate::git::branch_diff_against_main(),
        };
        let result = diff
            .map(|diff| {
                let steps = crate::git::guided::parse_files(&diff.diff);
                let (title, description, source) = match source {
                    Source::Unpushed { .. } => (
                        format!(
                            "What is not pushed on {} (against {})",
                            diff.branch, diff.base_ref
                        ),
                        "",
                        Source::Unpushed {
                            branch: diff.branch,
                            base_ref: diff.base_ref,
                        },
                    ),
                    Source::Codebase { .. } => (
                        format!("The whole codebase on {}", diff.branch),
                        CODEBASE_DESCRIPTION,
                        Source::Codebase {
                            branch: diff.branch,
                        },
                    ),
                    _ => (
                        format!("Branch {} against {}", diff.branch, diff.base_ref),
                        "",
                        Source::Branch {
                            branch: diff.branch,
                            base_ref: diff.base_ref,
                        },
                    ),
                };
                let overview = super::load::overview_of(&title, description, &diff.commits, &steps);
                Loaded {
                    source,
                    steps,
                    overview,
                }
            })
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

pub(super) fn load_pull_request(
    pr: crate::github::PullRequest,
    local: bool,
) -> Receiver<Result<Loaded, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = (|| -> anyhow::Result<Loaded> {
            let diff = crate::github::pr_diff(pr.number)?;
            let commit = crate::github::pr_head(pr.number)?;
            let steps = crate::git::guided::parse_files(&diff);
            let overview = super::load::overview_of(
                &format!(
                    "Pull request #{} {} ({} into {})",
                    pr.number, pr.title, pr.head, pr.base
                ),
                &pr.body,
                &[],
                &steps,
            );
            Ok(Loaded {
                source: Source::PullRequest {
                    number: pr.number,
                    title: pr.title,
                    head: pr.head,
                    base: pr.base,
                    commit,
                    local,
                },
                steps,
                overview,
            })
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

/// The model's opening read of the whole change.
pub(super) fn ask_overview(contexts: Vec<String>, at_once: usize) -> Receiver<GenMsg> {
    ask_in_parts(contexts, at_once, crate::llm::stream_guided_overview)
}

pub(super) fn ask_step(overview: String, file: String) -> Receiver<GenMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || crate::llm::stream_guided_step(overview, file, tx));
    rx
}

/// The model's read of the whole change across files, once the walk is through them.
pub(super) fn ask_final(contexts: Vec<String>, at_once: usize) -> Receiver<GenMsg> {
    ask_in_parts(contexts, at_once, crate::llm::stream_guided_final)
}

/// Each of `contexts` asked of the model with `ask`, `at_once` at a time, and
/// answered as one. A single part streams as it is written. Several are put
/// together in order once every part is in, each part's answer sent on as it
/// comes so the read is seen to grow; a part that fails says so in the answer,
/// and only every part failing fails the read.
pub(super) fn ask_in_parts<F>(contexts: Vec<String>, at_once: usize, ask: F) -> Receiver<GenMsg>
where
    F: Fn(String, Sender<GenMsg>) + Send + Sync + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    if contexts.len() <= 1 {
        let context = contexts.into_iter().next().unwrap_or_default();
        crate::git::spawn_pinned(move || ask(context, tx));
        return rx;
    }
    crate::git::spawn_pinned(move || {
        let count = contexts.len();
        let queue = Arc::new(Mutex::new(
            contexts.into_iter().enumerate().collect::<Vec<_>>(),
        ));
        let ask = Arc::new(ask);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        for _ in 0..at_once.clamp(1, count) {
            let (queue, ask, done_tx, tx) =
                (queue.clone(), ask.clone(), done_tx.clone(), tx.clone());
            crate::git::spawn_pinned(move || {
                loop {
                    let next = queue.lock().ok().and_then(|mut queue| queue.pop());
                    let Some((at, context)) = next else {
                        break;
                    };
                    let _ = done_tx.send((at, answer_of(&*ask, context, &tx)));
                }
            });
        }
        drop(done_tx);
        let mut answers: Vec<Option<Result<(String, GenStats), String>>> = vec![None; count];
        for (at, answer) in done_rx {
            if let Ok((text, _)) = &answer {
                let _ = tx.send(GenMsg::Output(format!("{text}\n")));
            }
            answers[at] = Some(answer);
        }
        let mut texts = Vec::new();
        let mut stats = GenStats::default();
        let mut failed = None;
        let mut failures = 0;
        for (at, answer) in answers.into_iter().enumerate() {
            match answer {
                Some(Ok((text, part))) => {
                    texts.push(text);
                    stats.prompt_tokens += part.prompt_tokens;
                    stats.cached_tokens += part.cached_tokens;
                    stats.completion_tokens += part.completion_tokens;
                    stats.truncated |= part.truncated;
                    stats.served_model = stats.served_model.or(part.served_model);
                }
                answer => {
                    let message = match answer {
                        Some(Err(message)) => message,
                        _ => "it never answered".to_string(),
                    };
                    texts.push(format!(
                        "Part {} of {count} could not be read: {message}",
                        at + 1
                    ));
                    failed.get_or_insert(message);
                    failures += 1;
                }
            }
        }
        let message = match failed {
            Some(message) if failures == count => GenMsg::Error(message),
            _ => GenMsg::Done {
                text: super::findings::combine(&texts),
                stats,
            },
        };
        let _ = tx.send(message);
    });
    rx
}

/// What one part's read came to, its thinking passed on to `tx` meanwhile.
fn answer_of<F>(ask: &F, context: String, tx: &Sender<GenMsg>) -> Result<(String, GenStats), String>
where
    F: Fn(String, Sender<GenMsg>),
{
    let (part_tx, part_rx) = std::sync::mpsc::channel();
    ask(context, part_tx);
    let mut answer = Err(stopped_unexpectedly("the model request"));
    for message in part_rx {
        match message {
            GenMsg::Thinking(_) => {
                let _ = tx.send(GenMsg::Thinking(String::new()));
            }
            GenMsg::Done { text, stats } => answer = Ok((text, stats)),
            GenMsg::Error(message) => answer = Err(message),
            GenMsg::Output(_) | GenMsg::Reset => {}
        }
    }
    answer
}

/// The model's answer to a question about a step.
pub(super) fn ask_question(
    context: String,
    commentary: String,
    question: String,
) -> Receiver<GenMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        crate::llm::stream_guided_question(context, commentary, question, tx)
    });
    rx
}

/// Claude Code reading the checkout at `root` to settle the questions
/// `prompt` carries: the findings that still matter, one a line.
pub(super) fn check_questions(root: String, prompt: String) -> Receiver<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = crate::llm::run_claude_check(std::path::Path::new(&root), &prompt)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

/// Claude Code making the change `prompt` describes in the checkout at
/// `root`: what it said it did, or why it could not.
pub(super) fn run_fix(root: String, prompt: String) -> Receiver<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::git::spawn_pinned(move || {
        let result = crate::llm::run_claude_fix(std::path::Path::new(&root), &prompt)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::guided::{Commentary, Verdict, findings};

    fn answer(rx: Receiver<GenMsg>) -> Result<String, String> {
        for message in rx {
            match message {
                GenMsg::Done { text, .. } => return Ok(text),
                GenMsg::Error(message) => return Err(message),
                _ => {}
            }
        }
        Err("the read never finished".to_string())
    }

    /// A model that finds one thing in each part, and an issue in `bad`;
    /// a part named `broken` fails.
    fn read_parts(parts: &[&str]) -> Result<String, String> {
        let contexts = parts.iter().map(|part| part.to_string()).collect();
        answer(ask_in_parts(contexts, 2, |part, tx| {
            let message = if part == "broken" {
                GenMsg::Error("the server went away".to_string())
            } else {
                let verdict = if part == "bad" { "issue" } else { "ok" };
                GenMsg::Done {
                    text: format!(
                        "Summary: Read {part}.\n{part}.rs:L1 nit: something in {part}\nVerdict: {verdict}"
                    ),
                    stats: GenStats::default(),
                }
            };
            let _ = tx.send(message);
        }))
    }

    #[test]
    fn a_read_in_parts_answers_as_one_with_the_worst_verdict() {
        let text = read_parts(&["fine", "bad", "also_fine"]).unwrap();

        let reading = findings::read(&text);
        for part in ["fine", "bad", "also_fine"] {
            assert!(
                reading
                    .cited
                    .iter()
                    .any(|(path, _)| *path == format!("{part}.rs")),
                "{text}"
            );
            assert!(
                reading
                    .summary
                    .as_deref()
                    .unwrap_or_default()
                    .contains(part),
                "{text}"
            );
        }
        let commentary = Commentary {
            text,
            done: true,
            ..Default::default()
        };
        assert_eq!(commentary.verdict(), Some(Verdict::Issue));
    }

    #[test]
    fn a_part_that_fails_is_said_and_the_other_parts_still_count() {
        let text = read_parts(&["fine", "broken", "also_fine"]).unwrap();

        assert!(text.contains("the server went away"), "{text}");
        let reading = findings::read(&text);
        assert!(reading.cited.iter().any(|(path, _)| path == "also_fine.rs"));
    }

    #[test]
    fn an_answer_that_reads_like_a_failure_still_counts() {
        let contexts = vec!["broken".to_string(), "fine".to_string()];
        let read = answer(ask_in_parts(contexts, 2, |part, tx| {
            let message = if part == "broken" {
                GenMsg::Error("the server went away".to_string())
            } else {
                GenMsg::Done {
                    text: "Part of the module is fine.\nVerdict: ok".to_string(),
                    stats: GenStats::default(),
                }
            };
            let _ = tx.send(message);
        }));

        assert!(read.unwrap().contains("Part of the module is fine."));
    }

    #[test]
    fn every_part_failing_fails_the_read() {
        assert!(read_parts(&["broken", "broken"]).is_err());
    }
}
