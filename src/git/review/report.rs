//! The text report a review renders to.

use super::category::is_test_path;
use super::{ReviewEntryPoint, ReviewFile, ReviewRender, plural, short_oid, truncate_review_text};

pub(super) fn render_assisted_review(review: &ReviewRender<'_>) -> String {
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

pub(super) fn effect_summary(
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
            .map(|line| format!(":{line}"))
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

/// A prompt per domain, added when the change touches that domain's paths or
/// symbols. Listed in the order they appear in the checklist.
const DOMAIN_CHECKS: [(&[&str], &[&str], &str); 5] = [
    (
        &["kafka", "topic", "event", "request", "response"],
        &["request", "response", "event", "topic"],
        "- Verify message contracts: topic names, serialized field names, nullable/default values, and backwards compatibility.",
    ),
    (
        &["adapter", "mapper", "converter"],
        &["adapter", "mapper", "convert", "deserialize"],
        "- Check adapter and mapping boundaries with representative old and new payloads.",
    ),
    (
        &["service", "processor", "workflow", "flow"],
        &["service", "processor", "workflow", "flow"],
        "- Trace service flow side effects, ordering, retries, and idempotency for partial failures.",
    ),
    (
        &["model", "dto", "request", "response"],
        &["data class", "class", "enum"],
        "- Review model/API compatibility: required fields, defaults, validation, and renamed concepts.",
    ),
    (
        &["repository", "database", "migration", "cache", "dao"],
        &["repository", "cache", "query"],
        "- Check persistence and cache behavior, including migrations, invalidation, and rollback expectations.",
    ),
];

pub(super) fn review_checklist(files: &[ReviewFile], entries: &[ReviewEntryPoint]) -> Vec<String> {
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

    for (path_needles, symbol_needles, prompt) in DOMAIN_CHECKS {
        if path_matches_any(&paths, path_needles) || symbol_matches_any(entries, symbol_needles) {
            lines.push(prompt.to_string());
        }
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
            "- Use `l` on `Full diff against main` for an LLM pass over the whole entry-point tree, then select a risky file or entry and press `l` for focused follow-up."
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
