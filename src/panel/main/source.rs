//! The source a review node points at, and the sections shown around it.

use ratatui::text::Line;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    state::{AppState, DiffViewMode},
    ui,
};

mod line;
mod notes;
mod overlay;

pub(super) use line::owned_spans;
pub(super) use overlay::inline_diff_overlay;

use line::*;
use notes::*;
use overlay::*;

pub(super) struct SourceSection {
    pub path: String,
    pub body: Vec<String>,
    pub context: Vec<String>,
    pub notes: BTreeMap<usize, Vec<String>>,
}

/// Per-frame cache so file reads aren't repeated across the
/// scroll-bound and rendering passes of the review panel.
#[derive(Default)]
pub(super) struct RenderCache {
    files: HashMap<String, Option<String>>,
}

impl RenderCache {
    fn read(&mut self, path: &str) -> Option<&str> {
        if !self.files.contains_key(path) {
            self.files
                .insert(path.to_string(), std::fs::read_to_string(path).ok());
        }
        self.files.get(path).and_then(|opt| opt.as_deref())
    }
}

pub(super) fn review_source_context_lines(
    cache: &mut RenderCache,
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    syntax_path: Option<&str>,
    indent: &str,
) -> Vec<Line<'static>> {
    let mut lines = vec![section_header(indent, "source context")];
    let sections = source_sections(state, review, node);
    let side_by_side = side_by_side_diff_enabled(state);
    let viewport_width = state.diff_viewport_width;
    if !sections.is_empty() {
        let multiple = sections.len() > 1;
        for section in sections {
            // A suppressed file has no patch to overlay, and reading it to
            // display it would undo the suppression.
            if crate::git::is_suppressed_diff_body(&section.body) {
                lines.extend(fallback_source_context_lines(
                    &section.body,
                    &section.context,
                    Some(&section.path),
                    &section.notes,
                    indent,
                    side_by_side,
                    viewport_width,
                ));
                continue;
            }
            if let Some(mut source) = full_source_with_inline_diff(
                cache,
                &section.path,
                &section.body,
                &section.notes,
                indent,
                SourceRenderOptions {
                    show_path: multiple,
                    side_by_side,
                    viewport_width,
                },
            ) {
                lines.append(&mut source);
            } else {
                lines.extend(fallback_source_context_lines(
                    &section.body,
                    &section.context,
                    Some(&section.path),
                    &section.notes,
                    indent,
                    side_by_side,
                    viewport_width,
                ));
            }
        }
        return lines;
    }

    lines.extend(fallback_source_context_lines(
        &node.body,
        &node.context,
        syntax_path,
        &BTreeMap::new(),
        indent,
        side_by_side,
        viewport_width,
    ));
    lines
}

fn side_by_side_diff_enabled(state: &AppState) -> bool {
    state.diff_view_mode == DiffViewMode::SideBySide
}

pub(super) fn source_sections(
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> Vec<SourceSection> {
    if is_full_diff_root(node)
        && let Some(sections) = full_diff_source_sections(review)
        && !sections.is_empty()
    {
        let mut sections = sections;
        attach_inline_review_notes(state, review, node, &mut sections);
        return sections;
    }

    let mut sections = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut notes_by_path = inline_review_notes_by_path(state, review, node);
    for candidate in std::iter::once(node).chain(
        review
            .nodes
            .iter()
            .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
    ) {
        let Some(path) = super::review::review_node_path(&candidate.title) else {
            continue;
        };
        if candidate.body.is_empty() && candidate.context.is_empty() {
            continue;
        }
        if !seen_paths.insert(path.to_string()) {
            continue;
        }
        sections.push(SourceSection {
            path: path.to_string(),
            body: candidate.body.clone(),
            context: candidate.context.clone(),
            notes: notes_by_path.remove(path).unwrap_or_default(),
        });
    }
    sections
}

fn is_full_diff_root(node: &crate::git::ReviewNode) -> bool {
    node.parent.is_none() && node.title == "Full diff against main"
}

fn full_diff_source_sections(review: &crate::git::AssistedReview) -> Option<Vec<SourceSection>> {
    let (_, diff) = review.report.split_once("\nFull diff against main\n")?;
    let mut sections = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_body = Vec::new();

    for line in diff.lines() {
        if let Some(path) = diff_git_path(line) {
            push_full_diff_section(&mut sections, current_path.take(), &mut current_body);
            current_path = Some(path);
        }
        if current_path.is_some() {
            current_body.push(line.to_string());
        }
    }
    push_full_diff_section(&mut sections, current_path, &mut current_body);

    Some(sections)
}

fn diff_git_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git a/")?;
    let (_, path) = rest.split_once(" b/")?;
    (path != "/dev/null" && !path.trim().is_empty()).then(|| path.trim().to_string())
}

fn push_full_diff_section(
    sections: &mut Vec<SourceSection>,
    path: Option<String>,
    body: &mut Vec<String>,
) {
    let Some(path) = path else {
        body.clear();
        return;
    };
    if body.is_empty() {
        return;
    }
    sections.push(SourceSection {
        path,
        body: std::mem::take(body),
        context: Vec::new(),
        notes: BTreeMap::new(),
    });
}

fn review_node_in_subtree(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    root_id: &str,
) -> bool {
    let mut parent = node.parent.as_deref();
    while let Some(parent_id) = parent {
        if parent_id == root_id {
            return true;
        }
        parent = review
            .nodes
            .iter()
            .find(|candidate| candidate.id == parent_id)
            .and_then(|candidate| candidate.parent.as_deref());
    }
    false
}

fn source_sections_contain_path(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
) -> bool {
    std::iter::once(node)
        .chain(
            review
                .nodes
                .iter()
                .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
        )
        .any(|candidate| super::review::review_node_path(&candidate.title) == Some(path))
        || (is_full_diff_root(node)
            && full_diff_source_sections(review)
                .is_some_and(|sections| sections.iter().any(|section| section.path == path)))
}

fn first_changed_line_for_path(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
) -> Option<usize> {
    std::iter::once(node)
        .chain(
            review
                .nodes
                .iter()
                .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
        )
        .filter(|candidate| super::review::review_node_path(&candidate.title) == Some(path))
        .find_map(|candidate| {
            review_node_line(&candidate.title).or_else(|| first_body_hunk_new_line(&candidate.body))
        })
        .or_else(|| {
            full_diff_source_sections(review)?
                .into_iter()
                .find(|section| section.path == path)
                .and_then(|section| first_body_hunk_new_line(&section.body))
        })
        .map(|line| line.max(1))
}

fn fallback_source_context_lines(
    body: &[String],
    context: &[String],
    syntax_path: Option<&str>,
    notes: &BTreeMap<usize, Vec<String>>,
    indent: &str,
    side_by_side: bool,
    viewport_width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !body.is_empty() {
        lines.push(section_header(indent, "diff"));
        if side_by_side {
            lines.extend(prefixed_side_by_side_diff_lines(
                body,
                syntax_path,
                indent,
                viewport_width,
            ));
        } else {
            for body in body {
                let mut spans = context_prefix(indent);
                let body_line = syntax_path
                    .map(|path| ui::highlight_diff_line_for_path(body, path))
                    .unwrap_or_else(|| ui::highlight_diff_line(body));
                spans.extend(owned_spans(body_line));
                lines.push(Line::from(spans));
            }
        }
    }
    if !context.is_empty() {
        lines.push(section_header(indent, "source"));
        for context in context {
            lines.push(source_context_line(context, syntax_path, indent));
        }
    }
    if !notes.is_empty() {
        lines.push(section_header(indent, "review notes"));
        for line_notes in notes.values() {
            for note in line_notes {
                lines.push(source_note_line(indent, note));
            }
        }
    }
    lines
}
