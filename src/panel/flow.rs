use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use crate::{
    app,
    config::is_protected_branch_name,
    state::{AppState, BranchView, FlowAction, FlowRun, Modal, SPINNER_FRAMES, clamp_index},
    ui,
};

use super::scroll;

mod preview;

fn merge_main_available(state: &AppState) -> bool {
    state.merge_main_available()
}

/// The actions the menu offers for this checkout. On a feature branch only
/// what concerns that branch is listed: merging the trunk in, releasing it,
/// and resetting the checkout. Resetting an environment is offered on that
/// environment's own branch; starting features and cleaning up are the
/// trunk's business and wait there.
pub fn available_actions(state: &AppState) -> Vec<FlowAction> {
    let feature = state.on_feature_branch();
    FlowAction::ALL
        .into_iter()
        .filter(|action| match action {
            FlowAction::MergeMain => merge_main_available(state),
            FlowAction::ReleaseDev | FlowAction::ReleaseTest => action
                .release_env()
                .is_some_and(|env| state.release_branch(env).is_some()),
            FlowAction::ResetDev | FlowAction::ResetTest => action
                .release_env()
                .and_then(|env| state.release_branch(env))
                .is_some_and(|deploy| state.branch.as_deref() == Some(deploy)),
            FlowAction::TransferDiff => !feature && selected_feature_branch(state).is_some(),
            FlowAction::DiscardCheckout => state.branch.is_some(),
            FlowAction::NewFeature | FlowAction::CleanOrphans => !feature,
        })
        .collect()
}

/// The modal's footprint. The preview needs room beside the list, so this is
/// wider than a menu alone would be — and every layout below is measured from
/// it, so it has one home.
fn modal_area(area: Rect) -> Rect {
    let w = (area.width * 9 / 10).clamp(58, 180).min(area.width);
    let h = (area.height * 4 / 5).clamp(14, 44).min(area.height);
    ui::centered(area, w, h)
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let modal = modal_area(area);
    let dividers = render_body(state, modal, frame);
    // The light is laid on last so every shape the modal takes gets the same
    // outline, whatever rooms it is divided into underneath.
    if state.decorative_animations {
        ui::animate_modal_border(state.animation_ms, modal, &dividers, frame);
    }
}

/// Draws the modal and hands back the dividers it split itself by.
fn render_body(state: &AppState, modal: Rect, frame: &mut Frame) -> Vec<Rect> {
    if let Some(job) = &state.workflow_job {
        return render_running(state, job, modal, frame);
    }

    if !state.branch_actions_available() {
        let text = vec![
            Line::from(Span::styled(
                "Branch actions unavailable",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Checkout or select a local branch first."),
            Line::from(""),
            Line::from(vec![
                Span::styled("Esc", Style::default().fg(Color::Gray)),
                Span::raw(" back"),
            ]),
        ];
        let inner = ui::modal_frame(frame, modal, "Flow");
        frame.render_widget(Paragraph::new(text), inner);
        return Vec::new();
    }

    if let Some(action) = state.flow_input {
        let mut text = vec![Line::from(state.flow_action_label(action)), Line::from("")];
        if action == FlowAction::TransferDiff
            && let Some(source) = selected_feature_branch(state)
        {
            text.push(Line::from(vec![
                Span::styled("source: ", Style::default().fg(Color::Yellow)),
                Span::raw(source),
            ]));
            text.push(Line::from(""));
        }
        text.extend([
            Line::from(vec![
                Span::styled("new branch: ", Style::default().fg(Color::Yellow)),
                Span::raw(state.flow_text.as_str()),
                Span::styled("\u{2588}", Style::default().fg(Color::Cyan)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Enter", Style::default().fg(Color::Green)),
                Span::raw(" create  "),
                Span::styled("Esc", Style::default().fg(Color::Gray)),
                Span::raw(" back"),
            ]),
            Line::from(""),
        ]);
        text.extend(branch_name_mascot());
        let inner = ui::modal_frame(frame, modal, "Branch Actions");
        frame.render_widget(Paragraph::new(text), inner);
        return Vec::new();
    }

    if let Some(action) = state.flow_confirm {
        let text = vec![
            Line::from(Span::styled(
                state.flow_action_label(action),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            warning_for(state, action),
            Line::from(""),
            Line::from(vec![
                Span::styled("y", Style::default().fg(Color::Green)),
                Span::raw(" run  "),
                Span::styled("n/Esc", Style::default().fg(Color::Gray)),
                Span::raw(" cancel"),
            ]),
        ];
        let inner = ui::modal_frame(frame, modal, "Confirm Branch Action");
        frame.render_widget(Paragraph::new(text), inner);
        return Vec::new();
    }

    let inner = ui::modal_frame(frame, modal, "Branch Actions");
    let (chunks, mut dividers) =
        ui::modal_rows(frame, inner, &[Constraint::Length(1), Constraint::Min(0)]);

    frame.render_widget(
        Paragraph::new(vec![Line::from(selected_branch_line(state))]),
        chunks[0],
    );

    let actions = available_actions(state);
    let selected_idx = clamp_index(state.flow_idx, actions.len());
    let ((list_area, preview_area), gaps) = split_menu(frame, chunks[1]);
    dividers.extend(gaps);

    let items: Vec<ListItem> = actions
        .iter()
        .map(|action| ListItem::new(action_line(state, *action)))
        .collect();
    let list = List::new(items)
        .highlight_style(crate::ui::palette::selection())
        .highlight_symbol("\u{203a} ");
    let offset = scroll::selection_scroll_offset(
        selected_idx,
        actions.len(),
        list_area.height as usize,
        state.flow_scroll_offset,
    );
    let mut list_state = scroll::list_state(selected_idx, offset);
    frame.render_stateful_widget(list, list_area, &mut list_state);
    ui::section_title(frame, list_area, "Actions");

    if let Some(area) = preview_area
        && let Some(action) = selected_idx.and_then(|idx| actions.get(idx).copied())
    {
        let run = flow_run(state, action);
        // The steps have first call on the pane's height, since they are read
        // rather than watched; the diagram stretches into whatever is left,
        // with a row spare for the caption wrapping.
        let steps = step_lines(&run);
        let reserved = u16::try_from(steps.len() + 1).unwrap_or(u16::MAX);
        let mut lines = preview::lines(
            state,
            &run,
            preview::Progress::Menu,
            if state.decorative_animations {
                state.animation_ms
            } else {
                0
            },
            area.width,
            area.height.saturating_sub(reserved),
        );
        lines.extend(steps);
        frame.render_widget(
            // The caption is a sentence and runs past the pane; the diagram
            // lines are built to fit, so only the sentence ever wraps.
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            area,
        );
        ui::section_title(frame, area, "What it does");
    }
    dividers
}

/// The modal while a branch action runs: the steps down the left with the one
/// in progress marked, and beside them the graph the menu drew, its marker moved
/// to wherever the run has got to. The list says what is being done; the picture
/// says what it is being done to, which is the part a list of eleven git
/// operations does not tell you.
fn render_running(
    state: &AppState,
    job: &crate::state::WorkflowJob,
    modal: Rect,
    frame: &mut Frame,
) -> Vec<Rect> {
    // Only a branch action has a graph, and only where it has room for one; the
    // jobs that have neither keep the whole modal for their steps.
    let graph = job
        .flow
        .as_ref()
        .filter(|run| preview::preview(state, run).is_some());
    let inner = ui::modal_frame(frame, modal, "Branch Actions");
    let (steps_area, graph_area, dividers) = match graph {
        Some(_) => {
            let ((steps, graph), gaps) = split_menu(frame, inner);
            (steps, graph, gaps)
        }
        None => (inner, None, Vec::new()),
    };

    let spinner = SPINNER_FRAMES[state.animation_tick % SPINNER_FRAMES.len()];
    let mut text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                spinner,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(job.label.clone(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(""),
    ];
    if job.steps.is_empty() {
        text.push(Line::from(Span::styled(
            "Git workflow is running",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
    } else {
        text.extend(workflow_lines(job, state.animation_tick));
    }
    frame.render_widget(Paragraph::new(text), steps_area);

    if let Some(area) = graph_area
        && let Some(run) = graph
    {
        let lines = preview::lines(
            state,
            run,
            // No progress reported yet means the first step is the one running,
            // which is what the step list beside it shows too.
            preview::Progress::Step(job.current_step.unwrap_or(0)),
            if state.decorative_animations {
                state.animation_ms
            } else {
                0
            },
            area.width,
            // Nothing shares this pane with the diagram, so all of it but a
            // row for the caption to wrap into is the diagram's.
            area.height.saturating_sub(1),
        );
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        ui::section_title(frame, area, "Where it is");
    }
    dividers
}

/// The steps the flow will run, in order, under the diagram. The picture shows
/// where commits end up; this shows what is actually done to get them there,
/// which is the part worth reading before a force-push.
fn step_lines(run: &FlowRun) -> Vec<Line<'static>> {
    let steps = steps_for(run);
    if steps.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "steps",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    lines.extend(steps.into_iter().enumerate().map(|(idx, step)| {
        Line::from(vec![
            Span::styled(
                format!("{:>2}. ", idx + 1),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(step, Style::default().fg(Color::Gray)),
        ])
    }));
    lines
}

/// What the highlighted action would run against, resolved the way running it
/// resolves it. The graph and the step list are both built from this, so the
/// menu cannot draw one branch and run against another.
fn flow_run(state: &AppState, action: FlowAction) -> FlowRun {
    let branch = match action {
        FlowAction::TransferDiff => selected_feature_branch(state),
        _ => state.branch.clone(),
    }
    .unwrap_or_default();
    FlowRun {
        action,
        branch,
        target: action
            .release_env()
            .and_then(|env| state.release_branch(env))
            .map(str::to_string),
        // Nothing has been typed yet; a run carries the name it was given.
        input: None,
    }
}

/// The steps a run performs, asked of the same code that narrates them while it
/// runs — so what the menu promises and what the progress list shows cannot
/// drift apart.
fn steps_for(run: &FlowRun) -> Vec<String> {
    app::workflow_steps(
        run.action,
        &run.branch,
        run.input.as_deref(),
        run.target.as_deref(),
    )
}

/// The menu on the left and the preview on the right, or the whole width for
/// the menu when there is not enough of it to show both.
fn split_menu(frame: &mut Frame, area: Rect) -> ((Rect, Option<Rect>), Vec<Rect>) {
    let (panes, gaps) = menu_areas_with_gaps(area);
    ui::draw_dividers(frame, &gaps);
    (panes, gaps)
}

/// Where the two panes land, without drawing the divider between them.
fn menu_areas(area: Rect) -> (Rect, Option<Rect>) {
    menu_areas_with_gaps(area).0
}

fn menu_areas_with_gaps(area: Rect) -> ((Rect, Option<Rect>), Vec<Rect>) {
    // The steps and the menu rows both fit in this, and the divider between
    // the two panes costs a column of its own.
    let list_width = 44u16;
    if area.width < list_width + 1 + preview::MIN_WIDTH || area.height < preview::MIN_HEIGHT {
        return ((area, None), Vec::new());
    }
    let (chunks, gaps) = ui::modal_column_areas(
        area,
        &[
            Constraint::Length(list_width),
            Constraint::Min(preview::MIN_WIDTH),
        ],
    );
    ((chunks[0], Some(chunks[1])), gaps)
}

/// A menu row: a glyph and a colour for what kind of change it is, so the
/// destructive ones are told apart from the everyday ones before they are read.
fn action_line(state: &AppState, action: FlowAction) -> Line<'static> {
    let (glyph, color) = action_mark(action);
    Line::from(vec![
        Span::styled(
            format!("{glyph} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(state.flow_action_label(action), Style::default().fg(color)),
    ])
}

/// What an action is marked with. Deploy branches keep the colours they have in
/// the deployment block; anything that throws work away is red, whatever it is
/// called.
fn action_mark(action: FlowAction) -> (char, Color) {
    match action {
        FlowAction::MergeMain => ('\u{2913}', Color::Magenta),
        FlowAction::ReleaseDev => ('\u{2912}', Color::Cyan),
        FlowAction::ReleaseTest => ('\u{2912}', Color::Yellow),
        FlowAction::ResetDev | FlowAction::ResetTest | FlowAction::DiscardCheckout => {
            ('\u{21ba}', Color::Red)
        }
        FlowAction::CleanOrphans => ('\u{2717}', Color::Red),
        FlowAction::NewFeature => ('\u{271a}', Color::Green),
        FlowAction::TransferDiff => ('\u{21dd}', Color::LightMagenta),
    }
}

pub(crate) fn sync_scroll_offset(state: &mut AppState, area: Rect) {
    if state.flow_confirm.is_some()
        || state.flow_input.is_some()
        || state.workflow_job.is_some()
        || !state.branch_actions_available()
    {
        state.flow_scroll_offset = 0;
        return;
    }

    let actions_len = available_actions(state).len();
    state.flow_scroll_offset = scroll::selection_scroll_offset(
        clamp_index(state.flow_idx, actions_len),
        actions_len,
        actions_area(area).height as usize,
        state.flow_scroll_offset,
    );
}

fn actions_area(area: Rect) -> Rect {
    let inner = ui::modal_inner(modal_area(area));
    let (chunks, _) = ui::modal_row_areas(inner, &[Constraint::Length(1), Constraint::Min(0)]);
    menu_areas(chunks[1]).0
}

/// The one line of typing that names a branch sits in a pane the size of the
/// screen. A small friend fills the rest of it and says the one thing worth
/// knowing about the prompt: a space is not a branch name character, so it is
/// typed as a dash.
fn branch_name_mascot() -> Vec<Line<'static>> {
    let fur = Style::default().fg(Color::Magenta);
    let whisper = Style::default().fg(Color::DarkGray);
    vec![
        Line::from(Span::styled("    /\\_/\\", fur)),
        Line::from(vec![
            Span::styled("   ( o.o )", fur),
            Span::styled("   \u{2500}\u{2500} spaces become dashes", whisper),
        ]),
        Line::from(Span::styled("    > ^ <", fur)),
        Line::from(Span::styled("   (\")_(\")", fur)),
    ]
}

/// What a keystroke puts into a branch name. Git will not take a space in a
/// ref, and the name typed with spaces is the name that was meant, so the
/// space is the dash the reader would have typed.
fn branch_name_char(typed: char) -> char {
    if typed == ' ' { '-' } else { typed }
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    if state.workflow_job.is_some() {
        return Ok(());
    }

    if !state.branch_actions_available() {
        state.flow_confirm = None;
        state.flow_input = None;
        state.flow_text.clear();
        if key.code == KeyCode::Esc {
            state.modal = Modal::None;
        }
        return Ok(());
    }

    if let Some(action) = state.flow_input {
        match key.code {
            KeyCode::Esc => {
                state.flow_input = None;
                state.flow_text.clear();
            }
            KeyCode::Enter => {
                let name = state.flow_text.trim().to_owned();
                if name.is_empty() {
                    state.set_status("branch name cannot be empty", true);
                } else {
                    state.flow_input = None;
                    state.flow_text.clear();
                    app::run_flow_action(state, action, Some(name));
                }
            }
            KeyCode::Backspace => {
                state.flow_text.pop();
            }
            KeyCode::Char(c) => {
                state.flow_text.push(branch_name_char(c));
            }
            _ => {}
        }
        return Ok(());
    }

    if let Some(action) = state.flow_confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                state.flow_confirm = None;
                app::run_flow_action(state, action, None);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.flow_confirm = None;
            }
            _ => {}
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let actions = available_actions(state);
            state.flow_idx = clamp_index(state.flow_idx, actions.len()).unwrap_or(0);
            state.flow_idx = state
                .flow_idx
                .saturating_add(1)
                .min(actions.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let actions = available_actions(state);
            state.flow_idx = clamp_index(state.flow_idx, actions.len()).unwrap_or(0);
            state.flow_idx = state.flow_idx.saturating_sub(1);
        }
        KeyCode::Enter => {
            let actions = available_actions(state);
            let Some(action) = actions
                .get(state.flow_idx.min(actions.len().saturating_sub(1)))
                .copied()
            else {
                return Ok(());
            };
            if action.needs_input() {
                state.flow_input = Some(action);
                state.flow_text.clear();
            } else if action.needs_confirmation() {
                state.flow_confirm = Some(action);
            } else {
                app::run_flow_action(state, action, None);
            }
        }
        _ => {}
    }
    Ok(())
}

fn warning_for(state: &AppState, action: FlowAction) -> Line<'static> {
    let configured_base = crate::preferences::base_branch();
    let target = action
        .release_env()
        .and_then(|env| state.release_branch(env))
        .unwrap_or_default()
        .to_string();
    match action {
        FlowAction::ResetDev | FlowAction::ResetTest => Line::from(Span::styled(
            "Hard reset and force push. Unique target history will be lost.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        FlowAction::DiscardCheckout => Line::from(Span::styled(
            "Hard resets current branch to its remote and deletes untracked files.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        FlowAction::CleanOrphans => Line::from(Span::styled(
            "Deletes local branches without upstream tracking.",
            Style::default().fg(Color::Red),
        )),
        FlowAction::ReleaseDev | FlowAction::ReleaseTest => Line::from(format!(
            "Pushes current branch, syncs {target}, merges origin/{configured_base}, merges current, pushes HEAD to {target}, then returns."
        )),
        FlowAction::MergeMain => Line::from(
            "Stashes local changes, updates main from origin/main, returns, merges origin/main, pushes current, then restores.",
        ),
        FlowAction::NewFeature => Line::from(""),
        FlowAction::TransferDiff => Line::from(""),
    }
}

fn selected_feature_branch(state: &AppState) -> Option<String> {
    if state.branch_view != BranchView::Local {
        return None;
    }
    let branch = state.selected_branch_ref()?;
    if is_protected_branch_name(branch) {
        None
    } else {
        Some(branch.to_string())
    }
}

fn selected_branch_line(state: &AppState) -> String {
    match state.selected_branch_ref() {
        Some(branch) if state.branch_view == BranchView::Local => format!("selected: {branch}"),
        Some(branch) => format!("selected remote: {branch}"),
        None => "no branch selected".to_string(),
    }
}

fn workflow_lines(job: &crate::state::WorkflowJob, tick: usize) -> Vec<Line<'static>> {
    let current = job.current_step.unwrap_or(0);
    let frame = match tick % 4 {
        0 => "|",
        1 => "/",
        2 => "-",
        _ => "\\",
    };
    job.steps
        .iter()
        .enumerate()
        .map(|(idx, step)| {
            if idx < current {
                Line::from(vec![
                    Span::styled("[x] ", Style::default().fg(Color::Green)),
                    Span::raw(step.clone()),
                ])
            } else if idx == current {
                Line::from(vec![
                    Span::styled(format!(">{frame}< "), Style::default().fg(Color::Cyan)),
                    Span::styled(
                        step.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled("[ ] ", Style::default().fg(Color::DarkGray)),
                    Span::styled(step.clone(), Style::default().fg(Color::DarkGray)),
                ])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Branch, ReleaseBranches};

    fn state() -> AppState {
        let mut state = AppState::new();
        state.branch = Some("feature/send-cv".into());
        state.branches = vec![Branch {
            name: "feature/send-cv".into(),
            is_current: true,
            upstream: Some("origin/feature/send-cv".into()),
            upstream_gone: false,
            ahead: 1,
            behind: 0,
            behind_main: 2,
            last_commit_unix: None,
        }];
        state.branches_idx = 0;
        state.release_branches = ReleaseBranches::new(Some("develop".into()), Some("test".into()));
        state
    }

    fn text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// On a feature branch the menu is about that branch: bring the trunk
    /// in, release it, or throw the checkout away. Trunk housekeeping is not
    /// offered until the trunk is checked out.
    #[test]
    fn a_feature_branch_offers_only_the_actions_about_itself() {
        let offered = available_actions(&state());
        assert!(offered.contains(&FlowAction::MergeMain), "{offered:?}");
        assert!(offered.contains(&FlowAction::ReleaseTest), "{offered:?}");
        assert!(
            offered.contains(&FlowAction::DiscardCheckout),
            "{offered:?}"
        );
        for general in [
            FlowAction::ResetTest,
            FlowAction::ResetDev,
            FlowAction::NewFeature,
            FlowAction::TransferDiff,
            FlowAction::CleanOrphans,
        ] {
            assert!(
                !offered.contains(&general),
                "{general:?} is trunk business: {offered:?}"
            );
        }
    }

    #[test]
    fn the_trunk_offers_the_general_housekeeping_but_no_reset() {
        let mut state = state();
        state.branch = Some("main".into());
        state.branches[0].name = "main".into();
        let offered = available_actions(&state);
        for general in [FlowAction::NewFeature, FlowAction::CleanOrphans] {
            assert!(
                offered.contains(&general),
                "{general:?} missing: {offered:?}"
            );
        }
        assert!(
            !offered.contains(&FlowAction::ResetTest) && !offered.contains(&FlowAction::ResetDev),
            "resetting an environment belongs on its branch: {offered:?}"
        );
    }

    /// Resetting an environment is done standing on it: the test branch
    /// offers to reset test and nothing else, so the menu cannot be used to
    /// wipe an environment nobody is looking at.
    #[test]
    fn an_environment_branch_offers_to_reset_only_itself() {
        let mut state = state();
        state.branch = Some("test".into());
        state.branches[0].name = "test".into();
        let offered = available_actions(&state);
        assert!(offered.contains(&FlowAction::ResetTest), "{offered:?}");
        assert!(!offered.contains(&FlowAction::ResetDev), "{offered:?}");
    }

    /// The steps shown are the steps run: both come from `workflow_steps`, so
    /// what the menu promises cannot drift from what the progress list narrates.
    #[test]
    fn every_step_the_flow_would_run_is_listed() {
        let state = state();
        for action in available_actions(&state) {
            let run = flow_run(&state, action);
            let shown = text(&step_lines(&run));
            for step in steps_for(&run) {
                assert!(
                    shown.contains(&step),
                    "{action:?} does not list {step:?}: {shown}"
                );
            }
        }
    }

    /// A release is named after where it lands, so the steps have to name the
    /// deploy branch rather than whatever branch happens to be checked out.
    #[test]
    fn the_steps_for_a_release_name_the_branch_it_lands_on() {
        let state = state();
        let steps = steps_for(&flow_run(&state, FlowAction::ReleaseTest)).join("\n");
        assert!(steps.contains("checkout test"), "{steps}");
        assert!(steps.contains("merge origin/feature/send-cv"), "{steps}");
    }

    /// Anything that throws work away is marked the same way, whatever it is
    /// called: the menu should not need reading twice to spot them.
    #[test]
    fn destructive_actions_are_all_marked_in_red() {
        for action in [
            FlowAction::ResetDev,
            FlowAction::ResetTest,
            FlowAction::DiscardCheckout,
            FlowAction::CleanOrphans,
        ] {
            assert_eq!(action_mark(action).1, Color::Red, "{action:?}");
        }
        for action in [FlowAction::MergeMain, FlowAction::NewFeature] {
            assert_ne!(action_mark(action).1, Color::Red, "{action:?}");
        }
    }
}
