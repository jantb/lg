//! Marking files reviewed, and starting the review over.

use super::*;

/// Mark every file the plan says is safe to skim as reviewed.
pub(super) fn mark_skims_reviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(plan) = guided.plan() else {
        state.set_status("the plan is not read yet", false);
        return;
    };
    let keys: Vec<String> = guided
        .steps
        .iter()
        .filter(|step| plan.skim.contains_key(&step.path))
        .map(GuidedFile::key)
        .filter(|key| !guided.progress.reviewed.contains(key))
        .collect();
    if keys.is_empty() {
        state.set_status("no files left to skim", false);
        return;
    }
    let count = keys.len();
    guided.progress.reviewed.extend(keys);
    match guided.save() {
        Ok(()) => state.set_status(format!("{count} file(s) to skim marked reviewed"), false),
        Err(message) => state.set_status(message, true),
    }
}

/// The first press only asks: it forgets every note, which the walk cannot give back.
pub(super) fn reset_review(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    if !std::mem::take(&mut guided.reset_armed) {
        guided.reset_armed = true;
        let notes = guided.progress.notes.len();
        state.set_status(
            format!(
                "reset again to start over: forgets {notes} note(s), every reviewed mark and tick, and the model's reads"
            ),
            false,
        );
        return;
    }
    guided.progress = GuidedProgress {
        walk: guided.progress.walk,
        ..GuidedProgress::default()
    };
    guided.commentary.clear();
    guided.exchanges.clear();
    guided.failed.clear();
    guided.earlier.clear();
    guided.asks.clear();
    guided.checks.clear();
    guided.check_queue.clear();
    guided.read_wanted.clear();
    guided.chat_owed.clear();
    guided.shared_brief = None;
    guided.delete_armed = None;
    guided.mode = Mode::Browse;
    guided.current = OVERVIEW;
    guided.cursor = 0;
    guided.side_scroll = 0;
    guided.final_scroll.set(0);
    guided.final_pick = 0;
    match guided.save() {
        Ok(()) => state.set_status(
            "review reset \u{2014} starting over from the overview",
            false,
        ),
        Err(message) => state.set_status(message, true),
    }
}

pub(super) fn toggle_reviewed(state: &mut AppState) {
    let Some(guided) = state.guided.as_mut() else {
        return;
    };
    let Some(key) = guided.step().map(GuidedFile::key) else {
        return;
    };
    if let Some(at) = guided.progress.reviewed.iter().position(|k| *k == key) {
        guided.progress.reviewed.remove(at);
    } else {
        guided.progress.reviewed.push(key);
    }
    if let Err(message) = guided.save() {
        state.set_status(message, true);
    }
}
