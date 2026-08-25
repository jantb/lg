//! Matching review findings to the source lines they are about.

use std::collections::BTreeMap;

use crate::state::AppState;

use super::*;

pub(super) fn attach_inline_review_notes(
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    sections: &mut [SourceSection],
) {
    let mut notes_by_path = inline_review_notes_by_path(state, review, node);
    for section in sections {
        section.notes = notes_by_path.remove(&section.path).unwrap_or_default();
    }
}

pub(super) fn inline_review_notes_by_path(
    state: &AppState,
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
) -> BTreeMap<String, BTreeMap<usize, Vec<String>>> {
    let mut notes: BTreeMap<String, BTreeMap<usize, Vec<String>>> = BTreeMap::new();
    for candidate in std::iter::once(node).chain(
        review
            .nodes
            .iter()
            .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
    ) {
        let Some(path) = crate::panel::main::review::review_node_path(&candidate.title) else {
            continue;
        };
        let Some(line) = review_node_line(&candidate.title)
            .or_else(|| first_body_hunk_new_line(&candidate.body))
            .map(|line| line.max(1))
        else {
            continue;
        };
        if let Some(note) = inline_review_note_text(candidate) {
            push_inline_note(&mut notes, path, line, note);
        }
        if let Some(assist) = inline_assist_note_text(state, &candidate.id) {
            push_inline_note(&mut notes, path, line, assist);
        }
    }

    for (path, finding) in &state.review_style_findings {
        if !source_sections_contain_path(review, node, path) {
            continue;
        }
        if matches!(finding.severity, crate::state::ReviewStyleSeverity::Ok) {
            continue;
        }
        let line = style_finding_line(review, node, path, finding).unwrap_or(1);
        push_inline_note(
            &mut notes,
            path,
            line,
            format!(
                "style {}: {}",
                finding.severity.label().to_ascii_lowercase(),
                finding.reason.trim()
            ),
        );
    }
    notes
}

fn style_finding_line(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
    finding: &crate::state::ReviewStyleFinding,
) -> Option<usize> {
    finding
        .line
        .or_else(|| line_matching_reason(review, node, path, &finding.reason))
        .or_else(|| first_changed_line_for_path(review, node, path))
        .map(|line| line.max(1))
}

fn push_inline_note(
    notes: &mut BTreeMap<String, BTreeMap<usize, Vec<String>>>,
    path: &str,
    line: usize,
    note: String,
) {
    let line_notes = notes
        .entry(path.to_string())
        .or_default()
        .entry(line.max(1))
        .or_default();
    if !line_notes.contains(&note) {
        line_notes.push(note);
    }
}

fn line_matching_reason(
    review: &crate::git::AssistedReview,
    node: &crate::git::ReviewNode,
    path: &str,
    reason: &str,
) -> Option<usize> {
    let fragments = code_fragments(reason);
    if fragments.is_empty() {
        return None;
    }
    std::iter::once(node)
        .chain(
            review
                .nodes
                .iter()
                .filter(|candidate| review_node_in_subtree(review, candidate, &node.id)),
        )
        .filter(|candidate| {
            crate::panel::main::review::review_node_path(&candidate.title) == Some(path)
        })
        .find_map(|candidate| {
            line_matching_reason_in_body(&candidate.body, &fragments)
                .or_else(|| line_matching_reason_in_context(&candidate.context, &fragments))
        })
        .or_else(|| {
            full_diff_source_sections(review)?
                .into_iter()
                .find(|section| section.path == path)
                .and_then(|section| line_matching_reason_in_body(&section.body, &fragments))
        })
        .or_else(|| line_matching_reason_in_file(path, &fragments))
}

fn line_matching_reason_in_body(body: &[String], fragments: &[String]) -> Option<usize> {
    let mut new_line = 0usize;
    let mut in_hunk = false;
    for line in body {
        if let Some((_, new_start)) = parse_hunk_header(line) {
            new_line = new_start;
            in_hunk = true;
            continue;
        }
        if !in_hunk || line.starts_with("\\ No newline") {
            continue;
        }
        if line.starts_with('-') {
            continue;
        }
        let Some(source) = line.strip_prefix('+').or_else(|| line.strip_prefix(' ')) else {
            continue;
        };
        if source_matches_fragments(source, fragments) {
            return Some(new_line.max(1));
        }
        new_line = new_line.saturating_add(1);
    }
    None
}

fn line_matching_reason_in_context(context: &[String], fragments: &[String]) -> Option<usize> {
    context.iter().find_map(|line| {
        let (line_no, source) = line.split_once(" | ")?;
        source_matches_fragments(source, fragments)
            .then(|| line_no.trim().parse().ok())
            .flatten()
    })
}

fn line_matching_reason_in_file(path: &str, fragments: &[String]) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .enumerate()
        .find_map(|(idx, line)| source_matches_fragments(line, fragments).then_some(idx + 1))
}

fn code_fragments(reason: &str) -> Vec<String> {
    reason
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ':' | '`' | '"' | '\''))
        .map(|part| {
            part.trim_matches(|ch: char| {
                matches!(ch, '.' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>')
            })
        })
        .filter(|part| {
            part.len() >= 8
                && part
                    .chars()
                    .any(|ch| matches!(ch, '.' | '(' | ')' | '_') || ch.is_ascii_uppercase())
        })
        .map(str::to_string)
        .collect()
}

fn source_matches_fragments(source: &str, fragments: &[String]) -> bool {
    let compact_source = compact_code(source);
    fragments
        .iter()
        .map(|fragment| compact_code(fragment))
        .any(|fragment| !fragment.is_empty() && compact_source.contains(&fragment))
}

fn compact_code(s: &str) -> String {
    s.chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>()
}

fn inline_assist_note_text(state: &AppState, node_id: &str) -> Option<String> {
    let text = if let Some(job) = &state.review_assist_job
        && job.node_id == node_id
    {
        if job.output.trim().is_empty() {
            "thinking..."
        } else {
            job.output.trim()
        }
    } else {
        state.review_assists.get(node_id)?.trim()
    };
    first_note_line(text).map(|line| format!("llm: {line}"))
}

fn first_note_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| {
            line.trim()
                .trim_start_matches('#')
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim()
        })
        .find(|line| !line.is_empty())
        .map(|line| truncate_note(line, 120))
}

fn truncate_note(line: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in line.chars().take(max_chars) {
        out.push(ch);
    }
    if line.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn inline_review_note_text(node: &crate::git::ReviewNode) -> Option<String> {
    if let Some(effect) = node
        .body
        .iter()
        .find_map(|line| line.trim().strip_prefix("effect: "))
    {
        let effect = effect.trim();
        if !effect.is_empty() {
            return Some(format!("entry point: {effect}"));
        }
    }

    if (node.id.contains(":entry:") || node.id.contains(":hunk:"))
        && let Some((_, description)) = node.title.split_once(" - ")
    {
        let description = description.trim();
        if !description.is_empty() {
            return Some(format!("entry point: {description}"));
        }
    }

    None
}

pub(super) fn review_node_line(title: &str) -> Option<usize> {
    let location = title
        .split_once(" in ")
        .map(|(path, _)| path)
        .or_else(|| title.split_once(" - ").map(|(location, _)| location))
        .unwrap_or(title);
    let (_, line) = location.rsplit_once(':')?;
    line.chars()
        .all(|ch| ch.is_ascii_digit())
        .then(|| line.parse().ok())
        .flatten()
}

pub(super) fn first_body_hunk_new_line(body: &[String]) -> Option<usize> {
    body.iter()
        .find_map(|line| parse_hunk_header(line).map(|(_, new_line)| new_line))
}
