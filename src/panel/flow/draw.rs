//! Drawing the flow modal: the action menu with its preview, the confirmation
//! before an action runs, and the steps while it does.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::panel::scroll;
use crate::state::{AppState, FlowAction, FlowRun, SPINNER_FRAMES, clamp_index};
use crate::ui;

use super::{
    available_actions, flow_run, preview, selected_branch_line, selected_feature_branch, steps_for,
};

/// The modal's footprint. The preview needs room beside the list, so this is
/// wider than a menu alone would be — and every layout below is measured from
/// it, so it has one home.
pub(super) fn modal_area(area: Rect) -> Rect {
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
        let input_row = text.len() as u16;
        text.extend([
            Line::from(vec![
                Span::styled("new branch: ", Style::default().fg(Color::Yellow)),
                Span::raw(state.flow_text.as_str()),
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
        let column =
            ("new branch: ".len() + state.flow_text.before_cursor().chars().count()) as u16;
        if input_row < inner.height && column < inner.width {
            frame.set_cursor_position((inner.x + column, inner.y + input_row));
        }
        return Vec::new();
    }

    if let Some(action) = state.flow_confirm {
        let mut text = vec![
            Line::from(Span::styled(
                state.flow_action_label(action),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            warning_for(state, action),
            Line::from(""),
        ];
        if !state.flow_confirm_detail.is_empty() {
            text.extend(
                state
                    .flow_confirm_detail
                    .iter()
                    .map(|line| Line::from(line.clone())),
            );
            text.push(Line::from(""));
        }
        text.push(Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Green)),
            Span::raw(" run  "),
            Span::styled("n/Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel"),
        ]));
        let inner = ui::modal_frame(frame, modal, "Confirm Branch Action");
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
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
    let selected_idx = clamp_index(state.flow_list.idx, actions.len());
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
        state.flow_list.scroll,
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
pub(super) fn step_lines(run: &FlowRun) -> Vec<Line<'static>> {
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

/// The menu on the left and the preview on the right, or the whole width for
/// the menu when there is not enough of it to show both.
fn split_menu(frame: &mut Frame, area: Rect) -> ((Rect, Option<Rect>), Vec<Rect>) {
    let (panes, gaps) = menu_areas_with_gaps(area);
    ui::draw_dividers(frame, &gaps);
    (panes, gaps)
}

/// Where the two panes land, without drawing the divider between them.
pub(super) fn menu_areas(area: Rect) -> (Rect, Option<Rect>) {
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
pub(super) fn action_mark(action: FlowAction) -> (char, Color) {
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
        state.flow_list.scroll = 0;
        return;
    }

    let actions_len = available_actions(state).len();
    state.flow_list.scroll = scroll::selection_scroll_offset(
        clamp_index(state.flow_list.idx, actions_len),
        actions_len,
        actions_area(area).height as usize,
        state.flow_list.scroll,
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
pub(super) fn branch_name_char(typed: char) -> char {
    if typed == ' ' { '-' } else { typed }
}

/// Most branches a confirmation lists in one group before it says how many
/// more there are.
const CONFIRM_LIST_MAX: usize = 12;

/// What the confirmation names beyond the warning: for cleaning orphans, every
/// branch it would try to delete, split by whether git will let it go. Asked
/// of git once, when the prompt opens, rather than on every frame.
pub(super) fn confirm_detail(state: &AppState, action: FlowAction) -> Vec<String> {
    if action != FlowAction::CleanOrphans {
        return Vec::new();
    }
    let current = state.branch.clone().unwrap_or_default();
    let orphans = match crate::git::orphan_branches(&current) {
        Ok(orphans) => orphans,
        Err(err) => return vec![format!("could not list the branches: {err}")],
    };
    if orphans.is_empty() {
        return vec!["No local branch is without an upstream; nothing would be deleted.".into()];
    }
    let (deleted, kept): (Vec<_>, Vec<_>) =
        orphans.into_iter().partition(|branch| branch.unmerged == 0);
    let mut lines = Vec::new();
    if !deleted.is_empty() {
        lines.push(format!("Deleted ({}):", deleted.len()));
        push_capped(
            &mut lines,
            deleted.iter().map(|branch| match branch.unpushed {
                0 => format!("  {}", branch.name),
                n => format!("  {} \u{2014} {n} commit(s) on no remote", branch.name),
            }),
        );
    }
    if !kept.is_empty() {
        let target = if current.is_empty() { "HEAD" } else { &current };
        lines.push(format!("Kept, not merged into {target} ({}):", kept.len()));
        push_capped(
            &mut lines,
            kept.iter().map(|branch| {
                format!(
                    "  {} \u{2014} {} unmerged, {} on no remote",
                    branch.name, branch.unmerged, branch.unpushed
                )
            }),
        );
    }
    lines
}

fn push_capped(lines: &mut Vec<String>, items: impl ExactSizeIterator<Item = String>) {
    let total = items.len();
    lines.extend(items.take(CONFIRM_LIST_MAX));
    if total > CONFIRM_LIST_MAX {
        lines.push(format!("  \u{2026} and {} more", total - CONFIRM_LIST_MAX));
    }
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
            format!(
                "Hard resets {target} and force-pushes it (with lease), rewriting the remote {target} branch. Unique {target} history will be lost."
            ),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        FlowAction::DiscardCheckout => Line::from(Span::styled(
            "Hard resets current branch to its remote and deletes untracked files.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        FlowAction::CleanOrphans => Line::from(Span::styled(
            "Deletes merged local branches without upstream tracking. Unmerged ones and lg backups are kept.",
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
