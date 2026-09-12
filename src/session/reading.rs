//! Reading a session's screen for a question, and tracing what was read.

use super::*;

/// Openings of the questions an agent blocks on: claude's tool permissions,
/// file edits and the trust prompt a new directory gets, and the approvals
/// codex asks for before running a command.
pub(super) const QUESTION_MARKERS: &[&str] = &[
    "do you want",
    "would you like",
    "do you trust",
    "allow command",
    "allow this",
    "approve command",
    "approve this",
];

/// What a chosen option is ticked with once the question has been answered.
pub(super) const ANSWERED_MARKS: &[char] = &['\u{2714}', '\u{2713}'];

/// Whether the screen is showing a question claude is waiting on an answer to.
///
/// A choice list with one row selected is claude's own shape for asking, and
/// nothing it writes in an answer looks like that. The wording on its own is not
/// enough — an answer can easily contain "do you want" — so a question phrase
/// only counts when there are numbered options under it to pick from.
pub(super) fn is_asking(text: &str) -> bool {
    if text
        .lines()
        .any(|line| pending_choice(line).is_some_and(|selected| selected))
    {
        return true;
    }
    let lowered = text.to_lowercase();
    QUESTION_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
        && text.lines().any(|line| pending_choice(line).is_some())
}

/// Whether `line` is a row of a choice list still waiting to be answered, and
/// whether it is the selected one.
///
/// The caret alone does not make a choice list — it is also claude's ordinary
/// input prompt — so the number after it is what counts. An answered question
/// stays on screen with its chosen row ticked, which is how a prompt that has
/// already been dealt with is told from one that has not.
pub(super) fn pending_choice(line: &str) -> Option<bool> {
    let line = line.trim();
    if line.contains(ANSWERED_MARKS) {
        return None;
    }
    let (selected, rest) = match line.strip_prefix('\u{276f}') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, line),
    };
    let digits = leading_digits(rest);
    (digits > 0 && rest[digits..].starts_with('.')).then_some(selected)
}

/// Bytes of ASCII digits at the start of `text`.
pub(super) fn leading_digits(text: &str) -> usize {
    text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len()
}

/// Records a hook reporting in, and what the session reads as now.
pub(super) fn trace_event(label: &str, event: crate::hooks::HookEvent, activity: SessionActivity) {
    trace(&format!("{label} hook {} -> {activity:?}", event.name));
}

/// Records a question appearing or going away, with the screen lines it was read
/// off, so a shape this misses can be recovered afterwards instead of having to
/// be caught live.
pub(super) fn trace_reading(label: &str, activity: SessionActivity, screen: &str) {
    let mut tail: Vec<&str> = screen
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(TRACE_TAIL_LINES)
        .collect();
    tail.reverse();
    trace(&format!(
        "{label} screen -> {activity:?} | {}",
        tail.join(" \u{23ce} ")
    ));
}

/// Appends a line to the file named by `LG_SESSION_TRACE`, when there is one.
pub(super) fn trace(line: &str) {
    let Some(path) = std::env::var_os("LG_SESSION_TRACE") else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}
