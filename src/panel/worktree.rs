use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::{
    state::{AppState, Modal, PendingAction, WorktreeField},
    ui,
};

const LABEL_WIDTH: u16 = 8;
const FIRST_FIELD_ROW: u16 = 3;

pub fn render(state: &AppState, area: Rect, frame: &mut Frame) {
    let w = 76.min(area.width);
    let h = 11.min(area.height);
    let modal = ui::centered(area, w, h);
    frame.render_widget(Clear, modal);
    if modal.width < 28 || modal.height < 9 {
        frame.render_widget(
            Paragraph::new("Terminal too small for the worktree form")
                .block(ui::bordered("New Worktree")),
            modal,
        );
        return;
    }

    let value_width = modal
        .width
        .saturating_sub(2)
        .saturating_sub(LABEL_WIDTH)
        .max(1) as usize;
    let reuses_branch = branch_exists(state);
    let lines = vec![
        Line::from(vec![
            Span::styled("next to  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                truncate_tail(&state.worktree_repo_dir, value_width),
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::from(""),
        field_line(
            "Branch",
            &state.worktree_branch_input,
            state.worktree_field == WorktreeField::Branch,
            value_width,
            if reuses_branch {
                Some("use existing branch")
            } else {
                Some("create a new branch")
            },
        ),
        field_line(
            "Base",
            &state.worktree_base_input,
            state.worktree_field == WorktreeField::Base,
            value_width,
            if reuses_branch {
                Some("unused \u{2014} branch exists")
            } else {
                Some("new branch starts here")
            },
        ),
        field_line(
            "Path",
            &state.worktree_path_input,
            state.worktree_field == WorktreeField::Path,
            value_width,
            Some(if state.worktree_path_edited {
                "worktree directory"
            } else {
                "directory follows branch name"
            }),
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" field    "),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" create    "),
            Span::styled("Esc", Style::default().fg(Color::Gray)),
            Span::raw(" cancel"),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(ui::bordered("New Worktree")),
        modal,
    );
    animate_border(state.animation_ms, modal, frame);
    if let Some((x, y)) = active_field_cursor(state, modal) {
        frame.set_cursor_position(Position::new(x, y));
    }
}

/// Two soft light trails orbit the frame. Only tint border cells so the form
/// and cursor remain still; continuous colour interpolation avoids cell jumps.
fn animate_border(clock_ms: u64, area: Rect, frame: &mut Frame) {
    let horizontal = area.width - 1;
    let vertical = area.height - 1;
    let perimeter = 2 * (u32::from(horizontal) + u32::from(vertical));
    let phase = (clock_ms % 4_800) as f64 / 4_800.0;
    for step in 0..perimeter {
        let position = f64::from(step) / f64::from(perimeter);
        let glow = |offset: f64| {
            let distance = (position - phase - offset + 0.5).rem_euclid(1.0) - 0.5;
            (-((distance / 0.085).powi(2))).exp()
        };
        let cyan = glow(0.0);
        let violet = glow(0.5);
        let color = Color::Rgb(
            (85.0 + 35.0 * cyan + 150.0 * violet).min(255.0) as u8,
            (75.0 + 170.0 * cyan + 45.0 * violet).min(255.0) as u8,
            (135.0 + 115.0 * cyan + 120.0 * violet).min(255.0) as u8,
        );
        let w = u32::from(horizontal);
        let h = u32::from(vertical);
        let (x, y) = if step < w {
            (step, 0)
        } else if step < w + h {
            (w, step - w)
        } else if step < 2 * w + h {
            (2 * w + h - step, h)
        } else {
            (0, perimeter - step)
        };
        // Leave the title at its steady, readable accent colour.
        if y == 0 && (1..=12).contains(&x) {
            continue;
        }
        frame.buffer_mut()[(area.x + x as u16, area.y + y as u16)].set_fg(color);
    }
}

/// Whether the typed branch already exists locally, which decides whether the
/// base ref is used at all.
fn branch_exists(state: &AppState) -> bool {
    let branch = state.worktree_branch_input.trim();
    !branch.is_empty() && state.branches.iter().any(|known| known.name == branch)
}

fn field_line(
    label: &'static str,
    value: &str,
    selected: bool,
    value_width: usize,
    hint: Option<&str>,
) -> Line<'static> {
    let label_style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let value_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let mut spans = vec![
        Span::styled(format!("{label:<8}"), label_style),
        Span::styled(truncate_tail(value, value_width), value_style),
    ];
    // The hint only fits once the value leaves room for it.
    if let Some(hint) = hint {
        let used = value.chars().count();
        if value_width.saturating_sub(used) > hint.chars().count() + 3 {
            spans.push(Span::styled(
                format!("   {hint}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    Line::from(spans)
}

/// Paths grow at the end, so keep the tail visible when one is too long.
fn truncate_tail(value: &str, width: usize) -> String {
    let len = value.chars().count();
    if len <= width {
        return value.to_string();
    }
    let skip = len.saturating_sub(width.saturating_sub(1));
    format!("\u{2026}{}", value.chars().skip(skip).collect::<String>())
}

fn active_field_cursor(state: &AppState, modal: Rect) -> Option<(u16, u16)> {
    if modal.width <= 2 || modal.height <= 2 {
        return None;
    }
    let (offset, value) = match state.worktree_field {
        WorktreeField::Branch => (0, &state.worktree_branch_input),
        WorktreeField::Base => (1, &state.worktree_base_input),
        WorktreeField::Path => (2, &state.worktree_path_input),
    };
    let content_width = modal.width.saturating_sub(2);
    let value_width = content_width.saturating_sub(LABEL_WIDTH + 1) as usize;
    let cursor_x = LABEL_WIDTH.saturating_add(value.chars().count().min(value_width) as u16);
    Some((modal.x + 1 + cursor_x, modal.y + FIRST_FIELD_ROW + offset))
}

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Tab | KeyCode::Down => {
            state.worktree_field = state.worktree_field.next(true);
        }
        KeyCode::BackTab | KeyCode::Up => {
            state.worktree_field = state.worktree_field.next(false);
        }
        KeyCode::Enter => submit(state),
        KeyCode::Backspace if !ctrl => {
            match state.worktree_field {
                WorktreeField::Branch => {
                    state.worktree_branch_input.pop();
                    state.sync_worktree_path();
                }
                WorktreeField::Base => {
                    state.worktree_base_input.pop();
                }
                WorktreeField::Path => {
                    state.worktree_path_input.pop();
                    // An emptied path goes back to following the branch.
                    state.worktree_path_edited = !state.worktree_path_input.is_empty();
                    state.sync_worktree_path();
                }
            }
        }
        KeyCode::Char(c) if !ctrl => match state.worktree_field {
            WorktreeField::Branch => {
                state.worktree_branch_input.push(c);
                state.sync_worktree_path();
            }
            WorktreeField::Base => state.worktree_base_input.push(c),
            WorktreeField::Path => {
                state.worktree_path_edited = true;
                state.worktree_path_input.push(c);
            }
        },
        _ => {}
    }
    Ok(())
}

fn submit(state: &mut AppState) {
    let branch = state.worktree_branch_input.trim().to_string();
    if branch.is_empty() {
        state.set_status("a worktree needs a branch name", true);
        state.worktree_field = WorktreeField::Branch;
        return;
    }
    let path = state.worktree_path_input.trim().to_string();
    if path.is_empty() {
        state.set_status("a worktree needs a path", true);
        state.worktree_field = WorktreeField::Path;
        return;
    }
    let base = state.worktree_base_input.trim().to_string();
    if base.is_empty() && !branch_exists(state) {
        state.set_status("a new branch needs a base to start from", true);
        state.worktree_field = WorktreeField::Base;
        return;
    }

    state.modal = Modal::None;
    state.pending_action = Some(PendingAction::CreateWorktree { path, branch, base });
}
