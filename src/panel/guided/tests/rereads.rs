//! A file, or the whole change, read again after the reviewer changed it:
//! what was open is confirmed in its first words, and only what the change
//! touched is looked at anew.

use crate::git::guided::TickedFinding;
use crate::state::AppState;

use crate::panel::guided::load::apply_loaded;

use super::*;

const ISSUE: &str = "-1 stands in for missing and is used as an index.";
const NIT: &str = "tail could say what it ends.";

/// `walk_with_findings` once a guard line was added to a.rs above `new one`,
/// which moves every line under it down one; b.rs is left alone.
fn guard_added(guided: &Guided) -> Loaded {
    let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
                @@ -1,3 +1,4 @@\n keep\n+guard\n-old one\n+new one\n tail\n\
                @@ -20,3 +21,3 @@\n keep\n-old two\n+new two\n keep\n\
                diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
    Loaded {
        source: guided.source.clone(),
        steps: crate::git::guided::parse_files(diff),
        overview: Overview {
            text: guided.overview.clone(),
            brief: String::new(),
        },
    }
}

/// `walk_with_findings`, saved as `branch`, once the guard was added.
fn guarded(branch: &str) -> AppState {
    let mut state = walk_with_findings_saved_as(branch);
    let guided = state.guided.as_mut().unwrap();
    let loaded = guard_added(guided);
    apply_loaded(guided, loaded, false);
    state
}

fn tick(state: &mut AppState, line: usize, text: &str) {
    state
        .guided
        .as_mut()
        .unwrap()
        .progress
        .ticked
        .push(TickedFinding {
            path: "a.rs".into(),
            line,
            text: text.into(),
        });
}

fn shows(state: &AppState, text: &str) -> bool {
    shown_in(state, "a.rs")
        .iter()
        .any(|(shown, _)| shown == text)
}

/// The diff line of a.rs the finding `text` is shown under.
fn shown_under(state: &AppState, text: &str) -> Option<String> {
    let guided = state.guided.as_deref().unwrap();
    let open = guided
        .open_findings()
        .into_iter()
        .find(|open| open.finding.text == text)?;
    let at = open.at?;
    Some(guided.steps[open.step - 1].lines[at].text.clone())
}

#[test]
fn a_finding_the_reread_leaves_out_stays_open_under_the_line_it_moved_to() {
    let mut state = guarded("guided-test-reread-left-out");

    answer(&mut state, 1, "Summary: Adds a guard.\nVerdict: ok");

    assert_eq!(shown_under(&state, ISSUE).as_deref(), Some("+new one"));
    assert_eq!(shown_under(&state, NIT).as_deref(), Some(" tail"));
}

#[test]
fn a_standing_finding_keeps_its_first_words_however_the_reread_puts_them() {
    let mut state = guarded("guided-test-reread-reworded");

    answer(
        &mut state,
        1,
        "Summary: Adds a guard.\n\
         Stands: F1 L3\n\
         L3 issue: the sentinel -1 still indexes the table.\n\
         Stands: F2 L4\n\
         Verdict: issue",
    );

    assert!(shows(&state, ISSUE), "{:?}", shown_in(&state, "a.rs"));
    assert!(!shows(&state, "the sentinel -1 still indexes the table."));
}

#[test]
fn a_ticked_finding_is_still_ticked_after_a_reread() {
    let mut state = walk_with_findings_saved_as("guided-test-reread-ticked");
    tick(&mut state, 2, ISSUE);
    let guided = state.guided.as_mut().unwrap();
    let loaded = guard_added(guided);
    apply_loaded(guided, loaded, false);

    answer(
        &mut state,
        1,
        "Summary: Adds a guard.\nStands: F1 L4\nVerdict: nit",
    );

    assert!(
        shown_in(&state, "a.rs").contains(&(ISSUE.to_string(), true)),
        "{:?}",
        shown_in(&state, "a.rs")
    );
}

#[test]
fn a_nit_on_a_line_the_fix_did_not_touch_is_not_shown() {
    let mut state = guarded("guided-test-reread-untouched-nit");

    answer(
        &mut state,
        1,
        "Summary: Adds a guard.\nL22 nit: new two could say what is new.\nVerdict: nit",
    );

    assert!(!shows(&state, "new two could say what is new."));
}

#[test]
fn an_issue_on_a_line_the_fix_did_not_change_is_not_shown() {
    let mut state = guarded("guided-test-reread-untouched-issue");

    answer(
        &mut state,
        1,
        "Summary: Adds a guard.\nL22 issue: new two overflows on an empty name.\nVerdict: issue",
    );

    assert!(!shows(&state, "new two overflows on an empty name."));
}

#[test]
fn an_issue_on_a_line_the_fix_changed_is_shown() {
    let mut state = guarded("guided-test-reread-new-issue");

    answer(
        &mut state,
        1,
        "Summary: Adds a guard.\n\
         L2 issue: an empty name returns before the cache is filled, so the next lookup misses.\n\
         Verdict: issue",
    );

    assert_eq!(
        shown_under(
            &state,
            "an empty name returns before the cache is filled, so the next lookup misses."
        )
        .as_deref(),
        Some("+guard")
    );
}

#[test]
fn a_fixed_finding_is_told_to_the_chat_in_its_first_words_and_no_longer_open() {
    let mut state = guarded("guided-test-reread-fixed");

    answer(
        &mut state,
        1,
        "Summary: The guard keeps -1 out.\nFixed: F1\nStands: F2 L4\nVerdict: nit",
    );

    let brief = state.guided.as_ref().unwrap().chat_brief("a.rs");
    assert!(brief.contains(&format!("fixed: {ISSUE}")), "{brief}");
    assert!(!shows(&state, ISSUE));
}

#[test]
fn a_finding_the_reviewer_dismissed_is_not_shown_when_a_fresh_read_raises_it_again() {
    let mut state = walk_with_findings_saved_as("guided-test-reread-dismissed");
    tick(&mut state, 3, NIT);

    answer(
        &mut state,
        1,
        &format!("Summary: Renames a line.\nL2 issue: {ISSUE}\nL3 nit: {NIT}\nVerdict: issue"),
    );

    assert!(shows(&state, ISSUE));
    assert!(!shows(&state, NIT), "{:?}", shown_in(&state, "a.rs"));
}

#[test]
fn a_fourth_nit_in_a_file_is_not_shown() {
    let mut state = walk_with_findings_saved_as("guided-test-nit-budget");

    answer(
        &mut state,
        1,
        "Summary: Renames a line.\n\
         L1 nit: keep is vague.\n\
         L2 nit: new one is vague.\n\
         L3 nit: tail is vague.\n\
         L21 nit: new two is vague.\n\
         Verdict: nit",
    );

    assert!(shows(&state, "tail is vague."));
    assert!(!shows(&state, "new two is vague."));
}

/// `walk_with_findings` with the whole change read, once the guard was added.
fn whole_guarded(branch: &str) -> AppState {
    let mut state = walk_with_findings_saved_as(branch);
    let guided = state.guided.as_mut().unwrap();
    let key = guided.final_key();
    guided.commentary.insert(
        key,
        commentary(
            "Summary: Two renames that agree.\n\
             a.rs:L21 nit: new two repeats new one.\n\
             b.rs:L1 issue: callers of x are not updated.\n\
             Verdict: issue",
        ),
    );
    let loaded = guard_added(guided);
    apply_loaded(guided, loaded, false);
    state
}

fn cited(state: &AppState) -> Vec<(String, String)> {
    state
        .guided
        .as_ref()
        .unwrap()
        .final_cited()
        .into_iter()
        .map(|(path, finding)| (path, finding.text))
        .collect()
}

#[test]
fn a_reread_of_the_whole_change_keeps_what_stands_in_its_first_words() {
    let mut state = whole_guarded("guided-test-whole-stands");
    let last = state.guided.as_ref().unwrap().final_step();

    answer(
        &mut state,
        last,
        "Summary: Still two renames.\n\
         Stands: F1 a.rs:L22\n\
         a.rs:L22 nit: new two still copies new one.\n\
         Fixed: F2\n\
         Verdict: nit",
    );

    let cited = cited(&state);
    assert!(
        cited.contains(&("a.rs".into(), "new two repeats new one.".into())),
        "{cited:?}"
    );
    assert!(
        !cited
            .iter()
            .any(|(_, text)| text == "new two still copies new one."),
        "{cited:?}"
    );
    assert!(
        !cited
            .iter()
            .any(|(_, text)| text == "callers of x are not updated."),
        "{cited:?}"
    );
}

#[test]
fn a_reread_of_the_whole_change_shows_a_new_issue_only_in_a_file_changed_since() {
    let mut state = whole_guarded("guided-test-whole-new");
    let last = state.guided.as_ref().unwrap().final_step();

    answer(
        &mut state,
        last,
        "Summary: A guard joins the renames.\n\
         Stands: F1 a.rs:L22\n\
         Stands: F2 b.rs:L1\n\
         a.rs:L2 issue: the guard skips the rename b.rs relies on.\n\
         b.rs:L1 issue: y is never read.\n\
         Verdict: issue",
    );

    let cited = cited(&state);
    assert!(
        cited.contains(&(
            "a.rs".into(),
            "the guard skips the rename b.rs relies on.".into()
        )),
        "{cited:?}"
    );
    assert!(
        !cited.iter().any(|(_, text)| text == "y is never read."),
        "{cited:?}"
    );
}

#[test]
fn a_sixth_nit_across_the_whole_change_is_not_shown() {
    let mut state = walk_with_findings_saved_as("guided-test-whole-nit-budget");
    let last = state.guided.as_ref().unwrap().final_step();

    answer(
        &mut state,
        last,
        "Summary: Two renames.\n\
         a.rs:L1 nit: one.\n\
         a.rs:L2 nit: two.\n\
         a.rs:L3 nit: three.\n\
         a.rs:L20 nit: four.\n\
         a.rs:L21 nit: five.\n\
         b.rs:L1 nit: six.\n\
         Verdict: nit",
    );

    let cited = cited(&state);
    assert!(
        cited.contains(&("a.rs".into(), "five.".into())),
        "{cited:?}"
    );
    assert!(!cited.iter().any(|(_, text)| text == "six."), "{cited:?}");
}
