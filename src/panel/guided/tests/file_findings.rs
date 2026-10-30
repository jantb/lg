//! A file's findings: under their lines, walked by the arrows, ticked off.

use crate::git::guided::TickedFinding;
use crate::state::AppState;

use crate::panel::guided::context::file_context;

use super::*;

#[test]
fn the_verdict_is_read_off_the_last_line_once_the_answer_is_done() {
    assert_eq!(
        commentary("Adds a cache.\nVerdict: issue").verdict(),
        Some(Verdict::Issue)
    );
    assert_eq!(
        commentary("Fine.\n\nVerdict: **ok**\n").verdict(),
        Some(Verdict::Ok)
    );
    assert_eq!(commentary("No verdict here").verdict(), None);

    let mut unfinished = commentary("Verdict: ok");
    unfinished.done = false;
    assert_eq!(unfinished.verdict(), None);
}

#[test]
fn the_arrows_walk_the_flagged_lines_across_files() {
    use ratatui::crossterm::event::KeyCode::{Char, Down, Esc, Up};
    let mut state = walk_with_findings();

    press(&mut state, Down);
    assert_eq!(cursor_line(&state), (1, "+new one".into()));

    press(&mut state, Char('c'));
    let Mode::Note { text, .. } = &state.guided.as_ref().unwrap().mode else {
        panic!("c opens a note");
    };
    assert!(text.contains("-1 stands in for missing"), "{}", &**text);
    press(&mut state, Esc);

    press(&mut state, Down);
    assert_eq!(cursor_line(&state), (1, " tail".into()));
    press(&mut state, Down);
    assert_eq!(cursor_line(&state), (2, "+y".into()));
    press(&mut state, Up);
    assert_eq!(cursor_line(&state), (1, " tail".into()));
}

#[test]
fn down_moves_a_line_while_the_model_is_still_reading() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    let key = guided.steps[0].key();
    guided.commentary.get_mut(&key).unwrap().done = false;
    let start = guided.cursor;

    press(&mut state, ratatui::crossterm::event::KeyCode::Down);

    let guided = state.guided.as_ref().unwrap();
    assert_eq!((guided.current, guided.cursor), (1, start + 1));
}

#[test]
fn ticking_findings_off_settles_the_file() {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    let first = guided.steps[0].clone();
    assert_eq!(guided.verdict(&first), Some(Verdict::Issue));
    let open = |guided: &Guided| {
        (
            guided.open_in_file("a.rs", findings::Severity::Issue),
            guided.open_in_file("a.rs", findings::Severity::Nit),
        )
    };
    assert_eq!(open(guided), (1, 1));

    guided.progress.ticked.push(TickedFinding {
        path: "a.rs".into(),
        line: 2,
        text: "-1 stands in for missing and is used as an index.".into(),
    });
    assert_eq!(guided.verdict(&first), Some(Verdict::Nit));
    assert_eq!(open(guided), (0, 1));

    guided.progress.ticked.push(TickedFinding {
        path: "a.rs".into(),
        line: 3,
        text: "tail could say what it ends.".into(),
    });
    assert_eq!(guided.verdict(&first), Some(Verdict::Ok));
    assert_eq!(open(guided), (0, 0));

    // An open question is not a defect: the hunk stays ok.
    let key = first.key();
    guided.commentary.insert(
        key,
        commentary("L2 question: can the index be -1 here?\nVerdict: ok"),
    );
    assert_eq!(guided.verdict(&first), Some(Verdict::Ok));
    assert_eq!(guided.open_in_file("a.rs", findings::Severity::Question), 1);
}

#[test]
fn r_reviews_the_file_again_with_what_was_said_before() {
    let mut state = walk_with_findings();
    state.ai_assist = true;
    // Already loading, so the test starts no read of its own.
    let (_reading, rx) = std::sync::mpsc::channel();
    state.guided.as_mut().unwrap().loading = Some(rx);

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('r'));

    let guided = state.guided.as_ref().unwrap();
    assert!(!guided.commentary.contains_key(&guided.steps[0].key()));
    assert!(file_context(guided, 1).contains("-1 stands in for missing"));

    let guided = state.guided.as_mut().unwrap();
    assert!(!guided.has_read("a.rs"));
    guided.loading = None;
    let key = guided.steps[0].key();
    guided.commentary.insert(
        key,
        commentary("Summary: Same.\nFixed: the index sentinel.\nVerdict: ok"),
    );
    assert!(guided.has_read("a.rs"));
    assert!(
        guided.has_read("gone.rs"),
        "a file the change no longer has"
    );
    assert!(
        guided
            .chat_brief("a.rs")
            .contains("fixed: the index sentinel.")
    );
}

#[test]
fn the_file_follows_the_diff_layout_with_findings_under_their_line() {
    let mut state = walk_with_findings();
    let hunk = |state: &AppState| {
        let (_, side) = panes(state);
        let guided = state.guided.as_deref().unwrap();
        let (parts, _) =
            draw::step_areas(guided, guided.step().unwrap(), side, state.diff_view_mode);
        parts[0].y..parts[0].y + parts[0].height
    };
    let row_of =
        |rows: &[String], text: &str| rows.iter().position(|row| row.contains(text)).unwrap();

    assert_eq!(state.diff_view_mode, DiffViewMode::SideBySide);
    let rows = screen(&state);
    assert_eq!(row_of(&rows, "old one"), row_of(&rows, "new one"));
    let finding = row_of(&rows, "-1 stands in for missing");
    assert!(finding > row_of(&rows, "new one"));
    assert!(hunk(&state).contains(&(finding as u16)));
    let column = |row: &str, text: &str| row[..row.find(text).unwrap()].chars().count();
    let old = row_of(&rows, "old one");
    assert!(
        column(&rows[finding], "-1 stands in") > column(&rows[old], "old one") + "old one".len(),
        "side by side, the finding sits right of the old code, under the new"
    );

    press(&mut state, ratatui::crossterm::event::KeyCode::Char('v'));

    assert_eq!(state.diff_view_mode, DiffViewMode::Unified);
    let rows = screen(&state);
    assert!(row_of(&rows, "new one") > row_of(&rows, "old one"));
    let finding = row_of(&rows, "-1 stands in for missing");
    assert!(finding > row_of(&rows, "new one"));
    assert!(hunk(&state).contains(&(finding as u16)));
}
