//! Reading the change again lets go of the work on what it no longer has.

use super::*;
use crate::panel::guided::load::apply_loaded;

/// `walk_with_findings` once a.rs's second hunk was edited.
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

fn as_it_was(guided: &Guided, steps: Vec<GuidedFile>) -> Loaded {
    Loaded {
        source: guided.source.clone(),
        steps,
        overview: Overview {
            text: guided.overview.clone(),
            brief: String::new(),
        },
    }
}

/// A read of a.rs as it was, its questions out with Claude, and the final
/// review of the change as it was, all still running when a.rs is edited.
#[test]
fn work_on_a_version_the_change_no_longer_has_stops_holding_the_walk() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    let a_key = guided.steps[0].key();
    let before = guided.steps.clone();
    guided.commentary.remove(&a_key);
    let (_read_tx, read_rx) = std::sync::mpsc::channel();
    guided
        .commentary
        .insert(a_key.clone(), Commentary::default());
    guided.asks.push(Ask {
        key: a_key.clone(),
        rx: read_rx,
    });
    let (_final_tx, final_rx) = std::sync::mpsc::channel();
    guided.asks.push(Ask {
        key: guided.final_key(),
        rx: final_rx,
    });
    let (_check_tx, check_rx) = std::sync::mpsc::channel();
    guided.checks.push(Check {
        key: a_key.clone(),
        questions: Vec::new(),
        answer: check_rx,
    });
    guided.check_queue.push(a_key.clone());

    let loaded = after_a_fix(guided);
    apply_loaded(guided, loaded, false);

    assert!(!guided.working(), "nothing left to wait for");

    let loaded = as_it_was(guided, before);
    apply_loaded(guided, loaded, false);
    assert!(
        !guided.commentary.get(&a_key).is_some_and(|c| !c.done),
        "a.rs back as it was is read again, not left waiting on a read let go"
    );
}

/// Work on what the change still has carries on through the read.
#[test]
fn work_on_what_did_not_change_carries_on() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    let b_key = guided.steps[1].key();
    let (_tx, rx) = std::sync::mpsc::channel();
    guided.asks.push(Ask { key: b_key, rx });

    let loaded = after_a_fix(guided);
    apply_loaded(guided, loaded, false);

    assert!(guided.working());
}
