//! A change too big for one read, read a part at a time.

use super::*;
use crate::panel::guided::context::{final_contexts, plan_contexts};

/// Four new files of about 45 KB each, so no part holds more than two. In
/// path order the directories pair up; the plan pairs them the other way.
fn walk_too_big_for_one_read() -> Guided {
    let file = |path: &str| {
        let lines: String = (0..1000)
            .map(|n| format!("+let value_{n} = compute_something_long({n});\n"))
            .collect();
        format!(
            "diff --git a/{path} b/{path}\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,1000 @@\n{lines}"
        )
    };
    let mut guided = Guided::new(Source::Codebase {
        branch: "guided-test-parts".into(),
    });
    guided.progress = GuidedProgress::default();
    guided.steps = crate::git::guided::parse_files(
        &["a/one.rs", "a/two.rs", "b/one.rs", "b/two.rs"]
            .map(file)
            .concat(),
    );
    guided.commentary.insert(
        OVERVIEW_KEY.to_string(),
        commentary(
            "Summary: Two features.\n\
             Concern: Ones\n\
             Files: a/one.rs, b/one.rs\n\
             Concern: Twos\n\
             Files: a/two.rs, b/two.rs",
        ),
    );
    guided
}

fn parts_with(parts: &[String], path: &str) -> Vec<usize> {
    let (plain, noted) = (format!("=== {path}\n"), format!("=== {path} ("));
    parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.contains(&plain) || part.contains(&noted))
        .map(|(at, _)| at)
        .collect()
}

#[test]
fn the_plan_reads_every_file_once_however_many_parts_it_takes() {
    let guided = walk_too_big_for_one_read();

    let parts = plan_contexts(&guided);

    assert!(parts.len() > 1);
    for path in ["a/one.rs", "a/two.rs", "b/one.rs", "b/two.rs"] {
        assert_eq!(parts_with(&parts, path).len(), 1, "{path}");
    }
}

#[test]
fn the_final_review_reads_each_concern_in_one_part() {
    let guided = walk_too_big_for_one_read();

    let parts = final_contexts(&guided);

    assert!(parts.len() > 1);
    assert_eq!(
        parts_with(&parts, "a/one.rs"),
        parts_with(&parts, "b/one.rs")
    );
    assert_eq!(
        parts_with(&parts, "a/two.rs"),
        parts_with(&parts, "b/two.rs")
    );
    assert_eq!(parts_with(&parts, "a/one.rs").len(), 1);
}
