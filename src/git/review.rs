use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::config::{BRANCH_MAIN, DEFAULT_PUSH_REMOTE};

use super::{head_branch, preferred_commit_ref, run};

mod source;

use source::{infer_entry_symbol, matches_kotlin_path, source_context};
#[cfg(test)]
use source::{kotlin_item_label, rust_item_label};

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

    let merge_base = run(&["merge-base", &base_ref, "HEAD"])
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    let diff_base = if merge_base.is_empty() {
        base_ref.as_str()
    } else {
        merge_base.as_str()
    };
    let commits = branch_review_commits(&base_ref)?;
    let files = worktree_review_files(diff_base)?;
    let stat = worktree_review_stat(diff_base).unwrap_or_default();
    let diff = worktree_review_diff(diff_base)?;
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

fn worktree_review_files(base: &str) -> Result<Vec<ReviewFile>> {
    let out = run(&[
        "diff",
        "--ignore-all-space",
        "--name-status",
        "--find-renames",
        base,
    ])?;
    let mut files = parse_review_files(&String::from_utf8_lossy(&out.stdout));
    for path in untracked_paths(".")? {
        if !files.iter().any(|file| file.path == path) {
            files.push(ReviewFile {
                status: "A".to_string(),
                path,
                old_path: None,
            });
        }
    }
    Ok(files)
}

fn parse_review_files(text: &str) -> Vec<ReviewFile> {
    let mut files = Vec::new();
    for line in text.lines() {
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
    files
}

fn worktree_review_stat(base: &str) -> Result<String> {
    let out = run(&[
        "diff",
        "--ignore-all-space",
        "--stat",
        "--find-renames",
        base,
    ])?;
    let mut stat = String::from_utf8_lossy(&out.stdout).into_owned();
    for path in untracked_paths(".")? {
        let untracked = untracked_file_stat(&path)?;
        append_review_diff_part(&mut stat, &untracked);
    }
    Ok(stat)
}

fn worktree_review_diff(base: &str) -> Result<String> {
    let out = run(&["diff", "--ignore-all-space", "--find-renames", base])?;
    let mut diff = String::from_utf8_lossy(&out.stdout).into_owned();
    for path in untracked_paths(".")? {
        let untracked = untracked_file_diff(&path)?;
        append_review_diff_part(&mut diff, &untracked);
    }
    Ok(diff)
}

fn append_review_diff_part(out: &mut String, part: &str) {
    if part.trim().is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(part);
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

fn untracked_paths(pathspec: &str) -> Result<Vec<String>> {
    let out = run(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
        pathspec,
    ])?;
    Ok(out
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect())
}

fn untracked_file_stat(path: &str) -> Result<String> {
    untracked_no_index_diff(
        path,
        &["diff", "--no-index", "--stat", "--", "/dev/null", path],
    )
}

fn untracked_file_diff(path: &str) -> Result<String> {
    untracked_no_index_diff(path, &["diff", "--no-index", "--", "/dev/null", path])
}

fn untracked_no_index_diff(path: &str, args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).output()?;
    if out.status.success() || out.status.code() == Some(1) {
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        normalize_no_index_path(&mut text, path);
        Ok(text)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(anyhow::anyhow!(
            "git {} failed for {path}: {}",
            args.join(" "),
            stderr.trim()
        ))
    }
}

fn normalize_no_index_path(diff: &mut String, path: &str) {
    let Some(file_name) = Path::new(path).file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let from = format!("b/{file_name}");
    let to = format!("b/{path}");
    *diff = diff.replace(&from, &to);
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
    let mut category_nodes = BTreeSet::new();
    for category in ReviewEntryCategory::ALL {
        for (path, group_indices) in groups_by_path(
            groups
                .iter()
                .enumerate()
                .filter(|(group_idx, group)| {
                    parents[*group_idx].is_none()
                        && ReviewEntryCategory::for_path(&group.path) == category
                })
                .map(|(group_idx, _)| group_idx),
            &groups,
        ) {
            let category_id =
                ensure_review_category_node(nodes, prefix, &root_id, category, &mut category_nodes);
            tree.push_file(nodes, &path, &group_indices, &category_id, 2, &mut emitted);
        }
    }
    for (group_idx, group) in groups.iter().enumerate() {
        let category = ReviewEntryCategory::for_path(&group.path);
        let category_id =
            ensure_review_category_node(nodes, prefix, &root_id, category, &mut category_nodes);
        tree.push_file(
            nodes,
            &group.path,
            &[group_idx],
            &category_id,
            2,
            &mut emitted,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReviewEntryCategory {
    Production,
    Tests,
    Migrations,
    Docs,
    Other,
}

impl ReviewEntryCategory {
    const ALL: [Self; 5] = [
        Self::Production,
        Self::Tests,
        Self::Migrations,
        Self::Docs,
        Self::Other,
    ];

    fn for_path(path: &str) -> Self {
        if is_test_path(path) {
            Self::Tests
        } else if is_migration_path(path) {
            Self::Migrations
        } else if is_doc_path(path) {
            Self::Docs
        } else if is_production_path(path) {
            Self::Production
        } else {
            Self::Other
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Tests => "tests",
            Self::Migrations => "migrations",
            Self::Docs => "docs",
            Self::Other => "other",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Production => "Production",
            Self::Tests => "Tests",
            Self::Migrations => "Migrations",
            Self::Docs => "Docs",
            Self::Other => "Other",
        }
    }
}

fn ensure_review_category_node(
    nodes: &mut Vec<ReviewNode>,
    prefix: &str,
    root_id: &str,
    category: ReviewEntryCategory,
    emitted: &mut BTreeSet<ReviewEntryCategory>,
) -> String {
    let id = format!("{prefix}:category:{}", category.id());
    if emitted.insert(category) {
        nodes.push(ReviewNode {
            id: id.clone(),
            parent: Some(root_id.to_string()),
            depth: 1,
            title: category.label().to_string(),
            body: Vec::new(),
            context: Vec::new(),
        });
    }
    id
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
                .filter(|(_, caller)| {
                    ReviewEntryCategory::for_path(&caller.path)
                        == ReviewEntryCategory::for_path(&callee.path)
                })
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
    out.push_str("Diff source: merge-base to current worktree, including staged, unstaged, and untracked files.\n");
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
    if paths.iter().any(|path| is_test_path(path)) {
        areas.push("test coverage");
    }
    if paths
        .iter()
        .any(|path| matches!(*path, "Cargo.toml" | "Cargo.lock" | "Makefile"))
    {
        areas.push("build or dependency configuration");
    }
    if !areas.is_empty() {
        lines.push("Primary touched areas:".to_string());
        lines.extend(areas.into_iter().map(|area| format!("- {area}")));
    }

    let mut trace_points: Vec<String> = entries
        .iter()
        .filter(|entry| entry.symbol != "file scope")
        .map(trace_point)
        .collect();
    trace_points.sort();
    trace_points.dedup();
    if !trace_points.is_empty() {
        lines.push("Start tracing at:".to_string());
        lines.extend(trace_points.into_iter().map(|point| format!("- {point}")));
    }

    lines
}

fn trace_point(entry: &ReviewEntryPoint) -> String {
    let location = entry
        .line
        .map(|line| format!("{}:{line}", entry.path))
        .unwrap_or_else(|| entry.path.clone());
    format!("{} — {}", entry.symbol, location)
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
    let has_tests = paths.iter().any(|path| is_test_path(path));
    let has_non_tests = paths.iter().any(|path| !is_test_path(path));

    if paths.contains(&"src/git.rs") {
        lines.push(
            "- Verify Git commands on a temporary repository before trusting the workflow."
                .to_string(),
        );
    }
    if paths.contains(&"src/app.rs") || paths.contains(&"src/state.rs") {
        lines.push(
            "- Check state transitions and background jobs for stale output or focus changes."
                .to_string(),
        );
    }
    if paths.iter().any(|path| path.starts_with("src/panel/")) {
        lines.push(
            "- Exercise the affected keybindings and render at narrow terminal widths.".to_string(),
        );
    }

    if path_matches_any(&paths, &["kafka", "topic", "event", "request", "response"])
        || symbol_matches_any(entries, &["request", "response", "event", "topic"])
    {
        lines.push(
            "- Verify message contracts: topic names, serialized field names, nullable/default values, and backwards compatibility."
                .to_string(),
        );
    }
    if path_matches_any(&paths, &["adapter", "mapper", "converter"])
        || symbol_matches_any(entries, &["adapter", "mapper", "convert", "deserialize"])
    {
        lines.push(
            "- Check adapter and mapping boundaries with representative old and new payloads."
                .to_string(),
        );
    }
    if path_matches_any(&paths, &["service", "processor", "workflow", "flow"])
        || symbol_matches_any(entries, &["service", "processor", "workflow", "flow"])
    {
        lines.push(
            "- Trace service flow side effects, ordering, retries, and idempotency for partial failures."
                .to_string(),
        );
    }
    if path_matches_any(&paths, &["model", "dto", "request", "response"])
        || symbol_matches_any(entries, &["data class", "class", "enum"])
    {
        lines.push(
            "- Review model/API compatibility: required fields, defaults, validation, and renamed concepts."
                .to_string(),
        );
    }
    if path_matches_any(
        &paths,
        &["repository", "database", "migration", "cache", "dao"],
    ) || symbol_matches_any(entries, &["repository", "cache", "query"])
    {
        lines.push(
            "- Check persistence and cache behavior, including migrations, invalidation, and rollback expectations."
                .to_string(),
        );
    }
    if has_tests {
        lines.push(
            "- Run the changed tests plus the nearest broader suite that exercises the touched production paths."
                .to_string(),
        );
    } else if has_non_tests && !entries.is_empty() {
        lines.push(
            "- No test files changed; consider adding coverage for the user-visible flow."
                .to_string(),
        );
    }
    if !entries.is_empty() {
        lines.push(
            "- Use `l` on `Full diff against main` for an LLM pass over the whole change, then use `l` again on the highest-risk file or entry nodes for focused follow-up."
                .to_string(),
        );
    }
    if lines.is_empty() {
        lines.push(
            "- Review the entry point trace and diffstat, then run the standard test command."
                .to_string(),
        );
    }
    lines
}

fn path_matches_any(paths: &[&str], needles: &[&str]) -> bool {
    paths.iter().any(|path| {
        let lower = path.to_ascii_lowercase();
        needles.iter().any(|needle| lower.contains(needle))
    })
}

fn symbol_matches_any(entries: &[ReviewEntryPoint], needles: &[&str]) -> bool {
    entries.iter().any(|entry| {
        let lower = entry.symbol.to_ascii_lowercase();
        needles.iter().any(|needle| lower.contains(needle))
    })
}

fn is_test_path(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.starts_with("test/")
        || path.contains("/test/")
        || path.starts_with("src/test/")
        || path.contains("/src/test/")
}

fn is_migration_path(path: &str) -> bool {
    path.contains("/db/migration/") || path.starts_with("db/migration/") || path.ends_with(".sql")
}

fn is_doc_path(path: &str) -> bool {
    path.starts_with("docs/")
        || path.starts_with(".agent/")
        || path.ends_with(".md")
        || path.ends_with(".adoc")
        || path.ends_with(".rst")
        || path.ends_with(".txt")
}

fn is_production_path(path: &str) -> bool {
    (path.starts_with("src/") || path.starts_with("app/") || path.starts_with("lib/"))
        && !is_test_path(path)
        && !is_migration_path(path)
        && !is_doc_path(path)
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

    #[test]
    fn effect_summary_lists_all_entry_symbols() {
        let files = vec![ReviewFile {
            status: "M".into(),
            path: "src/lib.rs".into(),
            old_path: None,
        }];
        let entries = (0..10)
            .map(|idx| ReviewEntryPoint {
                path: "src/lib.rs".into(),
                line: Some(idx + 1),
                symbol: format!("fn symbol_{idx}"),
                description: "updates symbol".into(),
                hunk: String::new(),
                patch: Vec::new(),
                context: Vec::new(),
                added: 1,
                removed: 0,
            })
            .collect::<Vec<_>>();

        let summary = effect_summary(&files, &entries, &["abc123".into()]).join("\n");

        assert!(
            summary.contains("Start tracing at:\n- fn symbol_0 — src/lib.rs:1"),
            "{summary}"
        );
        assert!(summary.contains("fn symbol_9 — src/lib.rs:10"), "{summary}");
        assert!(
            !summary.contains("..."),
            "entry symbol list should not be truncated: {summary}"
        );
    }

    #[test]
    fn effect_summary_keeps_same_symbol_in_different_files() {
        let files = vec![
            ReviewFile {
                status: "M".into(),
                path: "src/a.kt".into(),
                old_path: None,
            },
            ReviewFile {
                status: "M".into(),
                path: "src/b.kt".into(),
                old_path: None,
            },
        ];
        let entries = ["src/a.kt", "src/b.kt"]
            .into_iter()
            .map(|path| ReviewEntryPoint {
                path: path.into(),
                line: Some(7),
                symbol: "fun update".into(),
                description: "updates flow".into(),
                hunk: String::new(),
                patch: Vec::new(),
                context: Vec::new(),
                added: 1,
                removed: 0,
            })
            .collect::<Vec<_>>();

        let summary = effect_summary(&files, &entries, &["abc123".into()]).join("\n");

        assert!(summary.contains("fun update — src/a.kt:7"), "{summary}");
        assert!(summary.contains("fun update — src/b.kt:7"), "{summary}");
    }

    #[test]
    fn review_checklist_recognizes_source_set_tests() {
        let files = vec![ReviewFile {
            status: "M".into(),
            path: "src/test/kotlin/no/spenn/gravy/adapter/model/HouseholdIdConversionTest.kt"
                .into(),
            old_path: None,
        }];
        let entries = vec![ReviewEntryPoint {
            path: "src/test/kotlin/no/spenn/gravy/adapter/model/HouseholdIdConversionTest.kt"
                .into(),
            line: Some(1),
            symbol: "class HouseholdIdConversionTest".into(),
            description: "updates coverage".into(),
            hunk: String::new(),
            patch: Vec::new(),
            context: Vec::new(),
            added: 1,
            removed: 0,
        }];

        let checklist = review_checklist(&files, &entries).join("\n");
        let summary = effect_summary(&files, &entries, &["abc123".into()]).join("\n");

        assert!(
            !checklist.contains("No test files changed"),
            "source-set test files should count as tests: {checklist}"
        );
        assert!(
            checklist.contains("Run the changed tests"),
            "changed tests should produce a concrete test prompt: {checklist}"
        );
        assert!(checklist.contains("Use `l`"), "{checklist}");
        assert!(
            summary.contains("Primary touched areas:\n- test coverage"),
            "{summary}"
        );
    }

    #[test]
    fn build_review_nodes_groups_files_by_review_category() {
        let entries = vec![
            review_entry("src/main/kotlin/App.kt"),
            review_entry("src/test/kotlin/AppTest.kt"),
            review_entry("src/main/resources/db/migration/V1__app.sql"),
            review_entry("docs/app.md"),
            review_entry(".gitignore"),
        ];
        let files = entries
            .iter()
            .map(|entry| ReviewFile {
                status: "M".into(),
                path: entry.path.clone(),
                old_path: None,
            })
            .collect::<Vec<_>>();
        let commits = vec!["abc123 update review".to_string()];
        let render = ReviewRender {
            branch: "feature",
            base_ref: "main",
            merge_base: "",
            commits: &commits,
            files: &files,
            stat: "",
            entries: &entries,
            diff: "diff",
        };

        let nodes = build_review_nodes(&render);

        assert!(
            nodes
                .iter()
                .any(|node| node.id == "branch:category:production"
                    && node.parent.as_deref() == Some("branch")
                    && node.depth == 1
                    && node.title == "Production"),
            "{nodes:#?}"
        );
        assert!(
            nodes.iter().any(|node| node.id == "branch:category:tests"
                && node.parent.as_deref() == Some("branch")
                && node.title == "Tests"),
            "{nodes:#?}"
        );
        assert!(
            nodes
                .iter()
                .any(|node| node.id == "branch:category:migrations"
                    && node.parent.as_deref() == Some("branch")
                    && node.title == "Migrations"),
            "{nodes:#?}"
        );
        assert!(
            nodes.iter().any(|node| node.id == "branch:category:docs"
                && node.parent.as_deref() == Some("branch")
                && node.title == "Docs"),
            "{nodes:#?}"
        );
        assert!(
            nodes.iter().any(|node| node.id == "branch:category:other"
                && node.parent.as_deref() == Some("branch")
                && node.title == "Other"),
            "{nodes:#?}"
        );

        let production_file = nodes
            .iter()
            .find(|node| node.title.starts_with("src/main/kotlin/App.kt - "))
            .expect("production file node");
        assert_eq!(
            production_file.parent.as_deref(),
            Some("branch:category:production")
        );
        assert_eq!(production_file.depth, 2);

        let test_file = nodes
            .iter()
            .find(|node| node.title.starts_with("src/test/kotlin/AppTest.kt - "))
            .expect("test file node");
        assert_eq!(test_file.parent.as_deref(), Some("branch:category:tests"));
        assert_eq!(test_file.depth, 2);
    }

    #[test]
    fn build_review_nodes_keeps_called_production_file_in_production_category() {
        let entries = vec![
            ReviewEntryPoint {
                path: "src/main/kotlin/app/service/BalanceService.kt".into(),
                line: Some(162),
                symbol: "fun updateBalanceCache".into(),
                description: "updates cache behavior (+1 -1)".into(),
                hunk: "@@ -162 +162 @@".into(),
                patch: vec![
                    "@@ -162 +162 @@".into(),
                    "-    oldCache(items)".into(),
                    "+    newCache(items)".into(),
                ],
                context: Vec::new(),
                added: 1,
                removed: 1,
            },
            ReviewEntryPoint {
                path: "src/test/kotlin/app/service/BalanceServiceTest.kt".into(),
                line: Some(332),
                symbol: "fun updateBalanceCacheTest".into(),
                description: "updates assertions (+2 -2)".into(),
                hunk: "@@ -332 +332 @@".into(),
                patch: vec![
                    "@@ -332 +332 @@".into(),
                    "     service.updateBalanceCache(items)".into(),
                    "-    assertThat(balance).isEqualTo(oldValue)".into(),
                    "+    assertThat(balance).isEqualTo(newValue)".into(),
                ],
                context: Vec::new(),
                added: 1,
                removed: 1,
            },
        ];
        let files = entries
            .iter()
            .map(|entry| ReviewFile {
                status: "M".into(),
                path: entry.path.clone(),
                old_path: None,
            })
            .collect::<Vec<_>>();
        let commits = vec!["abc123 update cache".to_string()];
        let render = ReviewRender {
            branch: "feature",
            base_ref: "main",
            merge_base: "",
            commits: &commits,
            files: &files,
            stat: "",
            entries: &entries,
            diff: "diff",
        };

        let nodes = build_review_nodes(&render);

        let production_file = nodes
            .iter()
            .find(|node| {
                node.title
                    .starts_with("src/main/kotlin/app/service/BalanceService.kt - ")
            })
            .expect("production file node");
        assert_eq!(
            production_file.parent.as_deref(),
            Some("branch:category:production"),
            "{nodes:#?}"
        );

        let test_file = nodes
            .iter()
            .find(|node| {
                node.title
                    .starts_with("src/test/kotlin/app/service/BalanceServiceTest.kt - ")
            })
            .expect("test file node");
        assert_eq!(
            test_file.parent.as_deref(),
            Some("branch:category:tests"),
            "{nodes:#?}"
        );
    }

    #[test]
    fn review_checklist_adds_domain_specific_prompts() {
        let files = vec![
            ReviewFile {
                status: "M".into(),
                path: "src/main/kotlin/app/adapter/kafka/BalanceUpdatedEvent.kt".into(),
                old_path: None,
            },
            ReviewFile {
                status: "M".into(),
                path: "src/main/kotlin/app/service/BalanceService.kt".into(),
                old_path: None,
            },
            ReviewFile {
                status: "M".into(),
                path: "src/main/kotlin/app/model/BalanceRequest.kt".into(),
                old_path: None,
            },
        ];
        let entries = vec![ReviewEntryPoint {
            path: "src/main/kotlin/app/service/BalanceService.kt".into(),
            line: Some(12),
            symbol: "class BalanceService".into(),
            description: "updates balance".into(),
            hunk: String::new(),
            patch: Vec::new(),
            context: Vec::new(),
            added: 1,
            removed: 0,
        }];

        let checklist = review_checklist(&files, &entries).join("\n");

        assert!(checklist.contains("message contracts"), "{checklist}");
        assert!(
            checklist.contains("adapter and mapping boundaries"),
            "{checklist}"
        );
        assert!(
            checklist.contains("service flow side effects"),
            "{checklist}"
        );
        assert!(checklist.contains("model/API compatibility"), "{checklist}");
        assert!(checklist.contains("LLM pass"), "{checklist}");
        assert!(
            checklist.contains("No test files changed"),
            "production-only domain changes should still ask about coverage: {checklist}"
        );
    }

    fn review_entry(path: &str) -> ReviewEntryPoint {
        ReviewEntryPoint {
            path: path.into(),
            line: Some(1),
            symbol: "file scope".into(),
            description: "adds item (+1 -0)".into(),
            hunk: "@@ -1 +1 @@".into(),
            patch: vec!["@@ -1 +1 @@".into(), "-old".into(), "+new".into()],
            context: Vec::new(),
            added: 1,
            removed: 0,
        }
    }
}
