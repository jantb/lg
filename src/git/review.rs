use anyhow::Result;
use std::collections::BTreeSet;

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE};

use super::{head_branch, preferred_commit_ref, run};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewFile {
    status: String,
    path: String,
    old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewEntryPoint {
    path: String,
    line: Option<usize>,
    symbol: String,
    description: String,
    hunk: String,
    patch: Vec<String>,
    context: Vec<String>,
    added: usize,
    removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistedReview {
    pub report: String,
    pub nodes: Vec<ReviewNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNode {
    pub id: String,
    pub parent: Option<String>,
    pub depth: u16,
    pub title: String,
    pub body: Vec<String>,
    pub context: Vec<String>,
}

struct ReviewRender<'a> {
    branch: &'a str,
    base_ref: &'a str,
    merge_base: &'a str,
    commits: &'a [String],
    files: &'a [ReviewFile],
    stat: &'a str,
    entries: &'a [ReviewEntryPoint],
    diff: &'a str,
}

pub fn assisted_review_against_main() -> Result<String> {
    Ok(build_assisted_review_against_main()?.report)
}

pub fn build_assisted_review_against_main() -> Result<AssistedReview> {
    let base_ref =
        preferred_commit_ref(&format!("{DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN}"), BRANCH_MAIN)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not find {DEFAULT_PUSH_REMOTE}/{BRANCH_MAIN} or {BRANCH_MAIN}"
                )
            })?;
    let branch = head_branch().unwrap_or_else(|_| "HEAD".to_string());
    let range = format!("{base_ref}...HEAD");

    let merge_base = run(&["merge-base", &base_ref, "HEAD"])
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    let commits = branch_review_commits(&base_ref)?;
    let files = branch_review_files(&range)?;
    let stat = run(&[
        "diff",
        "--ignore-all-space",
        "--stat",
        "--find-renames",
        &range,
    ])
    .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    .unwrap_or_default();
    let diff = run(&["diff", "--ignore-all-space", "--find-renames", &range])
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())?;
    let entries = review_entry_points(&diff);

    let render = ReviewRender {
        branch: &branch,
        base_ref: &base_ref,
        merge_base: &merge_base,
        commits: &commits,
        files: &files,
        stat: &stat,
        entries: &entries,
        diff: &diff,
    };
    let report = render_assisted_review(&render);
    let nodes = build_review_nodes(&render);

    Ok(AssistedReview { report, nodes })
}

fn branch_review_commits(base_ref: &str) -> Result<Vec<String>> {
    let range = format!("{base_ref}..HEAD");
    let out = run(&["log", "--oneline", "--decorate=no", &range])?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn branch_review_files(range: &str) -> Result<Vec<ReviewFile>> {
    let out = run(&[
        "diff",
        "--ignore-all-space",
        "--name-status",
        "--find-renames",
        range,
    ])?;
    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 && parts[0].starts_with('R') {
            files.push(ReviewFile {
                status: parts[0].to_string(),
                path: parts[2].to_string(),
                old_path: Some(parts[1].to_string()),
            });
        } else if parts.len() >= 2 {
            files.push(ReviewFile {
                status: parts[0].to_string(),
                path: parts[1].to_string(),
                old_path: None,
            });
        }
    }
    Ok(files)
}

fn review_entry_points(diff: &str) -> Vec<ReviewEntryPoint> {
    let mut entries = Vec::new();
    let mut current_path = String::new();
    let mut current_hunk: Option<ReviewHunk> = None;

    for line in diff.lines() {
        if let Some(path) = parse_review_diff_path(line) {
            flush_review_hunk(&mut entries, &current_path, current_hunk.take());
            current_path = path;
            continue;
        }
        if line.starts_with("@@") {
            flush_review_hunk(&mut entries, &current_path, current_hunk.take());
            let new_line = parse_new_hunk_start(line).unwrap_or(0);
            current_hunk = Some(ReviewHunk {
                start_line: new_line,
                current_line: new_line,
                first_added_line: None,
                hunk: line.to_string(),
                patch: vec![line.to_string()],
                added: 0,
                removed: 0,
            });
            continue;
        }
        if let Some(hunk) = current_hunk.as_mut() {
            hunk.patch.push(line.to_string());
            if line.starts_with('+') && !line.starts_with("+++") {
                hunk.added += 1;
                hunk.first_added_line.get_or_insert(hunk.current_line);
                hunk.current_line = hunk.current_line.saturating_add(1);
            } else if line.starts_with('-') && !line.starts_with("---") {
                hunk.removed += 1;
            } else if !line.starts_with('\\') {
                hunk.current_line = hunk.current_line.saturating_add(1);
            }
        }
    }
    flush_review_hunk(&mut entries, &current_path, current_hunk.take());
    entries
}

struct ReviewHunk {
    start_line: usize,
    current_line: usize,
    first_added_line: Option<usize>,
    hunk: String,
    patch: Vec<String>,
    added: usize,
    removed: usize,
}

fn parse_review_diff_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let (_, b_path) = rest.split_once(" b/")?;
    Some(b_path.to_owned())
}

fn flush_review_hunk(entries: &mut Vec<ReviewEntryPoint>, path: &str, hunk: Option<ReviewHunk>) {
    let Some(hunk) = hunk else {
        return;
    };
    if path.is_empty() {
        return;
    }
    if is_import_only_hunk(path, &hunk.patch) {
        return;
    }
    let line = hunk.first_added_line.unwrap_or(hunk.start_line);
    let symbol = infer_entry_symbol(path, line, &hunk.hunk);
    let context = source_context(path, line);
    let description = describe_hunk(&hunk.patch, hunk.added, hunk.removed);
    entries.push(ReviewEntryPoint {
        path: path.to_string(),
        line: (line > 0).then_some(line),
        symbol,
        description,
        hunk: hunk.hunk,
        patch: hunk.patch,
        context,
        added: hunk.added,
        removed: hunk.removed,
    });
}

fn is_import_only_hunk(path: &str, patch: &[String]) -> bool {
    let mut changed = 0usize;
    for line in patch
        .iter()
        .filter(|line| line.starts_with('+') || line.starts_with('-'))
        .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
    {
        let body = line[1..].trim();
        if body.is_empty() {
            continue;
        }
        changed += 1;
        if !is_import_line(path, body) {
            return false;
        }
    }
    changed > 0
}

fn is_import_line(path: &str, line: &str) -> bool {
    let line = line
        .strip_prefix("pub ")
        .or_else(|| line.strip_prefix("public "))
        .unwrap_or(line);
    if path.ends_with(".rs") {
        line.starts_with("use ") || line.starts_with("extern crate ")
    } else if matches_kotlin_path(path) || path.ends_with(".java") {
        line.starts_with("import ") || line.starts_with("package ")
    } else {
        line.starts_with("import ") || line.starts_with("from ") || line.starts_with("export ")
    }
}

fn describe_hunk(patch: &[String], added: usize, removed: usize) -> String {
    let operation = match (added > 0, removed > 0) {
        (true, true) => "updates",
        (true, false) => "adds",
        (false, true) => "removes",
        (false, false) => "touches",
    };
    let mut signals = Vec::new();
    for line in patch
        .iter()
        .filter(|line| line.starts_with('+') || line.starts_with('-'))
        .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
    {
        collect_signal_words(&line[1..], &mut signals);
        if signals.len() >= 4 {
            break;
        }
    }
    if signals.is_empty() {
        format!("{operation} this block (+{added} -{removed})")
    } else {
        format!(
            "{operation} {} (+{added} -{removed})",
            signals.into_iter().take(4).collect::<Vec<_>>().join(", ")
        )
    }
}

fn collect_signal_words(line: &str, signals: &mut Vec<String>) {
    let trimmed = line.trim();
    if trimmed.is_empty() || matches!(trimmed, "{" | "}" | ");" | ")" | "]") {
        return;
    }
    for word in trimmed
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map(str::trim)
        .filter(|word| word.chars().count() >= 3)
        .filter(|word| !is_low_signal_word(word))
    {
        let word = truncate_review_text(word, 32);
        if !signals.contains(&word) {
            signals.push(word);
        }
        if signals.len() >= 4 {
            return;
        }
    }
}

fn is_low_signal_word(word: &str) -> bool {
    matches!(
        word,
        "let"
            | "mut"
            | "pub"
            | "fn"
            | "impl"
            | "self"
            | "Some"
            | "None"
            | "true"
            | "false"
            | "String"
            | "Vec"
            | "format"
            | "return"
            | "val"
            | "var"
            | "fun"
            | "class"
            | "object"
    )
}

fn parse_new_hunk_start(line: &str) -> Option<usize> {
    let plus = line.find(" +")? + 2;
    let rest = &line[plus..];
    let end = rest
        .find(|c: char| c == ',' || c.is_whitespace())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn infer_entry_symbol(path: &str, line: usize, hunk: &str) -> String {
    if path.ends_with(".rs") {
        if let Some(symbol) = infer_rust_symbol(path, line) {
            return symbol;
        }
    }
    if matches_kotlin_path(path) {
        if let Some(symbol) = infer_kotlin_symbol(path, line) {
            return symbol;
        }
    }
    if let Some(symbol) = hunk_symbol(hunk) {
        return symbol;
    }
    "file scope".to_string()
}

fn hunk_symbol(hunk: &str) -> Option<String> {
    let symbol = hunk.rsplit("@@").next()?.trim();
    if symbol.is_empty()
        || symbol == "where"
        || symbol.starts_with("use ")
        || symbol.starts_with("impl ")
    {
        return None;
    }
    Some(truncate_review_text(symbol, 96))
}

fn infer_rust_symbol(path: &str, line: usize) -> Option<String> {
    infer_source_symbol(path, line, rust_item_label)
}

fn infer_kotlin_symbol(path: &str, line: usize) -> Option<String> {
    infer_source_symbol(path, line, kotlin_item_label)
}

fn infer_source_symbol(
    path: &str,
    line: usize,
    label: fn(&str) -> Option<String>,
) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let target = line.saturating_sub(1);
    let lines: Vec<&str> = text.lines().collect();
    let start = target.saturating_sub(160);
    for raw in lines
        .get(start..=target.min(lines.len().saturating_sub(1)))?
        .iter()
        .rev()
    {
        let trimmed = raw.trim_start();
        if let Some(symbol) = label(trimmed) {
            return Some(symbol);
        }
    }
    None
}

fn matches_kotlin_path(path: &str) -> bool {
    path.ends_with(".kt") || path.ends_with(".kts")
}

fn rust_item_label(line: &str) -> Option<String> {
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    for prefix in [
        "async fn ",
        "fn ",
        "impl ",
        "trait ",
        "struct ",
        "enum ",
        "mod ",
        "const ",
        "static ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| c == '(' || c == '<' || c == ':' || c == '{' || c.is_whitespace())
                .next()
                .unwrap_or(rest)
                .trim();
            if !name.is_empty() {
                return Some(format!("{} {name}", prefix.trim_end()));
            }
        }
    }
    None
}

fn kotlin_item_label(line: &str) -> Option<String> {
    let line = line
        .strip_prefix("private ")
        .or_else(|| line.strip_prefix("internal "))
        .or_else(|| line.strip_prefix("protected "))
        .or_else(|| line.strip_prefix("public "))
        .unwrap_or(line);
    let line = line
        .strip_prefix("suspend ")
        .or_else(|| line.strip_prefix("inline "))
        .unwrap_or(line);
    for prefix in [
        "fun ",
        "class ",
        "data class ",
        "sealed class ",
        "enum class ",
        "object ",
        "interface ",
        "companion object",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| c == '(' || c == '<' || c == ':' || c == '{' || c.is_whitespace())
                .next()
                .unwrap_or(rest)
                .trim();
            let label = prefix.trim_end();
            if prefix == "companion object" {
                return Some(label.to_string());
            }
            if !name.is_empty() {
                return Some(format!("{label} {name}"));
            }
        }
    }
    None
}

fn source_context(path: &str, line: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let target = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    let start = find_source_item_start(path, &lines, target).unwrap_or(target.saturating_sub(8));
    let end = find_source_item_end(&lines, start)
        .unwrap_or_else(|| target.saturating_add(24).min(lines.len().saturating_sub(1)));

    lines[start..=end]
        .iter()
        .enumerate()
        .map(|(idx, text)| format!("{:>5} | {}", start + idx + 1, text))
        .collect()
}

fn find_source_item_start(path: &str, lines: &[&str], target: usize) -> Option<usize> {
    let start = target.saturating_sub(160);
    for (idx, raw) in lines.iter().enumerate().take(target + 1).skip(start).rev() {
        let trimmed = raw.trim_start();
        let is_item = if path.ends_with(".rs") {
            rust_item_label(trimmed).is_some()
        } else if matches_kotlin_path(path) {
            kotlin_item_label(trimmed).is_some()
        } else {
            false
        };
        if is_item {
            return Some(idx);
        }
    }
    None
}

fn find_source_item_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut balance = 0isize;
    let mut saw_open = false;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '{' => {
                    balance += 1;
                    saw_open = true;
                }
                '}' => balance -= 1,
                _ => {}
            }
        }
        if saw_open && balance <= 0 {
            return Some(idx);
        }
        if !saw_open && idx > start && line.trim().is_empty() {
            return Some(idx.saturating_sub(1));
        }
    }
    (!lines.is_empty()).then_some(lines.len() - 1)
}

fn build_review_nodes(review: &ReviewRender<'_>) -> Vec<ReviewNode> {
    let mut nodes = Vec::new();
    push_entry_nodes(
        &mut nodes,
        "branch",
        "Full diff against main",
        review.entries,
        review.diff.trim().is_empty(),
    );
    nodes.push(ReviewNode {
        id: "summary".to_string(),
        parent: None,
        depth: 0,
        title: format!(
            "Summary: {} vs {} ({} commits, {} files)",
            review.branch,
            review.base_ref,
            review.commits.len(),
            review.files.len()
        ),
        body: effect_summary(review.files, review.entries, review.commits),
        context: Vec::new(),
    });
    nodes.push(ReviewNode {
        id: "checklist".to_string(),
        parent: None,
        depth: 0,
        title: "Review checklist".to_string(),
        body: review_checklist(review.files, review.entries),
        context: Vec::new(),
    });
    nodes
}

fn push_entry_nodes(
    nodes: &mut Vec<ReviewNode>,
    prefix: &str,
    title: &str,
    entries: &[ReviewEntryPoint],
    empty: bool,
) {
    let root_id = prefix.to_string();
    nodes.push(ReviewNode {
        id: root_id.clone(),
        parent: None,
        depth: 0,
        title: title.to_string(),
        body: if empty {
            vec!["(empty)".to_string()]
        } else if entries.is_empty() {
            vec!["(only import changes hidden)".to_string()]
        } else {
            Vec::new()
        },
        context: Vec::new(),
    });
    if entries.is_empty() {
        return;
    }
    let groups = entry_groups(entries);
    let parents = entry_group_parents(entries, &groups);
    let tree = EntryTree {
        prefix,
        entries,
        groups: &groups,
        parents: &parents,
    };
    let mut emitted = BTreeSet::new();
    for (path, group_indices) in groups_by_path(
        groups
            .iter()
            .enumerate()
            .filter(|(group_idx, _)| parents[*group_idx].is_none())
            .map(|(group_idx, _)| group_idx),
        &groups,
    ) {
        tree.push_file(nodes, &path, &group_indices, &root_id, 1, &mut emitted);
    }
    for (group_idx, group) in groups.iter().enumerate() {
        tree.push_file(nodes, &group.path, &[group_idx], &root_id, 1, &mut emitted);
    }
}

struct EntryTree<'a> {
    prefix: &'a str,
    entries: &'a [ReviewEntryPoint],
    groups: &'a [EntryGroup],
    parents: &'a [Option<usize>],
}

impl EntryTree<'_> {
    fn push_file(
        &self,
        nodes: &mut Vec<ReviewNode>,
        path: &str,
        group_indices: &[usize],
        parent_id: &str,
        depth: u16,
        emitted: &mut BTreeSet<usize>,
    ) {
        let pending: Vec<usize> = group_indices
            .iter()
            .copied()
            .filter(|group_idx| !emitted.contains(group_idx))
            .collect();
        if pending.is_empty() {
            return;
        }

        let file_id = format!("{}:file:{}", self.prefix, pending[0]);
        nodes.push(ReviewNode {
            id: file_id.clone(),
            parent: Some(parent_id.to_string()),
            depth,
            title: self.file_title(path),
            body: self.file_patch_body(path),
            context: Vec::new(),
        });
        for group_idx in pending {
            self.push_group(nodes, group_idx, &file_id, depth.saturating_add(1), emitted);
        }
    }

    fn push_group(
        &self,
        nodes: &mut Vec<ReviewNode>,
        group_idx: usize,
        parent_id: &str,
        depth: u16,
        emitted: &mut BTreeSet<usize>,
    ) {
        if !emitted.insert(group_idx) {
            return;
        }

        let group = &self.groups[group_idx];
        let group_id = format!("{}:entry:{group_idx}", self.prefix);
        nodes.push(ReviewNode {
            id: group_id.clone(),
            parent: Some(parent_id.to_string()),
            depth,
            title: format!(
                "{} in {} - {}",
                group.path,
                group.symbol,
                entry_group_description(self.entries, group)
            ),
            body: self.group_patch_body(group),
            context: Vec::new(),
        });
        for idx in &group.indices {
            let entry = &self.entries[*idx];
            let location = entry
                .line
                .map(|line| format!(":{line}"))
                .unwrap_or_default();
            nodes.push(ReviewNode {
                id: format!("{}:hunk:{idx}", self.prefix),
                parent: Some(group_id.clone()),
                depth: depth.saturating_add(1),
                title: format!("{}{} - {}", entry.path, location, entry.description),
                body: std::iter::once(format!("effect: {}", entry.description))
                    .chain(entry.patch.iter().cloned())
                    .collect(),
                context: entry.context.clone(),
            });
        }

        for child_idx in 0..self.groups.len() {
            if self.parents[child_idx] == Some(group_idx) {
                if self.groups[child_idx].path == group.path {
                    self.push_group(
                        nodes,
                        child_idx,
                        &group_id,
                        depth.saturating_add(1),
                        emitted,
                    );
                } else {
                    self.push_file(
                        nodes,
                        &self.groups[child_idx].path,
                        &[child_idx],
                        &group_id,
                        depth.saturating_add(1),
                        emitted,
                    );
                }
            }
        }
    }

    fn file_title(&self, path: &str) -> String {
        let mut entry_count = 0usize;
        let mut added = 0usize;
        let mut removed = 0usize;
        for group in self.groups.iter().filter(|group| group.path == path) {
            entry_count += 1;
            added += group
                .indices
                .iter()
                .map(|idx| self.entries[*idx].added)
                .sum::<usize>();
            removed += group
                .indices
                .iter()
                .map(|idx| self.entries[*idx].removed)
                .sum::<usize>();
        }
        let noun = if entry_count == 1 {
            "entry point"
        } else {
            "entry points"
        };
        format!("{path} - {entry_count} {noun} (+{added} -{removed})")
    }

    fn file_patch_body(&self, path: &str) -> Vec<String> {
        self.groups
            .iter()
            .filter(|group| group.path == path)
            .flat_map(|group| {
                group
                    .indices
                    .iter()
                    .flat_map(|idx| self.entries[*idx].patch.iter().cloned())
            })
            .collect()
    }

    fn group_patch_body(&self, group: &EntryGroup) -> Vec<String> {
        group
            .indices
            .iter()
            .flat_map(|idx| self.entries[*idx].patch.iter().cloned())
            .collect()
    }
}

struct EntryGroup {
    path: String,
    symbol: String,
    indices: Vec<usize>,
}

fn entry_group_description(entries: &[ReviewEntryPoint], group: &EntryGroup) -> String {
    let added: usize = group.indices.iter().map(|idx| entries[*idx].added).sum();
    let removed: usize = group.indices.iter().map(|idx| entries[*idx].removed).sum();
    if group.indices.len() == 1 {
        entries[group.indices[0]].description.clone()
    } else {
        format!(
            "{} hunks update this entry point (+{added} -{removed})",
            group.indices.len()
        )
    }
}

fn entry_groups(entries: &[ReviewEntryPoint]) -> Vec<EntryGroup> {
    let mut groups = Vec::<EntryGroup>::new();
    for (idx, entry) in entries.iter().enumerate() {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.path == entry.path && group.symbol == entry.symbol)
        {
            group.indices.push(idx);
        } else {
            groups.push(EntryGroup {
                path: entry.path.clone(),
                symbol: entry.symbol.clone(),
                indices: vec![idx],
            });
        }
    }
    groups
}

fn groups_by_path<I>(indices: I, groups: &[EntryGroup]) -> Vec<(String, Vec<usize>)>
where
    I: IntoIterator<Item = usize>,
{
    let mut files = Vec::<(String, Vec<usize>)>::new();
    for idx in indices {
        let path = &groups[idx].path;
        if let Some((_, group_indices)) = files.iter_mut().find(|(candidate, _)| candidate == path)
        {
            group_indices.push(idx);
        } else {
            files.push((path.clone(), vec![idx]));
        }
    }
    files
}

fn entry_group_parents(entries: &[ReviewEntryPoint], groups: &[EntryGroup]) -> Vec<Option<usize>> {
    groups
        .iter()
        .enumerate()
        .map(|(callee_idx, callee)| {
            let callable = callable_symbol_name(&callee.symbol)?;
            groups
                .iter()
                .enumerate()
                .filter(|(caller_idx, _)| *caller_idx != callee_idx)
                .filter(|(_, caller)| entry_group_references(entries, caller, &callable))
                .map(|(caller_idx, _)| caller_idx)
                .next()
        })
        .collect()
}

fn entry_group_references(
    entries: &[ReviewEntryPoint],
    group: &EntryGroup,
    callable: &str,
) -> bool {
    group
        .indices
        .iter()
        .any(|idx| patch_references_callable(&entries[*idx].patch, callable))
}

fn patch_references_callable(patch: &[String], callable: &str) -> bool {
    patch
        .iter()
        .filter_map(|line| {
            line.strip_prefix('+')
                .or_else(|| line.strip_prefix(' '))
                .or_else(|| line.strip_prefix('-'))
        })
        .any(|line| line_references_callable(line, callable))
}

fn line_references_callable(line: &str, callable: &str) -> bool {
    line.match_indices(callable).any(|(idx, _)| {
        let before = line[..idx].chars().next_back();
        let after = line[idx + callable.len()..].chars().next();
        !before.is_some_and(is_ident_continue) && !after.is_some_and(is_ident_continue)
    })
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn callable_symbol_name(symbol: &str) -> Option<String> {
    let rest = symbol
        .strip_prefix("fn ")
        .or_else(|| symbol.strip_prefix("async fn "))
        .or_else(|| symbol.strip_prefix("fun "))?;
    let name = rest
        .split(|ch: char| ch == '(' || ch == '<' || ch == ':' || ch == '{' || ch.is_whitespace())
        .next()
        .unwrap_or(rest)
        .trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn render_assisted_review(review: &ReviewRender<'_>) -> String {
    let mut out = String::new();
    out.push_str("Assisted review against main\n");
    out.push_str("============================\n\n");
    out.push_str(&format!("Branch: {}\n", review.branch));
    out.push_str(&format!("Base: {}\n", review.base_ref));
    if !review.merge_base.is_empty() {
        out.push_str(&format!("Merge base: {}\n", short_oid(review.merge_base)));
    }
    out.push_str(&format!(
        "Scope: {} commit{}, {} file{}\n",
        review.commits.len(),
        plural(review.commits.len()),
        review.files.len(),
        plural(review.files.len())
    ));
    out.push_str("\nEffect summary\n");
    out.push_str("--------------\n");
    for line in effect_summary(review.files, review.entries, review.commits) {
        out.push_str("- ");
        out.push_str(&line);
        out.push('\n');
    }

    if !review.commits.is_empty() {
        out.push_str("\nCommits in review range\n");
        out.push_str("-----------------------\n");
        for commit in review.commits.iter().take(24) {
            out.push_str("- ");
            out.push_str(commit);
            out.push('\n');
        }
        if review.commits.len() > 24 {
            out.push_str(&format!(
                "- ... {} more commits\n",
                review.commits.len() - 24
            ));
        }
    }

    out.push_str("\nFiles changed\n");
    out.push_str("-------------\n");
    if review.files.is_empty() {
        out.push_str("- No committed branch diff against main.\n");
    } else {
        for file in review.files {
            out.push_str("- ");
            out.push_str(&file.status);
            out.push(' ');
            if let Some(old) = &file.old_path {
                out.push_str(old);
                out.push_str(" -> ");
            }
            out.push_str(&file.path);
            out.push('\n');
        }
    }

    if !review.stat.trim().is_empty() {
        out.push_str("\nDiffstat\n");
        out.push_str("--------\n");
        out.push_str(review.stat.trim_end());
        out.push('\n');
    }

    out.push_str("\nEntry point trace\n");
    out.push_str("-----------------\n");
    if review.entries.is_empty() {
        out.push_str("- No patch hunks found in the branch diff.\n");
    } else {
        render_entry_points(&mut out, review.entries);
    }
    out.push_str("\nReview checklist\n");
    out.push_str("----------------\n");
    for line in review_checklist(review.files, review.entries) {
        out.push_str("- ");
        out.push_str(&line);
        out.push('\n');
    }

    out.push_str("\nFull diff against main\n");
    out.push_str("----------------------\n");
    if review.diff.trim().is_empty() {
        out.push_str("(empty)\n");
    } else {
        out.push_str(review.diff.trim_end());
        out.push('\n');
    }
    out
}

fn effect_summary(
    files: &[ReviewFile],
    entries: &[ReviewEntryPoint],
    commits: &[String],
) -> Vec<String> {
    let mut lines = Vec::new();
    if files.is_empty() {
        lines.push("No committed branch changes were found against main.".to_string());
    } else {
        lines.push(format!(
            "The branch changes {} file{} across {} commit{}.",
            files.len(),
            plural(files.len()),
            commits.len(),
            plural(commits.len())
        ));
    }

    let mut areas = Vec::new();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();

    if paths.contains(&"src/app.rs") {
        areas.push("runtime orchestration and keyboard/job handling");
    }
    if paths.contains(&"src/state.rs") {
        areas.push("application state");
    }
    if paths.contains(&"src/git.rs") {
        areas.push("Git integration");
    }
    if paths.iter().any(|path| path.starts_with("src/panel/")) {
        areas.push("terminal UI panels");
    }
    if paths.iter().any(|path| path.starts_with("tests/")) {
        areas.push("test coverage");
    }
    if paths
        .iter()
        .any(|path| matches!(*path, "Cargo.toml" | "Cargo.lock" | "Makefile"))
    {
        areas.push("build or dependency configuration");
    }
    if !areas.is_empty() {
        lines.push(format!("Primary touched areas: {}.", areas.join(", ")));
    }

    let mut symbols: Vec<String> = entries
        .iter()
        .filter(|entry| entry.symbol != "file scope")
        .map(|entry| entry.symbol.clone())
        .collect();
    symbols.sort();
    symbols.dedup();
    if !symbols.is_empty() {
        lines.push(format!(
            "Start tracing at: {}{}.",
            symbols
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            if symbols.len() > 8 { ", ..." } else { "" }
        ));
    }

    lines
}

fn render_entry_points(out: &mut String, entries: &[ReviewEntryPoint]) {
    let mut last_path = "";
    for entry in entries {
        if entry.path != last_path {
            out.push_str(&format!("\n{}\n", entry.path));
            last_path = &entry.path;
        }
        let location = entry
            .line
            .map(|line| format!(":{}", line))
            .unwrap_or_default();
        out.push_str(&format!(
            "- {}{} in {} - {}\n",
            entry.path, location, entry.symbol, entry.description
        ));
        out.push_str("  ");
        out.push_str(&truncate_review_text(&entry.hunk, 140));
        out.push('\n');
    }
}

fn review_checklist(files: &[ReviewFile], entries: &[ReviewEntryPoint]) -> Vec<String> {
    let mut lines = Vec::new();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();

    if paths.contains(&"src/git.rs") {
        lines.push(
            "Verify Git commands on a temporary repository before trusting the workflow."
                .to_string(),
        );
    }
    if paths.contains(&"src/app.rs") || paths.contains(&"src/state.rs") {
        lines.push(
            "Check state transitions and background jobs for stale output or focus changes."
                .to_string(),
        );
    }
    if paths.iter().any(|path| path.starts_with("src/panel/")) {
        lines.push(
            "Exercise the affected keybindings and render at narrow terminal widths.".to_string(),
        );
    }
    if !paths.iter().any(|path| path.starts_with("tests/")) && !entries.is_empty() {
        lines.push(
            "No test files changed; consider adding coverage for the user-visible flow."
                .to_string(),
        );
    }
    if lines.is_empty() {
        lines.push(
            "Review the entry point trace and diffstat, then run the standard test command."
                .to_string(),
        );
    }
    lines
}

fn truncate_review_text(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn short_oid(oid: &str) -> &str {
    oid.get(..12).unwrap_or(oid)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_hunk_start_reads_added_side() {
        assert_eq!(parse_new_hunk_start("@@ -12,3 +42,8 @@ fn main"), Some(42));
        assert_eq!(parse_new_hunk_start("@@ -1 +1 @@"), Some(1));
        assert_eq!(parse_new_hunk_start("not a hunk"), None);
    }

    #[test]
    fn is_import_line_recognises_per_language() {
        assert!(is_import_line("foo.rs", "use std::fs;"));
        assert!(is_import_line("foo.rs", "extern crate serde;"));
        assert!(!is_import_line("foo.rs", "let x = 1;"));

        assert!(is_import_line("foo.kt", "import a.b.C"));
        assert!(is_import_line("foo.kt", "package a.b"));
        // Public modifier in front of an import still counts.
        assert!(is_import_line("foo.kt", "public import a.b.C"));
    }

    #[test]
    fn is_import_only_hunk_returns_true_only_when_all_changes_are_imports() {
        let imports = vec![
            "@@ -1 +1 @@".to_string(),
            "+use std::fs;".to_string(),
            "-use std::io;".to_string(),
        ];
        assert!(is_import_only_hunk("foo.rs", &imports));

        let mixed = vec![
            "@@ -1 +1 @@".to_string(),
            "+use std::fs;".to_string(),
            "+let x = 1;".to_string(),
        ];
        assert!(!is_import_only_hunk("foo.rs", &mixed));

        // No actual changes (only headers / context) → false.
        let empty = vec!["@@ -1 +1 @@".to_string(), " context".to_string()];
        assert!(!is_import_only_hunk("foo.rs", &empty));
    }

    #[test]
    fn describe_hunk_picks_operation_word_from_added_removed() {
        let patch = vec!["+let foo = 1;".to_string()];
        let desc = describe_hunk(&patch, 1, 0);
        assert!(desc.starts_with("adds "));
        assert!(desc.contains("(+1 -0)"));

        let patch = vec!["-let bar = 1;".to_string()];
        let desc = describe_hunk(&patch, 0, 1);
        assert!(desc.starts_with("removes "));

        let patch = vec!["+a".to_string(), "-b".to_string()];
        let desc = describe_hunk(&patch, 1, 1);
        assert!(desc.starts_with("updates "));
    }

    #[test]
    fn rust_item_label_extracts_named_items() {
        assert_eq!(rust_item_label("fn render() {"), Some("fn render".into()));
        assert_eq!(
            rust_item_label("pub async fn build() -> Result<()> {"),
            Some("async fn build".into())
        );
        assert_eq!(
            rust_item_label("pub(crate) struct AppState {"),
            Some("struct AppState".into())
        );
        assert_eq!(rust_item_label("let x = 1;"), None);
    }

    #[test]
    fn kotlin_item_label_strips_visibility_and_modifier_prefixes() {
        assert_eq!(
            kotlin_item_label("private fun handle(): Int {"),
            Some("fun handle".into())
        );
        assert_eq!(
            kotlin_item_label("data class Point(val x: Int)"),
            Some("data class Point".into())
        );
        assert_eq!(
            kotlin_item_label("companion object {"),
            Some("companion object".into())
        );
        assert_eq!(kotlin_item_label("val n = 1"), None);
    }

    #[test]
    fn callable_symbol_name_strips_signature_punctuation() {
        assert_eq!(callable_symbol_name("fn foo()"), Some("foo".into()));
        assert_eq!(
            callable_symbol_name("async fn build<T>(x: T)"),
            Some("build".into())
        );
        assert_eq!(
            callable_symbol_name("fun handle(x: Int)"),
            Some("handle".into())
        );
        assert_eq!(callable_symbol_name("struct AppState"), None);
    }

    #[test]
    fn line_references_callable_only_matches_whole_idents() {
        assert!(line_references_callable("    foo();", "foo"));
        assert!(line_references_callable("foo + 1", "foo"));
        // No match when surrounded by ident chars.
        assert!(!line_references_callable("foobar();", "foo"));
        assert!(!line_references_callable("let _foo = 1;", "foo"));
    }

    #[test]
    fn truncate_review_text_appends_ellipsis_when_cut() {
        assert_eq!(truncate_review_text("short", 10), "short");
        assert_eq!(truncate_review_text("0123456789xyz", 10), "0123456789...");
    }
}
