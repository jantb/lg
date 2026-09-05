//! The review jobs, and which nodes of the review tree open with them.

use std::collections::HashSet;

use crate::state::{DiffSource, GenMsg, ReviewFlagMsg, ReviewMsg};

use super::super::App;
use super::{
    drain_messages, drain_review_stream, first_status_line, join_worker, take_finished,
    tick_spinner,
};

pub(super) fn default_review_collapsed_nodes(
    review: &crate::git::AssistedReview,
) -> HashSet<String> {
    review
        .nodes
        .iter()
        .filter(|node| should_start_collapsed(review, node))
        .map(|node| node.id.clone())
        .collect()
}

fn should_start_collapsed(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> bool {
    if node.id == "branch" || node.id.starts_with("branch:category:") {
        return false;
    }
    if node.id == "checklist" {
        return false;
    }
    if node.id.contains(":file:") {
        return true;
    }
    let has_child = review
        .nodes
        .iter()
        .any(|candidate| candidate.parent.as_deref() == Some(node.id.as_str()));
    (node.parent.is_none() && !node.body.is_empty()) || (node.id.contains(":entry:") && has_child)
}

pub(super) fn initial_review_index(review: &crate::git::AssistedReview) -> usize {
    review
        .nodes
        .iter()
        .position(|node| node.id.starts_with("branch:file:"))
        .or_else(|| review.nodes.iter().position(|node| node.id == "branch"))
        .unwrap_or(0)
}

fn review_path_ancestor_ids(review: &crate::git::AssistedReview, path: &str) -> Vec<String> {
    let Some(node_id) = review_node_id_for_path(review, path) else {
        return Vec::new();
    };
    let mut ancestors = Vec::new();
    let mut parent = review
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .and_then(|node| node.parent.as_deref());
    while let Some(parent_id) = parent {
        ancestors.push(parent_id.to_string());
        parent = review
            .nodes
            .iter()
            .find(|node| node.id == parent_id)
            .and_then(|node| node.parent.as_deref());
    }
    ancestors
}

fn review_node_id_for_path<'a>(
    review: &'a crate::git::AssistedReview,
    path: &str,
) -> Option<&'a str> {
    review
        .nodes
        .iter()
        .find(|node| node.id.contains(":file:") && review_title_path(&node.title) == Some(path))
        .or_else(|| {
            review
                .nodes
                .iter()
                .find(|node| review_title_path(&node.title) == Some(path))
        })
        .map(|node| node.id.as_str())
}

fn review_title_path(title: &str) -> Option<&str> {
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

impl App {
    pub(in crate::app) fn drain_review_job(&mut self) {
        let Some((mut job, msg)) = take_finished(&mut self.state.review_job) else {
            return;
        };
        join_worker(job.handle.take());
        {
            match msg {
                ReviewMsg::Done(review) => {
                    let report = review.report.clone();
                    self.state.review = Some(*review);
                    self.state.review_collapsed.clear();
                    self.state.review_context_open.clear();
                    self.state.review_context_restore_collapsed.clear();
                    if let Some(review) = &self.state.review {
                        self.state.review_collapsed = default_review_collapsed_nodes(review);
                        self.state.review_idx = initial_review_index(review);
                    }
                    self.state.diff_source = DiffSource::Review;
                    self.state.set_diff_text(report);
                    self.state.diff_offset = 0;
                    self.state.set_status("review ready", false);
                }
                ReviewMsg::Error(err) => {
                    self.state
                        .set_diff_text(format!("error building assisted review: {err}"));
                    self.state.set_status(first_status_line(&err), true);
                }
            }
        }
    }

    pub(in crate::app) fn drain_review_flag_job(&mut self) {
        let mut handle = None;
        for msg in drain_messages(&self.state.review_flag_job) {
            match msg {
                ReviewFlagMsg::Started { path, index, total } => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        job.active_path = Some(path.clone());
                    }
                    let reveal_ids = self
                        .state
                        .review
                        .as_ref()
                        .map(|review| review_path_ancestor_ids(review, &path))
                        .unwrap_or_default();
                    for id in reveal_ids {
                        self.state.review_collapsed.remove(&id);
                    }
                    self.state.review_flag_active_path = Some(path.clone());
                    self.state
                        .set_status(format!("analyzing style {index}/{total}: {path}"), false);
                }
                ReviewFlagMsg::Done { path, finding } => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        job.completed = job.completed.saturating_add(1);
                    }
                    if self.state.review_flag_active_path.as_deref() == Some(path.as_str()) {
                        self.state.review_flag_active_path = None;
                    }
                    let severity = finding.severity;
                    let is_error = !matches!(severity, crate::state::ReviewStyleSeverity::Ok);
                    self.state
                        .review_style_findings
                        .insert(path.clone(), finding);
                    self.state.set_status(
                        format!("style {}: {path}", severity.label().to_ascii_lowercase()),
                        is_error,
                    );
                }
                ReviewFlagMsg::Error { path, message } => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        job.completed = job.completed.saturating_add(1);
                    }
                    if self.state.review_flag_active_path.as_deref() == Some(path.as_str()) {
                        self.state.review_flag_active_path = None;
                    }
                    self.state
                        .set_status(format!("style check failed for {path}: {message}"), true);
                }
                ReviewFlagMsg::Finished => {
                    if let Some(job) = self.state.review_flag_job.as_mut() {
                        handle = job.handle.take();
                    }
                    self.state.review_flag_job = None;
                    self.state.review_flag_active_path = None;
                    let warn_count = self
                        .state
                        .review_style_findings
                        .values()
                        .filter(|finding| {
                            matches!(finding.severity, crate::state::ReviewStyleSeverity::Warn)
                        })
                        .count();
                    let fail_count = self
                        .state
                        .review_style_findings
                        .values()
                        .filter(|finding| {
                            matches!(finding.severity, crate::state::ReviewStyleSeverity::Fail)
                        })
                        .count();
                    self.state.set_status(
                        format!("style pass complete: {warn_count} warn, {fail_count} fail"),
                        fail_count > 0,
                    );
                }
            }
        }
        join_worker(handle);
        tick_spinner(&mut self.state.review_flag_job);
    }

    pub(in crate::app) fn drain_review_assist(&mut self) {
        let status = drain_review_stream(
            &mut self.state.review_assist_job,
            &mut self.state.review_assists,
            "review explanation ready",
        );
        if let Some((text, is_error)) = status {
            self.state.set_status(text, is_error);
        }
    }

    pub(in crate::app) fn drain_review_pr_text(&mut self) {
        let status = drain_review_stream(
            &mut self.state.review_pr_job,
            &mut self.state.review_assists,
            "PR text ready",
        );
        if let Some((text, is_error)) = status {
            self.state.set_status(text, is_error);
        }
    }

    pub(in crate::app) fn drain_review_chat(&mut self) {
        let mut handle = None;
        for msg in drain_messages(&self.state.review_chat_job) {
            match msg {
                GenMsg::Thinking(_) => {}
                GenMsg::Output(output) => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        job.output.push_str(&output);
                    }
                }
                GenMsg::Reset => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        job.output.clear();
                    }
                }
                GenMsg::Done {
                    text: final_msg,
                    stats,
                } => {
                    let truncated = stats.truncated;
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        handle = job.handle.take();
                    }
                    self.state.review_chat_job = None;
                    self.state
                        .review_chat_messages
                        .push(crate::state::ReviewChatMessage {
                            role: crate::state::ReviewChatRole::Assistant,
                            content: final_msg,
                            note: truncated.then(|| crate::llm::TRUNCATED_NOTE.to_string()),
                        });
                    self.state.review_chat_scroll = u16::MAX;
                    self.state.set_status(
                        if truncated {
                            "review chat answer cut off at the token budget"
                        } else {
                            "review chat ready"
                        },
                        truncated,
                    );
                }
                GenMsg::Error(error) => {
                    if let Some(job) = self.state.review_chat_job.as_mut() {
                        handle = job.handle.take();
                    }
                    self.state.review_chat_job = None;
                    self.state
                        .review_chat_messages
                        .push(crate::state::ReviewChatMessage {
                            role: crate::state::ReviewChatRole::Assistant,
                            content: String::new(),
                            note: Some(format!("llm error: {error}")),
                        });
                    self.state.review_chat_scroll = u16::MAX;
                    self.state.set_status(error, true);
                }
            }
        }
        join_worker(handle);
        if self.state.review_chat_job.is_some() {
            tick_spinner(&mut self.state.review_chat_job);
            self.state.review_chat_scroll = u16::MAX;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{AssistedReview, ReviewNode};

    #[test]
    fn review_defaults_show_file_rows_but_keep_file_children_collapsed() {
        let review = AssistedReview {
            report: String::new(),
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
                    id: "branch:category:production".into(),
                    parent: Some("branch".into()),
                    depth: 1,
                    title: "Production (1 file, 1 entry point, +1 -1)".into(),
                    body: Vec::new(),
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "branch:file:0".into(),
                    parent: Some("branch:category:production".into()),
                    depth: 2,
                    title: "src/lib.rs - 1 entry point (+1 -1)".into(),
                    body: vec!["@@ -1 +1 @@".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "branch:entry:0".into(),
                    parent: Some("branch:file:0".into()),
                    depth: 3,
                    title: "src/lib.rs:1 in fn greet - updates greet (+1 -1)".into(),
                    body: vec!["@@ -1 +1 @@".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "summary".into(),
                    parent: None,
                    depth: 0,
                    title: "Summary".into(),
                    body: vec!["details".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: "checklist".into(),
                    parent: None,
                    depth: 0,
                    title: "Review checklist".into(),
                    body: vec!["- Check this".into()],
                    context: Vec::new(),
                },
                ReviewNode {
                    id: crate::git::REVIEW_PR_TEXT_NODE_ID.into(),
                    parent: Some("checklist".into()),
                    depth: 1,
                    title: "PR text - generated by LLM (y copy)".into(),
                    body: Vec::new(),
                    context: Vec::new(),
                },
            ],
        };

        let collapsed = default_review_collapsed_nodes(&review);

        assert!(!collapsed.contains("branch"));
        assert!(!collapsed.contains("branch:category:production"));
        assert!(collapsed.contains("branch:file:0"));
        assert!(collapsed.contains("summary"));
        assert!(!collapsed.contains("checklist"));
        assert!(!collapsed.contains(crate::git::REVIEW_PR_TEXT_NODE_ID));
        assert_eq!(initial_review_index(&review), 2);

        let reveal_ids = review_path_ancestor_ids(&review, "src/lib.rs");
        assert_eq!(
            reveal_ids,
            vec![
                "branch:category:production".to_string(),
                "branch".to_string()
            ]
        );
    }
}
