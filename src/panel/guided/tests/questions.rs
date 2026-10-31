//! Questions a read raises, settled in the code before the reviewer sees them.

use super::*;

/// Step one of `walk_with_findings` with `questions` out with Claude, and
/// the end of the channel its answer comes in on.
fn checking(
    guided: &mut Guided,
    questions: &[&str],
) -> std::sync::mpsc::Sender<Result<String, String>> {
    let key = guided.steps[0].key();
    guided.commentary.insert(
        key.clone(),
        Commentary {
            text: "Summary: Renames a line.\nVerdict: ok".into(),
            checking: true,
            ..Default::default()
        },
    );
    let (tx, rx) = std::sync::mpsc::channel();
    guided.checks.push(Check {
        key,
        questions: questions.iter().map(|q| q.to_string()).collect(),
        answer: rx,
    });
    tx
}

fn open_questions(guided: &Guided) -> Vec<String> {
    let commentary = &guided.commentary[&guided.steps[0].key()];
    findings::read(&commentary.text)
        .findings
        .into_iter()
        .filter(|finding| finding.severity == findings::Severity::Question)
        .map(|finding| finding.text)
        .collect()
}

#[test]
fn only_the_questions_the_code_cannot_settle_are_left_for_the_reviewer() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    let tx = checking(
        guided,
        &[
            "L2 question: is old one read anywhere else?",
            "L3 question: should the rename change the output?",
        ],
    );
    assert!(!guided.has_read("a.rs"), "its questions are still out");

    tx.send(Ok(
        "L3 question: should the rename change the output?".into()
    ))
    .unwrap();
    drain_checks(guided, &mut Vec::new());

    assert!(guided.has_read("a.rs"));
    assert_eq!(
        open_questions(guided),
        vec!["should the rename change the output?"]
    );
}

#[test]
fn a_check_that_fails_leaves_the_questions_as_they_were_asked() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    let tx = checking(guided, &["L2 question: is old one read anywhere else?"]);

    tx.send(Err("claude was not found on PATH".into())).unwrap();
    let mut notices = Vec::new();
    drain_checks(guided, &mut notices);

    assert!(guided.has_read("a.rs"));
    assert_eq!(
        open_questions(guided),
        vec!["is old one read anywhere else?"]
    );
    assert!(notices.iter().any(|(_, is_error)| *is_error));
}
