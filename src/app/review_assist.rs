use crate::state::{
    AppState, DiffSource, Pane, ReviewAssistJob, ReviewChatJob, ReviewFlagJob, ReviewFlagMsg,
    ReviewJob, ReviewMsg,
};

const REVIEW_PR_NODE_ID: &str = crate::git::REVIEW_PR_TEXT_NODE_ID;
const MAX_REVIEW_ASSIST_CONTEXT_BYTES: usize = 32_000;
const MAX_REVIEW_CHAT_CONTEXT_BYTES: usize = 32_000;
const MAX_REVIEW_FLAG_CONTEXT_BYTES: usize = 14_000;
const MAX_REVIEW_ASSIST_OVERVIEW_LINES: usize = 96;

pub(super) fn spawn_assisted_review(state: &mut AppState) {
    if state.review_job.is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let handle =
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
    state.review_job = Some(ReviewJob {
        rx,
        handle: Some(handle),
        spinner: 0,
    });
    state.focus = Pane::Main;
    state.diff_source = DiffSource::Review;
    state.diff_offset = 0;
    state.review = None;
    state.review_idx = 0;
    state.review_collapsed.clear();
    state.review_context_open.clear();
    state.review_context_restore_collapsed.clear();
    state.review_assists.clear();
    state.review_style_findings.clear();
    state.review_flag_active_path = None;
    state.review_chat_messages.clear();
    state.review_chat_input.clear();
    state.review_chat_cursor = 0;
    state.review_chat_scroll = 0;
    if let Some(mut job) = state.review_assist_job.take() {
        state.defer_thread_join(job.handle.take());
    }
    if let Some(mut job) = state.review_pr_job.take() {
        state.defer_thread_join(job.handle.take());
    }
    if let Some(mut job) = state.review_flag_job.take() {
        state.defer_thread_join(job.handle.take());
    }
    if let Some(mut job) = state.review_chat_job.take() {
        state.defer_thread_join(job.handle.take());
    }
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
    let handle = std::thread::spawn(move || {
        crate::llm::stream_review_assist(context, tx);
    });
    state.review_assist_job = Some(ReviewAssistJob {
        rx,
        handle: Some(handle),
        node_id,
        output: String::new(),
        spinner: 0,
    });
    state.set_status("explaining review item...", false);
}

pub(super) fn spawn_review_pr_text(state: &mut AppState) {
    let Some(context) = review_pr_context(state) else {
        return;
    };
    if let Some(mut job) = state.review_pr_job.take() {
        state.defer_thread_join(job.handle.take());
    }
    state.review_assists.remove(REVIEW_PR_NODE_ID);
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        crate::llm::stream_review_pr_text(context, tx);
    });
    state.review_pr_job = Some(ReviewAssistJob {
        rx,
        handle: Some(handle),
        node_id: REVIEW_PR_NODE_ID.to_string(),
        output: String::new(),
        spinner: 0,
    });
    state.set_status("writing PR text...", false);
}

pub(super) fn spawn_review_style_flags(state: &mut AppState) {
    let file_contexts = review_flag_contexts(state);
    if file_contexts.is_empty() {
        state.review_style_findings.clear();
        state.review_flag_active_path = None;
        return;
    }
    if let Some(mut job) = state.review_flag_job.take() {
        state.defer_thread_join(job.handle.take());
    }
    state.review_style_findings.clear();
    state.review_flag_active_path = None;
    let total = file_contexts.len();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        for (index, (path, context)) in file_contexts.into_iter().enumerate() {
            if tx
                .send(ReviewFlagMsg::Started {
                    path: path.clone(),
                    index: index + 1,
                    total,
                })
                .is_err()
            {
                return;
            }
            match review_style_flag_file(&path, context) {
                Ok(finding) => {
                    if tx.send(ReviewFlagMsg::Done { path, finding }).is_err() {
                        return;
                    }
                }
                Err(message) => {
                    if tx.send(ReviewFlagMsg::Error { path, message }).is_err() {
                        return;
                    }
                }
            }
        }
        let _ = tx.send(ReviewFlagMsg::Finished);
    });
    state.review_flag_job = Some(ReviewFlagJob {
        rx,
        handle: Some(handle),
        active_path: None,
        completed: 0,
        total,
        spinner: 0,
    });
    state.set_status("starting style flag pass...", false);
}

pub(super) fn spawn_review_chat(state: &mut AppState, prompt: String) {
    let Some(context) = review_chat_context(state) else {
        state.set_status("build review first", true);
        return;
    };
    if state.review_chat_job.is_some() {
        state.set_status("review chat already running", false);
        return;
    }
    let mut history = state.review_chat_messages.clone();
    if history.last().is_some_and(|message| {
        message.role == crate::state::ReviewChatRole::User && message.content == prompt
    }) {
        history.pop();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        crate::llm::stream_review_chat(context, history, prompt, tx);
    });
    state.review_chat_job = Some(ReviewChatJob {
        rx,
        handle: Some(handle),
        output: String::new(),
        spinner: 0,
    });
    state.set_status("asking review chat...", false);
}

fn review_flag_contexts(state: &AppState) -> Vec<(String, String)> {
    review_flag_candidates(state)
        .into_iter()
        .filter_map(|path| {
            review_flag_context_for_path(state, &path).map(|context| (path, context))
        })
        .collect()
}

fn review_flag_context_for_path(state: &AppState, path: &str) -> Option<String> {
    let review = state.review.as_ref()?;
    let mut out = String::new();
    push_limited_line(
        &mut out,
        &format!("File under review: {path}"),
        MAX_REVIEW_FLAG_CONTEXT_BYTES,
    );
    push_limited_line(&mut out, "", MAX_REVIEW_FLAG_CONTEXT_BYTES);
    push_limited_line(
        &mut out,
        "Relevant review nodes:",
        MAX_REVIEW_FLAG_CONTEXT_BYTES,
    );
    let mut matched = false;
    for node in &review.nodes {
        if review_node_path(&node.title) != Some(path) {
            continue;
        }
        matched = true;
        if out.len() >= MAX_REVIEW_FLAG_CONTEXT_BYTES {
            push_limited_line(
                &mut out,
                "... file review context truncated ...",
                MAX_REVIEW_FLAG_CONTEXT_BYTES,
            );
            break;
        }
        let indent = "  ".repeat(node.depth as usize);
        push_limited_line(
            &mut out,
            &format!("{indent}- {}", node.title),
            MAX_REVIEW_FLAG_CONTEXT_BYTES,
        );
        for body in &node.body {
            push_limited_line(
                &mut out,
                &format!("{indent}  {body}"),
                MAX_REVIEW_FLAG_CONTEXT_BYTES,
            );
        }
        for context in &node.context {
            push_limited_line(
                &mut out,
                &format!("{indent}  {context}"),
                MAX_REVIEW_FLAG_CONTEXT_BYTES,
            );
        }
    }
    matched.then_some(out)
}

fn review_flag_candidates(state: &AppState) -> Vec<String> {
    let Some(review) = &state.review else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for node in &review.nodes {
        let Some(path) = review_node_path(&node.title) else {
            continue;
        };
        if is_kotlin_path(path) && !paths.iter().any(|candidate| candidate == path) {
            paths.push(path.to_string());
        }
    }
    paths
}

fn review_chat_context(state: &AppState) -> Option<String> {
    let report = &state.review.as_ref()?.report;
    let mut out = String::new();
    push_limited_line(
        &mut out,
        "Full assisted review against main. Use this as the source of truth.",
        MAX_REVIEW_CHAT_CONTEXT_BYTES,
    );
    push_limited_line(&mut out, "", MAX_REVIEW_CHAT_CONTEXT_BYTES);
    for line in report.lines() {
        if out.len() >= MAX_REVIEW_CHAT_CONTEXT_BYTES {
            push_limited_line(
                &mut out,
                "... full review context truncated ...",
                MAX_REVIEW_CHAT_CONTEXT_BYTES,
            );
            break;
        }
        push_limited_line(&mut out, line, MAX_REVIEW_CHAT_CONTEXT_BYTES);
    }
    Some(out)
}

fn review_pr_context(state: &AppState) -> Option<String> {
    review_chat_context(state)
}

fn review_node_path(title: &str) -> Option<&str> {
    let location = title
        .split_once(" in ")
        .map(|(path, _)| path)
        .or_else(|| title.split_once(" - ").map(|(location, _)| location))
        .unwrap_or(title);
    let path = location
        .rsplit_once(':')
        .filter(|(_, line)| line.chars().all(|ch| ch.is_ascii_digit()))
        .map(|(path, _)| path)
        .unwrap_or(location)
        .trim();
    (!path.is_empty()).then_some(path)
}

fn is_kotlin_path(path: &str) -> bool {
    path.ends_with(".kt") || path.ends_with(".kts")
}

fn review_style_flag_file(
    path: &str,
    context: String,
) -> Result<crate::state::ReviewStyleFinding, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::llm::stream_review_style_flag(path.to_string(), context, tx);
    let mut final_msg = None;
    let mut thinking = String::new();
    let mut error = None;
    for msg in rx {
        match msg {
            crate::state::GenMsg::Done(output) => final_msg = Some(output),
            crate::state::GenMsg::Error(message) => error = Some(message),
            crate::state::GenMsg::Thinking(chunk) => thinking.push_str(&chunk),
            crate::state::GenMsg::Output(_) => {}
        }
    }
    if let Some(error) = error {
        return Err(error);
    }
    let response = final_msg
        .as_deref()
        .filter(|output| !output.trim().is_empty())
        .or_else(|| (!thinking.trim().is_empty()).then_some(thinking.as_str()))
        .ok_or_else(|| "empty LLM response".to_string())?;
    Ok(crate::llm::parse_review_style_finding(response))
}

fn review_assist_context(state: &AppState, node_id: &str) -> Option<String> {
    let review = state.review.as_ref()?;
    let selected = review.nodes.iter().find(|node| node.id == node_id)?;
    let mut out = String::new();
    push_review_overview(&mut out, &review.report);
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
        if !push_review_node_context(&mut out, node) {
            push_line(&mut out, "... review subtree truncated ...");
            break;
        }
    }
    push_related_test_context(&mut out, review, node_id);

    Some(out)
}

fn push_review_overview(out: &mut String, report: &str) {
    push_line(out, "Branch review overview:");
    let mut emitted = 0usize;
    for line in report.lines() {
        let trimmed = line.trim_end();
        if matches!(
            trimmed,
            "Entry point trace" | "Review checklist" | "Full diff against main"
        ) {
            break;
        }
        if trimmed.chars().all(|ch| ch == '=' || ch == '-') {
            continue;
        }
        push_line(out, trimmed);
        emitted += 1;
        if emitted >= MAX_REVIEW_ASSIST_OVERVIEW_LINES {
            push_line(out, "... review overview truncated ...");
            break;
        }
    }
    push_line(out, "");
}

fn push_review_node_context(out: &mut String, node: &crate::git::ReviewNode) -> bool {
    if out.len() >= MAX_REVIEW_ASSIST_CONTEXT_BYTES {
        return false;
    }
    let indent = "  ".repeat(node.depth as usize);
    push_line(out, &format!("{indent}- {}", node.title));
    for body in &node.body {
        push_line(out, &format!("{indent}  {body}"));
    }
    if !node.context.is_empty() {
        push_line(out, &format!("{indent}  source context:"));
        for context in &node.context {
            push_line(out, &format!("{indent}  {context}"));
        }
    }
    true
}

fn push_related_test_context(
    out: &mut String,
    review: &crate::git::AssistedReview,
    selected_id: &str,
) {
    let selected_indices = review
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(idx, _)| review_node_in_subtree(review, idx, selected_id).then_some(idx))
        .collect::<Vec<_>>();
    if selected_indices.is_empty() {
        return;
    }

    let selected_paths = selected_indices
        .iter()
        .filter_map(|idx| review.nodes.get(*idx))
        .filter_map(|node| review_node_path(&node.title))
        .filter(|path| is_source_path(path) && !is_test_path(path))
        .collect::<Vec<_>>();
    if selected_paths.is_empty() {
        return;
    }

    let related_test_roots = review
        .nodes
        .iter()
        .filter(|node| node.id.contains(":file:"))
        .filter_map(|node| review_node_path(&node.title).map(|path| (node.id.as_str(), path)))
        .filter(|(_, path)| is_test_path(path))
        .filter(|(_, path)| {
            selected_paths
                .iter()
                .any(|production_path| related_test_path(production_path, path))
        })
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    if related_test_roots.is_empty() {
        return;
    }

    push_line(out, "");
    push_line(out, "Related test changes outside selected subtree:");
    for (idx, node) in review.nodes.iter().enumerate() {
        if selected_indices.contains(&idx) {
            continue;
        }
        if !related_test_roots
            .iter()
            .any(|root_id| node.id == *root_id || review_node_in_subtree(review, idx, root_id))
        {
            continue;
        }
        if !push_review_node_context(out, node) {
            push_line(out, "... related test context truncated ...");
            break;
        }
    }
}

fn is_source_path(path: &str) -> bool {
    path.contains('/')
        && path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.'))
}

fn is_test_path(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.starts_with("src/test/")
        || path.contains("/src/test/")
        || file_stem(path).is_some_and(|stem| {
            stem.ends_with("Test") || stem.ends_with("Tests") || stem.ends_with("Spec")
        })
}

fn related_test_path(production_path: &str, test_path: &str) -> bool {
    let Some(production_stem) = file_stem(production_path) else {
        return false;
    };
    let Some(test_stem) = file_stem(test_path) else {
        return false;
    };
    let test_subject = test_stem
        .strip_suffix("Tests")
        .or_else(|| test_stem.strip_suffix("Test"))
        .or_else(|| test_stem.strip_suffix("Spec"))
        .unwrap_or(test_stem);

    !test_subject.is_empty()
        && (test_subject == production_stem
            || test_subject.contains(production_stem)
            || production_stem.contains(test_subject))
}

fn file_stem(path: &str) -> Option<&str> {
    std::path::Path::new(path).file_stem()?.to_str()
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
    push_limited_line(out, line, MAX_REVIEW_ASSIST_CONTEXT_BYTES);
}

fn push_limited_line(out: &mut String, line: &str, max_bytes: usize) {
    if out.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes - out.len();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{AssistedReview, ReviewNode};

    #[test]
    fn review_assist_context_includes_branch_overview_before_subtree() {
        let mut state = AppState::new();
        state.review = Some(AssistedReview {
            report: "\
Assisted review against main
============================

Branch: bugfix/pending-should-convert-items-individually-not-as-a-sum
Base: origin/main
Scope: 2 commits, 2 files

Commits in review range
-----------------------
- 1b7004a fix(balance): convert pending points to household currency - Aggregate pending transaction points into the household's base currency using exchange rates instead of reporting raw values per currency.

Files changed
-------------
- M src/main/kotlin/me/spenn/BalanceService.kt
- M src/test/kotlin/me/spenn/BalanceServiceTest.kt

Entry point trace
-----------------
- hidden from overview

Full diff against main
----------------------
diff --git a/src/main/kotlin/me/spenn/BalanceService.kt b/src/main/kotlin/me/spenn/BalanceService.kt
"
            .into(),
            nodes: vec![
                ReviewNode {
                    id: "branch".into(),
                    parent: None,
                    depth: 0,
                    title: "Full diff against main".into(),
                    body: Vec::new(),
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "branch:file:0".into(),
                    parent: Some("branch".into()),
                    depth: 1,
                    title: "src/main/kotlin/me/spenn/BalanceService.kt - 1 entry point (+1 -1)"
                        .into(),
                    body: vec!["+ converted pending value".into()],
                    context: Vec::new(),
                },
            ],
        });

        let context = review_assist_context(&state, "branch:file:0").unwrap();

        assert!(context.contains("Branch review overview:"), "{context}");
        assert!(
            context.contains(
                "Aggregate pending transaction points into the household's base currency"
            ),
            "{context}"
        );
        assert!(
            context.contains("- M src/test/kotlin/me/spenn/BalanceServiceTest.kt"),
            "{context}"
        );
        assert!(
            context.contains("selected: src/main/kotlin/me/spenn/BalanceService.kt"),
            "{context}"
        );
        assert!(!context.contains("- hidden from overview"), "{context}");
        assert!(!context.contains("diff --git a/"), "{context}");
    }
}
