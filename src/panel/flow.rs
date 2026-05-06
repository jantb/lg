use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, ListState, Paragraph},
};

use crate::{
    app,
    config::{BRANCH_DEV, BRANCH_MAIN, BRANCH_TEST},
    state::{AppState, FlowAction, Modal, SPINNER_FRAMES, clamp_index},
    ui,
};

fn merge_main_available(state: &AppState) -> bool {
    state
        .branch
        .as_deref()
        .is_some_and(|branch| !matches!(branch, BRANCH_MAIN | BRANCH_DEV | BRANCH_TEST))
}

fn available_actions(state: &AppState) -> Vec<FlowAction> {
    FlowAction::ALL
        .into_iter()
        .filter(|action| *action != FlowAction::MergeMain || merge_main_available(state))
        .collect()
}

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = (area.width * 7 / 10).clamp(58, 96).min(area.width);
    let h = 16.min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);

    if let Some(job) = &state.workflow_job {
        let spinner = SPINNER_FRAMES[job.spinner % SPINNER_FRAMES.len()];
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
            text.extend(workflow_lines(job));
        }
        frame.render_widget(Paragraph::new(text).block(ui::bordered("Flow")), modal);
        return;
    }

    if !state.flow_available() {
        let text = vec![
            Line::from(Span::styled(
                "Flow unavailable",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Create local develop and release/next branches to enable this workflow."),
            Line::from(""),
            Line::from(vec![
                Span::styled("Esc", Style::default().fg(Color::Gray)),
                Span::raw(" back"),
            ]),
        ];
        frame.render_widget(Paragraph::new(text).block(ui::bordered("Flow")), modal);
        return;
    }

    if let Some(action) = state.flow_input {
        let text = vec![
            Line::from(action.label()),
            Line::from(""),
            Line::from(vec![
                Span::styled("branch: ", Style::default().fg(Color::Yellow)),
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
        ];
        frame.render_widget(Paragraph::new(text).block(ui::bordered("Flow")), modal);
        return;
    }

    if let Some(action) = state.flow_confirm {
        let text = vec![
            Line::from(Span::styled(
                action.label(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            warning_for(action),
            Line::from(""),
            Line::from(vec![
                Span::styled("y", Style::default().fg(Color::Green)),
                Span::raw(" run  "),
                Span::styled("n/Esc", Style::default().fg(Color::Gray)),
                Span::raw(" cancel"),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(text).block(ui::bordered("Confirm Flow")),
            modal,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(0)])
        .split(modal);

    let diagram = vec![
        Line::from("origin/main  ----------------------------> production"),
        Line::from("     |"),
        Line::from("     +--> feature/*  --release-->  develop      -> dev"),
        Line::from("     |"),
        Line::from("     +--> feature/*  --release-->  release/next -> test"),
    ];
    frame.render_widget(
        Paragraph::new(diagram).block(ui::bordered("Flow Map")),
        chunks[0],
    );

    let actions = available_actions(state);
    let items: Vec<ListItem> = actions
        .iter()
        .map(|action| ListItem::new(Line::from(action.label())))
        .collect();
    let list = List::new(items)
        .block(ui::bordered("Actions"))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ")
        .scroll_padding(2);
    let mut list_state = ListState::default();
    if let Some(idx) = clamp_index(state.flow_idx, actions.len()) {
        list_state.select(Some(idx));
    }
    frame.render_stateful_widget(list, chunks[1], &mut list_state);
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    if state.workflow_job.is_some() {
        return Ok(());
    }

    if !state.flow_available() {
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
                state.flow_text.push(c);
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
            if action == FlowAction::NewFeature {
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

fn warning_for(action: FlowAction) -> Line<'static> {
    match action {
        FlowAction::ResetDev | FlowAction::ResetTest => Line::from(Span::styled(
            "Hard reset and force push. Unique target history will be lost.",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        FlowAction::CleanOrphans => Line::from(Span::styled(
            "Deletes local branches without upstream tracking.",
            Style::default().fg(Color::Red),
        )),
        FlowAction::ReleaseDev => Line::from(
            "Pushes current branch, syncs develop, merges origin/main, merges current, pushes HEAD to develop -> dev, then returns.",
        ),
        FlowAction::ReleaseTest => Line::from(
            "Pushes current branch, syncs release/next, merges origin/main, merges current, pushes HEAD to release/next -> test, then returns.",
        ),
        FlowAction::MergeMain => Line::from(
            "Stashes local changes, updates main from origin/main, returns, merges origin/main, pushes current, then restores.",
        ),
        FlowAction::NewFeature => Line::from(""),
    }
}

fn workflow_lines(job: &crate::state::WorkflowJob) -> Vec<Line<'static>> {
    let current = job.current_step.unwrap_or(0);
    let frame = match job.spinner % 4 {
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
