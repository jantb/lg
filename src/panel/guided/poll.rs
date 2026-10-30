//! Each tick: the change being read, the model's answers streaming in, the
//! questions out with Claude, and the reads to ask for next.

use super::*;

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

    let mut reload_after_fix = false;
    if let Some((rx, _)) = guided.fixing.as_ref() {
        match poll_once(rx) {
            Poll::Message(result) => {
                guided.fixing = None;
                match result {
                    Ok(said) => {
                        notices.push((format!("claude: {said}"), false));
                        reload_after_fix = true;
                    }
                    Err(message) => notices.push((message, true)),
                }
            }
            Poll::Disconnected => {
                guided.fixing = None;
                notices.push((stopped_unexpectedly("the claude fix"), true));
            }
            Poll::Pending => {}
        }
    }

    let answered = drain_asks(guided);
    // Without a checkout there is no code to settle questions in.
    if let Some(root) = &root {
        guided.check_queue.extend(answered);
        while guided.checks.len() < CONCURRENT_CHECKS && !guided.check_queue.is_empty() {
            let key = guided.check_queue.remove(0);
            start_question_check(guided, root, &key, &mut state.sessions);
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
    note_file_change_for_chat(guided);
    tell_chat_what_is_read(guided, &mut state.sessions);

    for (text, is_error) in notices {
        state.set_status(text, is_error);
    }
    if reload_after_fix {
        reload(state);
    }
}

/// The keys of the steps whose commentary came in whole.
fn drain_asks(guided: &mut Guided) -> Vec<String> {
    let mut finished = Vec::new();
    let mut answered = Vec::new();
    for (index, ask) in guided.asks.iter().enumerate() {
        let entry = match &ask.target {
            Target::Commentary(key) => guided.commentary.entry(key.clone()).or_default(),
            Target::Answer(key, at) => match guided
                .exchanges
                .get_mut(key)
                .and_then(|exchanges| exchanges.get_mut(*at))
            {
                Some(exchange) => &mut exchange.answer,
                None => {
                    finished.push(index);
                    continue;
                }
            },
        };
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
                    if let Target::Commentary(key) = &ask.target {
                        answered.push(key.clone());
                    }
                    finished.push(index);
                    break;
                }
                Poll::Message(GenMsg::Error(message)) => {
                    entry.error = Some(message);
                    entry.done = true;
                    entry.thinking = false;
                    if let Target::Commentary(key) = &ask.target {
                        guided.failed.insert(key.clone());
                    }
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
fn start_question_check(
    guided: &mut Guided,
    root: &str,
    key: &str,
    sessions: &mut crate::session::Sessions,
) {
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
    let step = &guided.steps[index];
    let chat = guided
        .chat
        .and_then(|id| sessions.get_mut(id))
        .and_then(|chat| {
            let file = crate::hooks::fresh_session_file(
                std::path::Path::new(root),
                &answer_file_name(key),
            )
            .ok()?;
            Some((chat, file))
        });
    let answer = match chat {
        Some((chat, file)) => {
            let message =
                crate::llm::build_guided_chat_questions(&step.path, &questions.join("\n"), &file);
            let bracketed = chat.bracketed_paste();
            chat.send(&crate::term::encode_paste(&message, bracketed));
            chat.send(b"\r");
            CheckAnswer::Chat(file)
        }
        None => {
            let prompt = crate::llm::build_guided_question_check_prompt(
                &step_context(guided, index + 1),
                &questions.join("\n"),
                &crate::settings::load(),
            );
            CheckAnswer::Headless(work::check_questions(root.to_string(), prompt))
        }
    };
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

/// One file per step: the key's own hash covers only the changed lines, which
/// two files can share.
fn answer_file_name(key: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!("guided-questions-{:016x}.md", hasher.finish())
}

/// Put what each finished check settled back into its step's commentary. A
/// check that failed puts the questions back as they were asked.
pub(super) fn drain_checks(guided: &mut Guided, notices: &mut Vec<(String, bool)>) {
    let mut finished = Vec::new();
    for (index, check) in guided.checks.iter().enumerate() {
        let polled = match &check.answer {
            CheckAnswer::Headless(rx) => poll_once(rx),
            CheckAnswer::Chat(file) => match std::fs::read_to_string(file) {
                Ok(settled) => {
                    let _ = std::fs::remove_file(file);
                    Poll::Message(Ok(settled))
                }
                Err(_) if guided.chat.is_none() => Poll::Message(Err(
                    "the review chat ended before it settled the questions".to_string(),
                )),
                Err(_) => Poll::Pending,
            },
        };
        let settled = match polled {
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
/// screen. A file is read only once the walk reaches it, the reviewer asks for
/// its concern to be read, or the chat is owed its redo.
pub(super) fn steps_to_read(guided: &Guided) -> Vec<usize> {
    let owed = |i: usize| {
        let path = &guided.steps[i - 1].path;
        guided.chat_owed.iter().any(|(owed, _)| owed == path)
    };
    let wanted = |i: usize| guided.read_wanted.contains(&guided.steps[i - 1].path);
    let mut seen = HashSet::new();
    std::iter::once(OVERVIEW)
        .chain(std::iter::once(guided.current))
        .chain(guided.walk_order().into_iter().filter(|&i| wanted(i)))
        .chain((1..=guided.total()).filter(|&i| owed(i)))
        .filter(|&i| seen.insert(i))
        .collect()
}

pub(super) fn start_wanted_commentary(guided: &mut Guided) {
    if guided.loading.is_some() || (guided.steps.is_empty() && guided.overview.is_empty()) {
        return;
    }
    let limit = concurrency();
    let running = guided
        .asks
        .iter()
        .filter(|ask| matches!(ask.target, Target::Commentary(_)))
        .count();
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
            let context = format!(
                "{}\nThe whole diff, file by file:\n{}",
                guided.overview,
                whole_diff(guided)
            );
            let bytes = context.len();
            (work::ask_overview(context), bytes)
        } else if index == guided.final_step() {
            let context = final_context(guided);
            let bytes = context.len();
            (work::ask_final(context), bytes)
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
        guided.asks.push(Ask {
            target: Target::Commentary(key),
            rx,
        });
        slots -= 1;
    }
}
