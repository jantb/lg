//! The final review of the whole change.

use crate::git::guided::TickedFinding;
use crate::state::{AppState, PendingAction};
use ratatui::crossterm::event::{MouseButton, MouseEventKind};

use crate::panel::guided::context::final_context;

use super::*;

#[test]
fn past_the_last_file_is_the_final_review_and_past_that_every_note() {
    use ratatui::crossterm::event::KeyCode::Right;
    let mut state = walk_with_findings();
    state.guided.as_mut().unwrap().current = 2;

    press(&mut state, Right);
    let guided = state.guided.as_ref().unwrap();
    assert!(guided.on_final());
    assert!(guided.is_reviewed(&guided.steps[1]));

    press(&mut state, Right);
    assert!(matches!(
        state.guided.as_ref().unwrap().mode,
        Mode::Summary { .. }
    ));
}

#[test]
fn the_final_review_is_shown_the_whole_change_and_what_is_still_open() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.progress.ticked.push(TickedFinding {
        path: "a.rs".into(),
        line: 3,
        text: "tail could say what it ends.".into(),
    });

    let context = final_context(guided);

    for line in ["+new one", "+new two", "+y"] {
        assert!(context.contains(line), "{line} missing from\n{context}");
    }
    assert!(context.contains("-1 stands in for missing"), "{context}");
    assert!(context.contains("y says less than x"), "{context}");
    assert!(!context.contains("tail could say"), "{context}");
}

/// The final step shows every file's diff with the findings across files
/// under their lines, and a click on one ticks it off.
#[test]
fn the_final_step_pins_findings_across_files_into_the_whole_change() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.current = guided.final_step();
    let key = guided.final_key();
    guided.commentary.insert(
        key,
        commentary(
            "Summary: Two renames that agree.\n\
             b.rs:L1 issue: callers of x are not updated.\n\
             Verdict: issue",
        ),
    );
    assert_eq!(
        state.guided.as_ref().unwrap().final_verdict(),
        Some(Verdict::Issue)
    );

    let rows = screen(&state);
    let row_of = |text: &str| rows.iter().position(|row| row.contains(text));
    let (_, side) = panes(&state);
    let guided = state.guided.as_deref().unwrap();
    let diff = draw::diff_area(guided, side, state.diff_view_mode).unwrap();
    assert!(row_of("new one").is_some() && row_of("new two").is_some());
    let pinned = (diff.y..diff.y + diff.height)
        .find(|&y| rows[y as usize].contains("callers of x are not updated"))
        .expect("the finding is pinned in the whole change");
    assert!(pinned as usize > row_of("b.rs").unwrap());

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        100,
        pinned,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        100,
        pinned,
    );

    assert_eq!(
        state.guided.as_ref().unwrap().final_verdict(),
        Some(Verdict::Ok)
    );
}

/// `walk_with_findings` on its final step, whose review cites a.rs and
/// b.rs. Its progress is saved under `branch`, so what one test ticks off
/// is not ticked in another.
fn on_final_with_findings(branch: &str) -> AppState {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.source = Source::Branch {
        branch: branch.into(),
        base_ref: "main".into(),
    };
    guided.progress = GuidedProgress {
        reviewed: guided.progress.reviewed.clone(),
        ..GuidedProgress::default()
    };
    guided.current = guided.final_step();
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
    state
}

#[test]
fn a_finding_of_the_final_review_is_picked_and_opened_on_its_line() {
    use ratatui::crossterm::event::KeyCode::{Char, Down, Enter};
    let mut state = on_final_with_findings("guided-test-final-open");

    press(&mut state, Down);
    press(&mut state, Enter);

    assert_eq!(cursor_line(&state), (2, "+y".into()));
    assert!(
        screen(&state)
            .iter()
            .any(|row| row.contains("callers of x are not updated")),
        "the finding shows under its line in the file"
    );
    press(&mut state, Char('t'));
    press(&mut state, Char('w'));
    assert_eq!(
        state.guided.as_ref().unwrap().final_verdict(),
        Some(Verdict::Nit)
    );
}

#[test]
fn the_picked_finding_is_ticked_off_on_the_final_review() {
    use ratatui::crossterm::event::KeyCode::{Char, Down};
    let mut state = on_final_with_findings("guided-test-final-tick");

    press(&mut state, Down);
    press(&mut state, Char('t'));

    assert_eq!(
        state.guided.as_ref().unwrap().final_verdict(),
        Some(Verdict::Nit)
    );
}

#[test]
fn the_chat_opens_on_the_final_review_with_what_is_open_across_the_change() {
    let mut state = on_final_with_findings("guided-test-final-chat");
    state
        .guided
        .as_mut()
        .unwrap()
        .progress
        .ticked
        .push(TickedFinding {
            path: "a.rs".into(),
            line: 21,
            text: "new two repeats new one.".into(),
        });

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('C'));

    assert_eq!(state.pending_action, Some(PendingAction::GuidedChat));
    let guided = state.guided.as_ref().unwrap();
    let subject = guided.chat_subject().unwrap();
    let brief = guided.chat_brief(&subject);
    assert!(
        brief.contains("b.rs:L1 issue: callers of x are not updated"),
        "{brief}"
    );
    assert!(!brief.contains("repeats new one"), "{brief}");
}

#[test]
fn a_fix_on_the_final_review_starts_from_the_picked_finding() {
    use ratatui::crossterm::event::KeyCode::{Char, Down};
    let mut state = on_final_with_findings("guided-test-final-fix");

    press(&mut state, Down);
    press(&mut state, Char('f'));

    let Mode::Fix { text, .. } = &state.guided.as_ref().unwrap().mode else {
        panic!("f asks what to change");
    };
    assert!(text.contains("callers of x are not updated"), "{}", &**text);
}

#[test]
fn the_chat_about_a_file_hears_what_the_final_review_found_in_it() {
    let state = on_final_with_findings("guided-test-final-brief");

    let brief = state.guided.as_ref().unwrap().chat_brief("b.rs");

    assert!(brief.contains("callers of x are not updated"), "{brief}");
}

/// Going to the final review shows it working from the first moment: in
/// line behind the reads already running, then reading the change, then
/// how many findings it has written.
#[test]
fn the_final_review_shows_how_far_it_has_got() {
    let mut state = walk_with_plan();
    state.ai_assist = true;
    let guided = state.guided.as_mut().unwrap();
    guided.current = guided.final_step();
    let (_busy, rx) = std::sync::mpsc::channel();
    guided.asks.push(Ask {
        target: Target::Commentary(guided.steps[0].key()),
        rx,
    });
    assert_eq!(guided.final_progress(), Some(Progress::Queued { ahead: 1 }));
    assert!(
        screen(&state)
            .iter()
            .any(|row| row.contains("queued behind 1 read"))
    );

    let guided = state.guided.as_mut().unwrap();
    let key = guided.final_key();
    guided.commentary.insert(
        key.clone(),
        Commentary {
            started: Some(std::time::Instant::now()),
            prompt_bytes: 50_000,
            ..Default::default()
        },
    );
    assert!(matches!(
        guided.final_progress(),
        Some(Progress::Reading { .. })
    ));
    assert!(
        screen(&state)
            .iter()
            .any(|row| row.contains("reading the whole change"))
    );

    let guided = state.guided.as_mut().unwrap();
    guided.commentary.get_mut(&key).unwrap().text =
        "Summary: Two parts.\nsrc/cache.rs:L1 issue: evict is never called.\n".into();
    assert_eq!(
        guided.final_progress(),
        Some(Progress::Finding { so_far: 1 })
    );
    assert!(
        screen(&state)
            .iter()
            .any(|row| row.contains("1 finding(s) so far"))
    );

    let guided = state.guided.as_mut().unwrap();
    guided.commentary.get_mut(&key).unwrap().done = true;
    assert_eq!(guided.final_progress(), None);
}
