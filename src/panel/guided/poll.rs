//! Each tick: the change being read, the model's answers streaming in, the
//! questions out with Claude, and the reads to ask for next.

use std::collections::HashSet;

use crate::state::{AppState, GenMsg, PendingAction, Poll, poll_once, stopped_unexpectedly};

use super::chat::tell_chat_what_is_read;
use super::context::{file_context, final_contexts, plan_contexts, step_context};
use super::load::{apply_loaded, reload};
use super::{
    Ask, CONCURRENT_CHECKS, CONCURRENT_CLAUDE, CONCURRENT_LOCAL, Check, Commentary, Guided, Mode,
    OVERVIEW, OVERVIEW_KEY, findings, work,
};

/// Take in whatever the background work produced since the last frame, and
/// start the commentary the reviewer is about to want.
pub fn poll(state: &mut AppState) {
    let ai = state.ai_assist;
    let root = state.repo_root.clone();
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let mut notices: Vec<(String, bool)> = Vec::new();

    if let Some(rx) = guided.loading.as_ref() {
        match poll_once(rx) {
            Poll::Message(Ok(loaded)) => {
                guided.loading = None;
                let first_load = guided.steps.is_empty() && guided.overview.is_empty();
                apply_loaded(guided, loaded, first_load);
                if first_load
                    && opens_chat_beside(guided, ai, root.is_some())
                    && state.pending_action.is_none()
                {
                    state.pending_action = Some(PendingAction::GuidedChat { focus: false });
                }
                if first_load {
                    notices.push((
                        format!(
                            "guided review: {} file(s); \u{2192} next, c note, a ask, e edit, f fix",
                            guided.total()
                        ),
                        false,
                    ));
                } else {
                    notices.push(("guided review refreshed".to_string(), false));
                }
            }
            Poll::Message(Err(message)) => {
                guided.loading = None;
                notices.push((format!("guided review: {message}"), true));
            }
            Poll::Disconnected => {
                guided.loading = None;
                notices.push((
                    format!(
                        "guided review: {}",
                        stopped_unexpectedly("reading the change")
                    ),
                    true,
                ));
            }
            Poll::Pending => {}
        }
    }

    let answered = drain_asks(guided);
    let final_key = guided.final_key();
    if answered.contains(&final_key) {
        notices.push((review_done(guided), false));
    }
    // Without a checkout there is no code to settle questions in.
    if let Some(root) = &root {
        guided.check_queue.extend(answered);
        while guided.checks.len() < CONCURRENT_CHECKS && !guided.check_queue.is_empty() {
            let key = guided.check_queue.remove(0);
            start_question_check(guided, root, &key);
        }
    }
    drain_checks(guided, &mut notices);
    if ai {
        start_wanted_commentary(guided);
    }
    if guided
        .chat
        .is_some_and(|id| state.sessions.get(id).is_none())
    {
        guided.chat = None;
        guided.chat_owed.clear();
        if guided.mode == Mode::Chat {
            guided.mode = Mode::Browse;
        }
        notices.push(("the review chat ended".to_string(), false));
    }
    let chat_done = chat_turn_ended(guided, &state.sessions);
    let owed = guided.loading.is_none() && std::mem::take(&mut guided.reload_owed);
    if chat_done {
        notices.push((
            "Claude is done \u{2014} reading the change again".to_string(),
            false,
        ));
    }
    tell_chat_what_is_read(guided, &mut state.sessions);

    for (text, is_error) in notices {
        state.set_status(text, is_error);
    }
    // What the chat changed shows in the walk: a file it edited is read
    // again, and the whole change after it, so the lists follow the fix.
    if chat_done || owed {
        reload(state);
    }
}

/// Whether the chat just finished a turn: it was working, or waiting on a
/// question, the last time and is not now.
pub(super) fn chat_turn_ended(guided: &mut Guided, sessions: &crate::session::Sessions) -> bool {
    use crate::session::SessionActivity::{NeedsInput, Working};
    let busy = guided
        .chat
        .and_then(|id| sessions.get(id))
        .is_some_and(|chat| matches!(chat.activity(), Working | NeedsInput));
    let ended = guided.chat_busy && !busy;
    guided.chat_busy = busy;
    ended
}

/// Claude opens beside a new review, ready to fix what the model finds:
/// when there is a model to find it and a checkout to fix it in.
pub(super) fn opens_chat_beside(guided: &Guided, ai: bool, checkout: bool) -> bool {
    ai && checkout && guided.chat.is_none() && guided.source.editable()
}

/// What the whole review came to, said when its final read is in.
fn review_done(guided: &Guided) -> String {
    let open = guided.open_findings();
    let count = |severity| {
        open.iter()
            .filter(|open| open.finding.severity == severity)
            .count()
    };
    format!(
        "review read: {} issue(s), {} nit(s), {} question(s) open \u{2014} M for the issues walk",
        count(findings::Severity::Issue),
        count(findings::Severity::Nit),
        count(findings::Severity::Question),
    )
}

/// The keys of the steps whose commentary came in whole.
fn drain_asks(guided: &mut Guided) -> Vec<String> {
    let mut finished = Vec::new();
    let mut answered = Vec::new();
    for (index, ask) in guided.asks.iter().enumerate() {
        let entry = guided.commentary.entry(ask.key.clone()).or_default();
        loop {
            match poll_once(&ask.rx) {
                Poll::Message(GenMsg::Thinking(_)) => entry.thinking = true,
                Poll::Message(GenMsg::Output(chunk)) => {
                    entry.thinking = false;
                    entry.text.push_str(&chunk);
                }
                Poll::Message(GenMsg::Reset) => entry.text.clear(),
                Poll::Message(GenMsg::Done { text, stats }) => {
                    entry.text = text;
                    if stats.truncated {
                        entry.text.push('\n');
                        entry.text.push_str(crate::llm::TRUNCATED_NOTE);
                    }
                    entry.done = true;
                    entry.thinking = false;
                    answered.push(ask.key.clone());
                    finished.push(index);
                    break;
                }
                Poll::Message(GenMsg::Error(message)) => {
                    entry.error = Some(message);
                    entry.done = true;
                    entry.thinking = false;
                    guided.failed.insert(ask.key.clone());
                    finished.push(index);
                    break;
                }
                Poll::Pending => break,
                // The worker died before the answer was done.
                Poll::Disconnected => {
                    entry.error = Some(stopped_unexpectedly("the model request"));
                    entry.done = true;
                    entry.thinking = false;
                    finished.push(index);
                    break;
                }
            }
        }
    }
    for index in finished.into_iter().rev() {
        guided.asks.remove(index);
    }
    answered
}

/// Send the questions step `key`'s commentary raised to Claude, to be settled
/// in the checkout, when the files there are the ones being reviewed.
fn start_question_check(guided: &mut Guided, root: &str, key: &str) {
    if !guided.source.editable() {
        return;
    }
    let Some(index) = guided.steps.iter().position(|step| step.key() == key) else {
        return;
    };
    let Some(commentary) = guided.commentary.get(key) else {
        return;
    };
    let (without, questions) = findings::split_questions(&commentary.text);
    if questions.is_empty() {
        return;
    }
    let prompt = crate::llm::build_guided_question_check_prompt(
        &step_context(guided, index + 1),
        &questions.join("\n"),
        &crate::settings::load(),
    );
    let answer = work::check_questions(root.to_string(), prompt);
    if let Some(commentary) = guided.commentary.get_mut(key) {
        commentary.text = without;
        commentary.done = false;
        commentary.checking = true;
    }
    guided.checks.push(Check {
        key: key.to_string(),
        questions,
        answer,
    });
}

/// Put what each finished check settled back into its step's commentary. A
/// check that failed puts the questions back as they were asked.
pub(super) fn drain_checks(guided: &mut Guided, notices: &mut Vec<(String, bool)>) {
    let mut finished = Vec::new();
    for (index, check) in guided.checks.iter().enumerate() {
        let settled = match poll_once(&check.answer) {
            Poll::Pending => continue,
            Poll::Message(Ok(settled)) => settled,
            Poll::Message(Err(message)) => {
                notices.push((format!("checking the questions: {message}"), true));
                check.questions.join("\n")
            }
            Poll::Disconnected => {
                notices.push((stopped_unexpectedly("checking the questions"), true));
                check.questions.join("\n")
            }
        };
        if let Some(commentary) = guided.commentary.get_mut(&check.key) {
            commentary.text = findings::with_settled(&commentary.text, &settled);
            commentary.done = true;
            commentary.checking = false;
        }
        finished.push(index);
    }
    for index in finished.into_iter().rev() {
        guided.checks.remove(index);
    }
}

/// How many requests may run at once. The local server answers one request
/// per session at a time; Claude takes several.
fn concurrency() -> usize {
    match crate::llm::current_provider() {
        crate::llm::LlmProvider::Claude => CONCURRENT_CLAUDE,
        crate::llm::LlmProvider::Mtplx => CONCURRENT_LOCAL,
    }
}

/// Most wanted first: the plan, which orders the walk, then the step on
/// screen, then every other file in the walk's order, and once they are all
/// read and settled the final review, which sees what they raised.
pub(super) fn steps_to_read(guided: &Guided) -> Vec<usize> {
    let owed = |i: usize| {
        let path = &guided.steps[i - 1].path;
        guided.chat_owed.iter().any(|(owed, _)| owed == path)
    };
    let on_screen = Some(guided.current).filter(|&i| i != guided.final_step());
    let last = guided.ready_for_final().then(|| guided.final_step());
    let mut seen = HashSet::new();
    std::iter::once(OVERVIEW)
        .chain(on_screen)
        .chain((1..=guided.total()).filter(|&i| owed(i)))
        .chain(guided.files_to_read())
        .chain(last)
        .filter(|&i| seen.insert(i))
        .collect()
}

pub(super) fn start_wanted_commentary(guided: &mut Guided) {
    if guided.loading.is_some() || (guided.steps.is_empty() && guided.overview.is_empty()) {
        return;
    }
    let limit = concurrency();
    let running = guided.asks.len();
    if running >= limit {
        return;
    }
    let wanted = steps_to_read(guided);
    let mut slots = limit - running;
    // A file waits for the plan, which tells it what concern it is part of.
    let planning = guided
        .commentary
        .get(OVERVIEW_KEY)
        .is_some_and(|plan| !plan.done);
    for index in wanted {
        if slots == 0 {
            break;
        }
        if planning && index != OVERVIEW && index != guided.final_step() {
            continue;
        }
        let key = if index == OVERVIEW {
            OVERVIEW_KEY.to_string()
        } else if index == guided.final_step() {
            guided.final_key()
        } else {
            guided.steps[index - 1].key()
        };
        let asked = guided.commentary.contains_key(&key) || guided.failed.contains(&key);
        if asked {
            continue;
        }
        let (rx, prompt_bytes) = if index == OVERVIEW {
            let contexts = plan_contexts(guided);
            let bytes = contexts.iter().map(String::len).sum();
            (work::ask_overview(contexts, concurrency()), bytes)
        } else if index == guided.final_step() {
            let contexts = final_contexts(guided);
            let bytes = contexts.iter().map(String::len).sum();
            (work::ask_final(contexts, concurrency()), bytes)
        } else {
            // The walk's first brief, so the cached prompt stays the same.
            let shared = guided
                .shared_brief
                .get_or_insert_with(|| guided.brief.clone())
                .clone();
            let context = file_context(guided, index);
            let bytes = shared.len() + context.len();
            (work::ask_step(shared, context), bytes)
        };
        guided.commentary.insert(
            key.clone(),
            Commentary {
                started: Some(std::time::Instant::now()),
                prompt_bytes,
                ..Commentary::default()
            },
        );
        guided.asks.push(Ask { key, rx });
        slots -= 1;
    }
}
