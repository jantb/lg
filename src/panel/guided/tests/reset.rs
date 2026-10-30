//! Resetting the review.

use super::*;

fn reviewed_with_a_note() -> AppState {
    let mut state = walk_with_findings();
    let guided = state.guided.as_mut().unwrap();
    guided.progress.notes.push(GuidedNote {
        path: "a.rs".into(),
        line: 2,
        side: Side::Right,
        body: "why?".into(),
        step: guided.steps[0].key(),
        quote: "+new one".into(),
    });
    guided.current = 2;
    state
}

fn was_reset(state: &AppState) -> bool {
    let guided = state.guided.as_ref().unwrap();
    guided.progress == GuidedProgress::default()
        && guided.commentary.is_empty()
        && guided.current == OVERVIEW
}

#[test]
fn x_resets_the_review_only_when_pressed_twice_in_a_row() {
    use ratatui::crossterm::event::KeyCode::Char;
    let mut state = reviewed_with_a_note();

    press(&mut state, Char('X'));
    assert!(!was_reset(&state), "the first press only asks");
    press(&mut state, Char('j'));
    press(&mut state, Char('X'));
    assert!(!was_reset(&state), "another key in between disarms it");

    press(&mut state, Char('X'));
    assert!(was_reset(&state));
}

#[test]
fn the_reset_button_under_the_steps_resets_on_a_second_click() {
    let mut state = reviewed_with_a_note();
    let (list, _) = panes(&state);
    let button = draw::reset_button_area(list);
    let click = |state: &mut AppState| {
        mouse(
            state,
            MouseEventKind::Down(MouseButton::Left),
            button.x + 2,
            button.y,
        )
    };

    click(&mut state);
    assert!(!was_reset(&state));
    assert!(screen(&state)[button.y as usize].contains("click again"));

    click(&mut state);
    assert!(was_reset(&state));
}
