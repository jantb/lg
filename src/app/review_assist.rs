use crate::state::{AppState, DiffSource, Pane, ReviewAssistJob, ReviewJob, ReviewMsg};

const MAX_REVIEW_ASSIST_CONTEXT_BYTES: usize = 18_000;

pub(super) fn spawn_assisted_review(state: &mut AppState) {
    if state.review_job.is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(
        move || match crate::git::build_assisted_review_against_main() {
            Ok(review) => {
                let _ = tx.send(ReviewMsg::Done(Box::new(review)));
            }
            Err(e) => {
                let _ = tx.send(ReviewMsg::Error(e.to_string()));
            }
        },
    );
    state.review_job = Some(ReviewJob { rx, spinner: 0 });
    state.focus = Pane::Main;
    state.diff_source = DiffSource::Review;
    state.diff_offset = 0;
    state.review = None;
    state.review_idx = 0;
    state.review_collapsed.clear();
    state.review_context_open.clear();
    state.review_assists.clear();
    state.review_assist_job = None;
    state.diff_text = "building assisted review against main...".to_string();
    state.diff_line_count = 1;
    state.set_status("building review...", false);
}

pub(super) fn spawn_review_assist(state: &mut AppState, node_id: String) {
    let Some(context) = review_assist_context(state, &node_id) else {
        state.set_status("no review item selected", true);
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        crate::ollama::stream_review_assist(context, tx);
    });
    state.review_assist_job = Some(ReviewAssistJob {
        rx,
        node_id,
        output: String::new(),
        spinner: 0,
    });
    state.set_status("explaining review item...", false);
}

fn review_assist_context(state: &AppState, node_id: &str) -> Option<String> {
    let review = state.review.as_ref()?;
    let selected = review.nodes.iter().find(|node| node.id == node_id)?;
    let mut out = String::new();
    push_line(
        &mut out,
        "Full diff against main, selected drilldown subtree:",
    );
    push_line(&mut out, &format!("selected: {}", selected.title));
    push_line(&mut out, "");

    for (idx, node) in review.nodes.iter().enumerate() {
        if !review_node_in_subtree(review, idx, node_id) {
            continue;
        }
        if out.len() >= MAX_REVIEW_ASSIST_CONTEXT_BYTES {
            push_line(&mut out, "... review subtree truncated ...");
            break;
        }
        let indent = "  ".repeat(node.depth as usize);
        push_line(&mut out, &format!("{indent}- {}", node.title));
        for body in &node.body {
            push_line(&mut out, &format!("{indent}  {body}"));
        }
        if !node.context.is_empty() {
            push_line(&mut out, &format!("{indent}  source context:"));
            for context in &node.context {
                push_line(&mut out, &format!("{indent}  {context}"));
            }
        }
    }

    Some(out)
}

fn review_node_in_subtree(
    review: &crate::git::AssistedReview,
    mut idx: usize,
    root_id: &str,
) -> bool {
    loop {
        let Some(node) = review.nodes.get(idx) else {
            return false;
        };
        if node.id == root_id {
            return true;
        }
        let Some(parent) = &node.parent else {
            return false;
        };
        let Some(parent_idx) = review
            .nodes
            .iter()
            .position(|candidate| &candidate.id == parent)
        else {
            return false;
        };
        idx = parent_idx;
    }
}

fn push_line(out: &mut String, line: &str) {
    if out.len() >= MAX_REVIEW_ASSIST_CONTEXT_BYTES {
        return;
    }
    let remaining = MAX_REVIEW_ASSIST_CONTEXT_BYTES - out.len();
    if line.len() < remaining {
        out.push_str(line);
        out.push('\n');
        return;
    }
    let take = remaining.saturating_sub(1);
    let mut added = 0usize;
    for ch in line.chars() {
        if ch.len_utf8() > take.saturating_sub(added) {
            break;
        }
        out.push(ch);
        added += ch.len_utf8();
    }
    out.push('\n');
}
