//! Reading the change again once it has been fixed.

use super::*;
use crate::panel::guided::load::apply_loaded;

/// What `walk_with_findings` loads after a.rs's second hunk was edited, and
/// b.rs left alone.
fn after_a_fix(guided: &Guided) -> Loaded {
    let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n\
                @@ -1,3 +1,3 @@\n keep\n-old one\n+new one\n tail\n\
                @@ -20,3 +20,3 @@\n keep\n-old two\n+fixed two\n keep\n\
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

#[test]
fn a_fixed_file_is_read_again_with_its_last_read_and_the_rest_stay_put() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.current = 2;
    let b_key = guided.steps[1].key();
    let a_key = guided.steps[0].key();
    guided.side_scroll = 3;

    let loaded = after_a_fix(guided);
    apply_loaded(guided, loaded, false);

    assert_ne!(guided.steps[0].key(), a_key, "a.rs changed");
    assert_eq!(guided.steps[1].key(), b_key, "b.rs did not");
    assert!(
        guided.commentary.contains_key(&b_key),
        "b.rs keeps its read"
    );
    assert!(!guided.commentary.contains_key(&guided.steps[0].key()));
    assert!(
        guided.earlier["a.rs"].contains("-1 stands in for missing"),
        "the next read of a.rs hears what the last one found"
    );
    assert!(!guided.earlier.contains_key("b.rs"));
    assert_eq!(guided.side_scroll, 3, "b.rs on screen did not change");
}

#[test]
fn the_cursor_stays_put_when_the_file_on_screen_did_not_change() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.current = 2;
    guided.cursor = 1;

    let loaded = after_a_fix(guided);
    apply_loaded(guided, loaded, false);
    assert_eq!((guided.current, guided.cursor), (2, 1));

    guided.current = 1;
    guided.cursor = 0;
    let loaded = after_a_fix(guided);
    let mut again = loaded;
    again.steps[0]
        .lines
        .retain(|line| !line.text.contains("fixed"));
    apply_loaded(guided, again, false);
    assert_eq!(guided.cursor, guided.steps[0].first_changed());
}

#[test]
fn a_read_asked_for_while_one_runs_follows_it() {
    let mut state = walk_with_findings();
    let (_tx, rx) = std::sync::mpsc::channel();
    state.guided.as_mut().unwrap().loading = Some(rx);

    reload(&mut state);

    assert!(state.guided.as_ref().unwrap().reload_owed);
}
